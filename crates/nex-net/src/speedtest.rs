//! On-demand speed test against Cloudflare's public endpoints
//! (`/__down?bytes=N` for download and latency, `POST /__up` for upload).
//! Runs only when the user clicks the button, on its own thread, and stops after a time budget
//! per phase, so slow connections don't burn minutes of data.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ureq::Agent;

const BASE: &str = "https://speed.cloudflare.com";
const DOWN_BYTES: u64 = 50_000_000;
const UP_CHUNK: usize = 2_000_000;
const PHASE_BUDGET: Duration = Duration::from_secs(8);
const REPORT_EVERY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Latency,
    Download,
    Upload,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Started(Phase),
    /// Running average of the phase so far, in megabits per second.
    Speed(Phase, f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedTestResult {
    pub ping_ms: f64,
    pub down_mbps: f64,
    pub up_mbps: f64,
}

/// Megabits per second (decimal, as ISPs advertise).
pub fn mbps(bytes: u64, secs: f64) -> f64 {
    if secs <= 0.0 {
        0.0
    } else {
        bytes as f64 * 8.0 / 1_000_000.0 / secs
    }
}

/// Incompressible-looking filler (xorshift), so compression on the path can't inflate the result.
fn filler(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("cancelled".into())
    } else {
        Ok(())
    }
}

fn err(e: ureq::Error) -> String {
    e.to_string()
}

fn agent() -> Agent {
    Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into()
}

/// Runs latency, download and upload in sequence. `report` is called from this (worker) thread.
pub fn run(report: &dyn Fn(Progress), cancel: &AtomicBool) -> Result<SpeedTestResult, String> {
    let agent = agent();

    report(Progress::Started(Phase::Latency));
    let ping_ms = latency(&agent)?;
    check(cancel)?;

    report(Progress::Started(Phase::Download));
    let down_mbps = download(&agent, report, cancel)?;

    report(Progress::Started(Phase::Upload));
    let up_mbps = upload(&agent, report, cancel)?;

    Ok(SpeedTestResult { ping_ms, down_mbps, up_mbps })
}

/// First request warms up TLS; the best of the next three is reported.
fn latency(agent: &Agent) -> Result<f64, String> {
    let url = format!("{BASE}/__down?bytes=0");
    let mut best = f64::MAX;
    for i in 0..4 {
        let start = Instant::now();
        let mut resp = agent.get(&url).call().map_err(err)?;
        let _ = resp.body_mut().read_to_string();
        if i > 0 {
            best = best.min(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    Ok(best)
}

fn download(agent: &Agent, report: &dyn Fn(Progress), cancel: &AtomicBool) -> Result<f64, String> {
    let url = format!("{BASE}/__down?bytes={DOWN_BYTES}");
    let mut resp = agent.get(&url).call().map_err(err)?;
    let mut reader = resp.body_mut().with_config().limit(DOWN_BYTES + 1_000_000).reader();

    let mut buf = vec![0u8; 64 * 1024];
    let start = Instant::now();
    let mut last_report = start;
    let mut total: u64 = 0;
    loop {
        check(cancel)?;
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        let now = Instant::now();
        if now - last_report >= REPORT_EVERY {
            report(Progress::Speed(Phase::Download, mbps(total, (now - start).as_secs_f64())));
            last_report = now;
        }
        if now - start >= PHASE_BUDGET {
            break;
        }
    }
    if total == 0 {
        return Err("no data received".into());
    }
    Ok(mbps(total, start.elapsed().as_secs_f64()))
}

/// Several small POSTs until the time budget is used, so a slow uplink never blocks for long.
fn upload(agent: &Agent, report: &dyn Fn(Progress), cancel: &AtomicBool) -> Result<f64, String> {
    let url = format!("{BASE}/__up");
    let payload = filler(UP_CHUNK);
    let start = Instant::now();
    let mut total: u64 = 0;
    loop {
        check(cancel)?;
        agent.post(&url).send(&payload[..]).map_err(err)?;
        total += UP_CHUNK as u64;
        let secs = start.elapsed().as_secs_f64();
        report(Progress::Speed(Phase::Upload, mbps(total, secs)));
        if start.elapsed() >= PHASE_BUDGET {
            return Ok(mbps(total, secs));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mbps_math() {
        assert!((mbps(12_500_000, 1.0) - 100.0).abs() < 1e-9);
        assert_eq!(mbps(1000, 0.0), 0.0);
    }

    #[test]
    fn filler_has_requested_length_and_is_not_constant() {
        let f = filler(1001);
        assert_eq!(f.len(), 1001);
        assert!(f.iter().any(|&b| b != f[0]));
    }

    #[test]
    fn cancel_flag_is_respected() {
        let c = AtomicBool::new(true);
        assert_eq!(check(&c), Err("cancelled".to_string()));
    }
}
