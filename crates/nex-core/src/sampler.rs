use std::collections::HashMap;
use std::time::Instant;

use crate::source::AdapterReading;

/// Readings closer together than this are ignored (avoids noisy division).
const MIN_INTERVAL_SECS: f64 = 0.05;

/// Speed of one adapter since the previous sample.
#[derive(Clone, Debug)]
pub struct AdapterSpeed {
    pub name: String,
    pub physical: bool,
    pub up: bool,
    /// Bytes per second.
    pub rx_bps: f64,
    pub tx_bps: f64,
    /// Bytes transferred since the previous sample.
    pub rx_delta: u64,
    pub tx_delta: u64,
}

/// Result of one sampling step.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub adapters: Vec<AdapterSpeed>,
    /// Total download, bytes/s. Physical adapters only (no VPN double counting).
    pub total_rx_bps: f64,
    /// Total upload, bytes/s.
    pub total_tx_bps: f64,
    /// Bytes downloaded since the previous snapshot (same adapter selection).
    pub total_rx_delta: u64,
    pub total_tx_delta: u64,
}

impl Snapshot {
    /// The adapter to name in the UI: the busiest physical adapter that is up
    /// (first one when idle), else the busiest up adapter, else `None`.
    pub fn primary_adapter(&self) -> Option<&AdapterSpeed> {
        let busiest = |physical_only: bool| {
            self.adapters
                .iter()
                .filter(|a| a.up && (a.physical || !physical_only))
                .max_by(|a, b| (a.rx_bps + a.tx_bps).total_cmp(&(b.rx_bps + b.tx_bps)))
        };
        busiest(true).or_else(|| busiest(false))
    }
}

#[derive(Default)]
pub struct Sampler {
    prev: HashMap<String, (u64, u64)>,
    last: Option<Instant>,
}

impl Sampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the latest readings. Returns `None` for the very first call (no baseline yet)
    /// or when called again too quickly. No bytes are lost in the second case: the baseline
    /// is only moved forward when a snapshot is produced.
    pub fn update(&mut self, now: Instant, readings: &[AdapterReading]) -> Option<Snapshot> {
        let elapsed = self
            .last
            .map(|t| now.saturating_duration_since(t).as_secs_f64())
            .unwrap_or(0.0);

        let mut snapshot = None;
        if elapsed >= MIN_INTERVAL_SECS {
            let mut adapters = Vec::with_capacity(readings.len());
            for r in readings {
                // New adapter, or counters that went backwards (reset / driver reload): 0.
                let (rx_delta, tx_delta) = match self.prev.get(&r.name) {
                    Some(&(prx, ptx)) => {
                        (r.rx_bytes.saturating_sub(prx), r.tx_bytes.saturating_sub(ptx))
                    }
                    None => (0, 0),
                };
                adapters.push(AdapterSpeed {
                    name: r.name.clone(),
                    physical: r.physical,
                    up: r.up,
                    rx_bps: rx_delta as f64 / elapsed,
                    tx_bps: tx_delta as f64 / elapsed,
                    rx_delta,
                    tx_delta,
                });
            }
            let t = totals(&adapters);
            snapshot = Some(Snapshot {
                adapters,
                total_rx_bps: t.rx_bps,
                total_tx_bps: t.tx_bps,
                total_rx_delta: t.rx_delta,
                total_tx_delta: t.tx_delta,
            });
        }

        if self.last.is_none() || elapsed >= MIN_INTERVAL_SECS {
            // Rebuild from scratch so adapters that vanished don't leave stale baselines
            // (a returning adapter with higher counters would show a false burst).
            self.prev.clear();
            for r in readings {
                self.prev.insert(r.name.clone(), (r.rx_bytes, r.tx_bytes));
            }
            self.last = Some(now);
        }
        snapshot
    }
}

#[derive(Default)]
struct Totals {
    rx_bps: f64,
    tx_bps: f64,
    rx_delta: u64,
    tx_delta: u64,
}

