//! Daily data-usage log. Pure data + a tiny binary format; file I/O lives in the app.
//!
//! Format (little endian): magic `NXU1`, `u32` count, then per day `u32 date (YYYYMMDD)`,
//! `u64 rx_bytes`, `u64 tx_bytes`. About 20 bytes per day.

const MAGIC: &[u8; 4] = b"NXU1";
const RECORD_LEN: usize = 20;
const HEADER_LEN: usize = 8;
/// Keep roughly two years.
const MAX_DAYS: usize = 800;

/// Share of a monthly data limit already used, in percent. `None` when no limit is set (0).
pub fn limit_percent(used_bytes: u64, limit_gb: u32) -> Option<f64> {
    if limit_gb == 0 {
        None
    } else {
        Some(used_bytes as f64 / (limit_gb as f64 * 1024.0 * 1024.0 * 1024.0) * 100.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DayUsage {
    /// Local date as `YYYYMMDD`.
    pub date: u32,
    pub rx: u64,
    pub tx: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageLog {
    days: Vec<DayUsage>,
}

impl UsageLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds traffic to `date`. Works for any date order (e.g. if the clock moves back).
    pub fn add(&mut self, date: u32, rx: u64, tx: u64) {
        match self.days.binary_search_by_key(&date, |d| d.date) {
            Ok(i) => {
                self.days[i].rx = self.days[i].rx.saturating_add(rx);
                self.days[i].tx = self.days[i].tx.saturating_add(tx);
            }
            Err(i) => {
                self.days.insert(i, DayUsage { date, rx, tx });
                if self.days.len() > MAX_DAYS {
                    let extra = self.days.len() - MAX_DAYS;
                    self.days.drain(..extra);
                }
            }
        }
    }

    /// `(rx, tx)` for one day.
    pub fn day(&self, date: u32) -> (u64, u64) {
        self.days
            .binary_search_by_key(&date, |d| d.date)
            .map(|i| (self.days[i].rx, self.days[i].tx))
            .unwrap_or((0, 0))
    }

    /// `(rx, tx)` for a calendar month given as `YYYYMM`.
    pub fn month_total(&self, year_month: u32) -> (u64, u64) {
        self.days
            .iter()
            .filter(|d| d.date / 100 == year_month)
            .fold((0, 0), |(rx, tx), d| (rx.saturating_add(d.rx), tx.saturating_add(d.tx)))
    }

    pub fn days(&self) -> &[DayUsage] {
        &self.days
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.days.len() * RECORD_LEN);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.days.len() as u32).to_le_bytes());
        for d in &self.days {
            out.extend_from_slice(&d.date.to_le_bytes());
            out.extend_from_slice(&d.rx.to_le_bytes());
            out.extend_from_slice(&d.tx.to_le_bytes());
        }
        out
    }

    /// Returns `None` for anything that is not a well-formed log (wrong magic, truncated...).
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN || &bytes[..4] != MAGIC {
            return None;
        }
        let count = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
        if count > MAX_DAYS * 4 || bytes.len() != HEADER_LEN + count * RECORD_LEN {
            return None;
        }
        let mut days: Vec<DayUsage> = bytes[HEADER_LEN..]
            .as_chunks::<RECORD_LEN>()
            .0
            .iter()
            .map(|c| DayUsage {
                date: u32::from_le_bytes(c[0..4].try_into().unwrap()),
                rx: u64::from_le_bytes(c[4..12].try_into().unwrap()),
                tx: u64::from_le_bytes(c[12..20].try_into().unwrap()),
            })
            .collect();
        days.sort_by_key(|d| d.date);
        // Duplicate dates (a damaged or merged file) are summed, not dropped.
        days.dedup_by(|later, kept| {
            if later.date == kept.date {
                kept.rx = kept.rx.saturating_add(later.rx);
                kept.tx = kept.tx.saturating_add(later.tx);
                true
            } else {
                false
            }
        });
        Some(Self { days })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_percent_math() {
        assert_eq!(limit_percent(5, 0), None);
        let half = 50u64 * 1024 * 1024 * 1024;
        assert!((limit_percent(half, 100).unwrap() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn adds_and_sums_by_day_and_month() {
        let mut log = UsageLog::new();
        log.add(20261114, 100, 10);
        log.add(20261114, 50, 5);
        log.add(20261115, 1, 1);
        log.add(20261201, 7, 7);
        assert_eq!(log.day(20261114), (150, 15));
        assert_eq!(log.day(20260101), (0, 0));
        assert_eq!(log.month_total(202611), (151, 16));
        assert_eq!(log.month_total(202612), (7, 7));
    }

    #[test]
    fn out_of_order_dates_stay_sorted() {
        let mut log = UsageLog::new();
        log.add(20261115, 1, 0);
        log.add(20261114, 2, 0);
        let dates: Vec<u32> = log.days().iter().map(|d| d.date).collect();
        assert_eq!(dates, vec![20261114, 20261115]);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let mut log = UsageLog::new();
        log.add(20261114, u64::MAX - 1, 42);
        log.add(20261115, 5, 6);
        assert_eq!(UsageLog::decode(&log.encode()), Some(log));
    }

    #[test]
    fn rejects_garbage_and_truncated_data() {
        assert_eq!(UsageLog::decode(b""), None);
        assert_eq!(UsageLog::decode(b"nope nope nope"), None);
        let mut log = UsageLog::new();
        log.add(20261114, 1, 1);
        let mut bytes = log.encode();
        bytes.pop();
        assert_eq!(UsageLog::decode(&bytes), None);
    }

    #[test]
    fn old_days_are_pruned() {
        let mut log = UsageLog::new();
        for i in 0..(MAX_DAYS as u32 + 10) {
            log.add(20000000 + i, 1, 1);
        }
        assert_eq!(log.days().len(), MAX_DAYS);
        assert_eq!(log.days()[0].date, 20000010);
    }

    #[test]
    fn decode_sums_duplicate_dates() {
        let mut log = UsageLog::new();
        log.add(20261114, 1, 2);
        let mut bytes = log.encode();
        // Append a second record for the same day and fix the count.
        bytes.extend_from_slice(&20261114u32.to_le_bytes());
        bytes.extend_from_slice(&10u64.to_le_bytes());
        bytes.extend_from_slice(&20u64.to_le_bytes());
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        let back = UsageLog::decode(&bytes).unwrap();
        assert_eq!(back.days().len(), 1);
        assert_eq!(back.day(20261114), (11, 22));
    }
}
