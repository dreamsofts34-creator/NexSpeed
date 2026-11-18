//! On-demand speed test against Cloudflare's public endpoints
//! (`/__down?bytes=N` for download and latency, `POST /__up` for upload).
//! Runs only when the user clicks the button, on its own thread. Download and upload use several
//! parallel connections (one TCP stream rarely fills a fast link) and ignore the first second
//! (TCP slow start). Every phase stops at a time budget or a data cap, so slow links don't burn
//! minutes and fast links don't burn gigabytes. A failed phase gives a partial result.

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ureq::Agent;

const BASE: &str = "https://speed.cloudflare.com";
/// Parallel connections per phase.
const STREAMS: usize = 4;
/// Each download connection asks for this many bytes.
const DOWN_STREAM_BYTES: u64 = 25_000_000;
const DOWN_MAX_BYTES: u64 = DOWN_STREAM_BYTES * STREAMS as u64;
/// Upload is sent in small chunks so a slow uplink still reports progress.
const UP_CHUNK: usize = 256_000;
/// Upload stops at this many bytes even if the time budget is not used up (fast uplinks).
const UP_MAX_BYTES: u64 = 50_000_000;
/// Most data a full test can use, in megabytes (download + upload), for the hint in the UI.
pub const MAX_DATA_MB: u64 = (DOWN_MAX_BYTES + UP_MAX_BYTES) / 1_000_000;
const PHASE_BUDGET: Duration = Duration::from_secs(8);
/// The first second of a phase (TCP slow start) is left out of the speed.
const WARMUP: f64 = 1.0;
/// Shortest measuring window after the warm-up that is trusted on its own.
const MIN_WINDOW: f64 = 0.5;
const POLL: Duration = Duration::from_millis(100);
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
    /// Current estimate of the phase, in megabits per second.
    Speed(Phase, f64),
}

/// `None` = that phase failed (the rest of the test still counts).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedTestResult {
    pub ping_ms: f64,
    pub down_mbps: Option<f64>,
    pub up_mbps: Option<f64>,
}

/// Megabits per second (decimal, as ISPs advertise).
pub fn mbps(bytes: u64, secs: f64) -> f64 {
    if secs <= 0.0 {
        0.0
    } else {
        bytes as f64 * 8.0 / 1_000_000.0 / secs
    }
}

/// Speed of a phase: bytes and time after the warm-up when that window is long enough,
/// otherwise the plain average of the whole run (very fast links finish within the warm-up).
fn speed_mbps(total: u64, elapsed: f64, warm: Option<(u64, f64)>) -> f64 {
    match warm {
        Some((bytes, at)) if elapsed - at >= MIN_WINDOW => mbps(total.saturating_sub(bytes), elapsed - at),
        _ => mbps(total, elapsed),
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

/// Error text returned when the test was cancelled by the user.
pub const CANCELLED: &str = "cancelled";

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

/// A phase ends when its time budget or its data cap is used up.
fn phase_done(total_bytes: u64, cap_bytes: u64, elapsed: Duration) -> bool {
    total_bytes >= cap_bytes || elapsed >= PHASE_BUDGET
}

fn err(e: ureq::Error) -> String {
    e.to_string()
}

fn agent() -> Agent {
    Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into()
}

/// Runs latency, download and upload in sequence. `report` is called from this (worker) thread.
/// Only a failed latency probe (no connection) or a cancel aborts the whole test.
pub fn run(report: &dyn Fn(Progress), cancel: &AtomicBool) -> Result<SpeedTestResult, String> {
    let agent = agent();

    report(Progress::Started(Phase::Latency));
    let ping_ms = latency(&agent, cancel)?;
    check(cancel)?;

    report(Progress::Started(Phase::Download));
    let down_mbps = optional(download(&agent, report, cancel))?;
    check(cancel)?;

    report(Progress::Started(Phase::Upload));
    let up_mbps = optional(upload(&agent, report, cancel))?;

    if down_mbps.is_none() && up_mbps.is_none() {
        return Err("no data could be transferred".into());
    }
    Ok(SpeedTestResult { ping_ms, down_mbps, up_mbps })
}

/// A failed phase becomes `None`; a cancel still stops everything.
fn optional(r: Result<f64, String>) -> Result<Option<f64>, String> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e == CANCELLED => Err(e),
        Err(_) => Ok(None),
    }
}