/// Sum of physical, up adapters. If none qualifies, fall back to all up adapters.
fn totals(adapters: &[AdapterSpeed]) -> Totals {
    let any_physical = adapters.iter().any(|a| a.up && a.physical);
    adapters
        .iter()
        .filter(|a| a.up && (a.physical || !any_physical))
        .fold(Totals::default(), |mut t, a| {
            t.rx_bps += a.rx_bps;
            t.tx_bps += a.tx_bps;
            t.rx_delta += a.rx_delta;
            t.tx_delta += a.tx_delta;
            t
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn r(name: &str, physical: bool, up: bool, rx: u64, tx: u64) -> AdapterReading {
        AdapterReading { name: name.into(), physical, up, rx_bytes: rx, tx_bytes: tx }
    }

    #[test]
    fn primary_adapter_prefers_busy_physical_one() {
        let mut s = Sampler::new();
        let t0 = Instant::now();
        s.update(
            t0,
            &[r("Ethernet", true, true, 0, 0), r("Wi-Fi", true, true, 0, 0), r("wt0", false, true, 0, 0)],
        );
        let snap = s
            .update(
                t0 + Duration::from_secs(1),
                &[
                    r("Ethernet", true, true, 10, 0),
                    r("Wi-Fi", true, true, 500, 0),
                    r("wt0", false, true, 900, 0),
                ],
            )
            .unwrap();
        assert_eq!(snap.primary_adapter().unwrap().name, "Wi-Fi");
    }

    #[test]
    fn primary_adapter_is_none_when_everything_is_down() {
        let mut s = Sampler::new();
        let t0 = Instant::now();
        s.update(t0, &[r("eth0", true, false, 0, 0)]);
        let snap = s.update(t0 + Duration::from_secs(1), &[r("eth0", true, false, 0, 0)]).unwrap();
        assert!(snap.primary_adapter().is_none());
    }

    #[test]
    fn first_update_has_no_snapshot() {
        let mut s = Sampler::new();
        assert!(s.update(Instant::now(), &[r("eth0", true, true, 1, 1)]).is_none());
    }

    #[test]
    fn computes_bytes_per_second_and_deltas() {
        let mut s = Sampler::new();
        let t0 = Instant::now();
        s.update(t0, &[r("eth0", true, true, 1000, 500)]);
        let snap = s
            .update(t0 + Duration::from_secs(1), &[r("eth0", true, true, 3000, 1500)])
            .unwrap();
        assert!((snap.total_rx_bps - 2000.0).abs() < 1e-6);
        assert!((snap.total_tx_bps - 1000.0).abs() < 1e-6);
        assert_eq!((snap.total_rx_delta, snap.total_tx_delta), (2000, 1000));
    }

    #[test]
    fn too_quick_second_call_loses_no_bytes() {
        let mut s = Sampler::new();
        let t0 = Instant::now();
        s.update(t0, &[r("eth0", true, true, 0, 0)]);
        assert!(s.update(t0 + Duration::from_millis(10), &[r("eth0", true, true, 100, 0)]).is_none());
        let snap = s
            .update(t0 + Duration::from_secs(1), &[r("eth0", true, true, 300, 0)])
            .unwrap();
        assert_eq!(snap.total_rx_delta, 300);
    }

    #[test]
    fn counter_reset_gives_zero_not_a_burst() {
        let mut s = Sampler::new();
        let t0 = Instant::now();
        s.update(t0, &[r("eth0", true, true, 9_000_000, 9_000_000)]);
        let snap = s
            .update(t0 + Duration::from_secs(1), &[r("eth0", true, true, 10, 10)])
            .unwrap();
        assert_eq!(snap.total_rx_bps, 0.0);
        assert_eq!(snap.total_rx_delta, 0);
    }

    #[test]
    fn vpn_is_not_double_counted() {
        let mut s = Sampler::new();
        let t0 = Instant::now();
        s.update(t0, &[r("Wi-Fi", true, true, 0, 0), r("wt0", false, true, 0, 0)]);
        let snap = s
            .update(
                t0 + Duration::from_secs(1),
                &[r("Wi-Fi", true, true, 1000, 0), r("wt0", false, true, 900, 0)],
            )
            .unwrap();
        assert!((snap.total_rx_bps - 1000.0).abs() < 1e-6);
        assert_eq!(snap.total_rx_delta, 1000);
    }

    #[test]
    fn falls_back_to_all_up_adapters_when_none_is_physical() {
        let mut s = Sampler::new();
        let t0 = Instant::now();
        s.update(t0, &[r("tun0", false, true, 0, 0)]);
        let snap = s
            .update(t0 + Duration::from_secs(1), &[r("tun0", false, true, 500, 0)])
            .unwrap();
        assert!((snap.total_rx_bps - 500.0).abs() < 1e-6);
    }
}
