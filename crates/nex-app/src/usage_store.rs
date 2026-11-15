//! Loads/saves the usage log and turns it into the text shown in the tray.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Datelike;
use nex_core::{format_bytes, last_days, usage_csv, UsageLog};

use crate::settings::APP_DIR;

/// Write to disk about once a minute (and on quit), not on every sample.
const FLUSH_EVERY_RECORDS: u32 = 60;

pub struct UsageStore {
    log: UsageLog,
    dirty: bool,
    since_flush: u32,
}

/// `<dir>/nexspeed-usage-YYYYMMDD.csv`
fn export_path(dir: &Path, today: u32) -> PathBuf {
    dir.join(format!("nexspeed-usage-{today}.csv"))
}

/// Today's local date as `YYYYMMDD`.
pub fn today_key() -> u32 {
    let d = chrono::Local::now();
    d.year() as u32 * 10_000 + d.month() * 100 + d.day()
}

fn path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_DIR).join("usage.bin"))
}

impl UsageStore {
    pub fn load() -> Self {
        let log = path()
            .and_then(|p| fs::read(p).ok())
            .and_then(|b| UsageLog::decode(&b))
            .unwrap_or_default();
        Self { log, dirty: false, since_flush: 0 }
    }

    pub fn record(&mut self, date: u32, rx: u64, tx: u64) {
        if rx > 0 || tx > 0 {
            self.log.add(date, rx, tx);
            self.dirty = true;
        }
        self.since_flush += 1;
        if self.since_flush >= FLUSH_EVERY_RECORDS {
            self.flush();
        }
    }

    /// Best effort (temp file + rename), never panics.
    pub fn flush(&mut self) {
        self.since_flush = 0;
        if !self.dirty {
            return;
        }
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("bin.tmp");
        if fs::write(&tmp, self.log.encode()).is_ok() && fs::rename(&tmp, &path).is_ok() {
            self.dirty = false;
        }
    }

    /// Download + upload for one day, in bytes.
    pub fn today_bytes(&self, today: u32) -> u64 {
        let (rx, tx) = self.log.day(today);
        rx.saturating_add(tx)
    }

    /// Download + upload for the month containing `today`, in bytes.
    pub fn month_bytes(&self, today: u32) -> u64 {
        let (rx, tx) = self.log.month_total(today / 100);
        rx.saturating_add(tx)
    }

    /// `(rx, tx)` for each of the last `n` days ending today, oldest first.
    pub fn series(&self, today: u32, n: usize) -> Vec<(u64, u64)> {
        last_days(today, n).into_iter().map(|d| self.log.day(d)).collect()
    }

    /// Writes the whole log as CSV into the Downloads folder (home folder if there is none)
    /// and returns the file path.
    pub fn export_csv(&self, today: u32) -> io::Result<PathBuf> {
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no Downloads or home folder"))?;
        let path = export_path(&dir, today);
        fs::write(&path, usage_csv(&self.log))?;
        Ok(path)
    }

    /// `Today 3.20 GB  ·  Month 48.7 GB` (download + upload).
    pub fn summary(&self, today: u32) -> String {
        let (tr, tt) = self.log.day(today);
        let (mr, mt) = self.log.month_total(today / 100);
        format!("Today {}  ·  Month {}", format_bytes(tr + tt), format_bytes(mr + mt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_adds_download_and_upload() {
        let mut s = UsageStore { log: UsageLog::new(), dirty: false, since_flush: 0 };
        s.log.add(20261114, 1024 * 1024, 1024 * 1024);
        assert_eq!(s.summary(20261114), "Today 2.00 MB  ·  Month 2.00 MB");
    }

    #[test]
    fn series_covers_the_last_days_with_zeros() {
        let mut s = UsageStore { log: UsageLog::new(), dirty: false, since_flush: 0 };
        s.log.add(20261114, 5, 6);
        assert_eq!(s.series(20261115, 3), vec![(0, 0), (5, 6), (0, 0)]);
    }

    #[test]
    fn export_file_name_has_the_date() {
        let p = export_path(Path::new("/x"), 20261115);
        assert_eq!(p, Path::new("/x").join("nexspeed-usage-20261115.csv"));
    }

    #[test]
    fn today_key_looks_like_a_date() {
        let k = today_key();
        assert!((20_000_101..=99_991_231).contains(&k));
    }
}