/// First request warms up TLS; the best of the next three is reported.
fn latency(agent: &Agent, cancel: &AtomicBool) -> Result<f64, String> {
    let url = format!("{BASE}/__down?bytes=0");
    let mut best = f64::MAX;
    for i in 0..4 {
        check(cancel)?;
        let start = Instant::now();
        let mut resp = agent.get(&url).call().map_err(err)?;
        let _ = resp.body_mut().read_to_string();
        if i > 0 {
            best = best.min(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    Ok(best)
}

/// Runs `STREAMS` workers that add transferred bytes to a shared counter, and measures the
/// speed from the counter until the budget / cap is reached, every worker has finished, or the
/// user cancels. Some failing workers are fine as long as data flowed.
fn measure<W>(
    phase: Phase,
    cap_bytes: u64,
    report: &dyn Fn(Progress),
    cancel: &AtomicBool,
    worker: W,
) -> Result<f64, String>
where
    W: Fn(&AtomicU64, &AtomicBool) -> Result<(), String> + Sync,
{
    let total = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let start = Instant::now();

    let outcome = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..STREAMS).map(|_| scope.spawn(|| worker(&total, &stop))).collect();
        let mut warm: Option<(u64, f64)> = None;
        let mut last_report = start;
        let mut cancelled = false;
        loop {
            std::thread::sleep(POLL);
            let elapsed = start.elapsed();
            let bytes = total.load(Ordering::Relaxed);
            if warm.is_none() && elapsed.as_secs_f64() >= WARMUP {
                warm = Some((bytes, elapsed.as_secs_f64()));
            }
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            if last_report.elapsed() >= REPORT_EVERY {
                report(Progress::Speed(phase, speed_mbps(bytes, elapsed.as_secs_f64(), warm)));
                last_report = Instant::now();
            }
            if phase_done(bytes, cap_bytes, elapsed) || handles.iter().all(|h| h.is_finished()) {
                break;
            }
        }
        stop.store(true, Ordering::Relaxed);
        let errors: Vec<String> = handles.into_iter().filter_map(|h| h.join().ok().and_then(|r| r.err())).collect();
        (cancelled, warm, errors)
    });

    let (cancelled, warm, errors) = outcome;
    if cancelled {
        return Err(CANCELLED.into());
    }
    let bytes = total.load(Ordering::Relaxed);
    if bytes == 0 {
        return Err(errors.into_iter().next().unwrap_or_else(|| "no data received".into()));
    }
    Ok(speed_mbps(bytes, start.elapsed().as_secs_f64(), warm))
}

fn download(agent: &Agent, report: &dyn Fn(Progress), cancel: &AtomicBool) -> Result<f64, String> {
    let url = format!("{BASE}/__down?bytes={DOWN_STREAM_BYTES}");
    measure(Phase::Download, DOWN_MAX_BYTES, report, cancel, |total, stop| {
        let mut resp = agent.get(&url).call().map_err(err)?;
        let mut reader = resp.body_mut().with_config().limit(DOWN_STREAM_BYTES + 1_000_000).reader();
        let mut buf = vec![0u8; 64 * 1024];
        while !stop.load(Ordering::Relaxed) {
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            total.fetch_add(n as u64, Ordering::Relaxed);
        }
        Ok(())
    })
}

/// Small POSTs from several connections until the budget or data cap is used.
fn upload(agent: &Agent, report: &dyn Fn(Progress), cancel: &AtomicBool) -> Result<f64, String> {
    let url = format!("{BASE}/__up");
    let payload = filler(UP_CHUNK);
    measure(Phase::Upload, UP_MAX_BYTES, report, cancel, |total, stop| {
        while !stop.load(Ordering::Relaxed) {
            agent.post(&url).send(&payload[..]).map_err(err)?;
            total.fetch_add(UP_CHUNK as u64, Ordering::Relaxed);
        }
        Ok(())
    })
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
        assert_eq!(check(&c), Err(CANCELLED.to_string()));
    }

    #[test]
    fn phase_ends_on_time_or_data_cap() {
        assert!(!phase_done(10, 100, Duration::from_secs(1)));
        assert!(phase_done(100, 100, Duration::from_secs(1)));
        assert!(phase_done(10, 100, PHASE_BUDGET));
    }

    #[test]
    fn data_hint_covers_both_directions() {
        assert_eq!(MAX_DATA_MB, 150);
    }

    #[test]
    fn warm_up_is_left_out_when_the_window_is_long_enough() {
        // 1 MB in the first second, then 12.5 MB in the next 4 s = 25 Mbps.
        let v = speed_mbps(13_500_000, 5.0, Some((1_000_000, 1.0)));
        assert!((v - 25.0).abs() < 1e-9);
    }

    #[test]
    fn short_runs_fall_back_to_the_plain_average() {
        assert!((speed_mbps(12_500_000, 1.0, None) - 100.0).abs() < 1e-9);
        // Warm-up ended 0.2 s before the run did: window too short.
        assert!((speed_mbps(12_500_000, 1.2, Some((1_000_000, 1.0))) - mbps(12_500_000, 1.2)).abs() < 1e-9);
    }

    #[test]
    fn failed_phase_is_partial_but_cancel_is_not() {
        assert_eq!(optional(Ok(5.0)), Ok(Some(5.0)));
        assert_eq!(optional(Err("boom".into())), Ok(None));
        assert_eq!(optional(Err(CANCELLED.into())), Err(CANCELLED.to_string()));
    }
}
