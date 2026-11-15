/// Number of samples kept for the graph (one per second => last 60 s).
pub const HISTORY_LEN: usize = 60;

/// Fixed-size history, oldest sample first, newest last. No allocation after creation.
#[derive(Clone, Debug)]
pub struct SpeedHistory {
    pub rx: [f32; HISTORY_LEN],
    pub tx: [f32; HISTORY_LEN],
}

impl Default for SpeedHistory {
    fn default() -> Self {
        Self { rx: [0.0; HISTORY_LEN], tx: [0.0; HISTORY_LEN] }
    }
}

impl SpeedHistory {
    pub fn push(&mut self, rx_bps: f64, tx_bps: f64) {
        self.rx.copy_within(1.., 0);
        self.tx.copy_within(1.., 0);
        self.rx[HISTORY_LEN - 1] = rx_bps as f32;
        self.tx[HISTORY_LEN - 1] = tx_bps as f32;
    }
}

/// Exponential moving average, used to make the displayed numbers calmer.
#[derive(Clone, Debug)]
pub struct Ema {
    alpha: f64,
    value: Option<f64>,
}

impl Ema {
    pub fn new(alpha: f64) -> Self {
        Self { alpha: alpha.clamp(0.0, 1.0), value: None }
    }

    pub fn update(&mut self, x: f64) -> f64 {
        let v = match self.value {
            Some(prev) => prev + self.alpha * (x - prev),
            None => x,
        };
        self.value = Some(v);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_shifts_left_and_appends() {
        let mut h = SpeedHistory::default();
        h.push(1.0, 10.0);
        h.push(2.0, 20.0);
        assert_eq!(h.rx[HISTORY_LEN - 1], 2.0);
        assert_eq!(h.rx[HISTORY_LEN - 2], 1.0);
        assert_eq!(h.tx[HISTORY_LEN - 2], 10.0);
        assert_eq!(h.rx[0], 0.0);
    }

    #[test]
    fn ema_starts_at_first_value_then_smooths() {
        let mut e = Ema::new(0.5);
        assert_eq!(e.update(100.0), 100.0);
        assert_eq!(e.update(0.0), 50.0);
        assert_eq!(e.update(0.0), 25.0);
    }
}
