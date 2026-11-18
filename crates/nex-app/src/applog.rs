//! Tiny log file (`nexspeed.log` next to the usage data). Release builds on Windows have no
//! console, so failures that used to go to `eprintln!` (tray, hotkey, autostart, panics) would
//! otherwise vanish. Only rare events are logged, so the file is opened per line.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::settings::APP_DIR;

/// Older lines are dropped (file moved to `.old`) once the log grows past this.
const MAX_BYTES: u64 = 256 * 1024;

fn path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join(APP_DIR).join("nexspeed.log"))
}

/// Appends one timestamped line. Never panics, ignores I/O errors; also prints to stderr.
pub fn line(msg: &str) {
    eprintln!("{msg}");
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = fs::rename(&path, path.with_extension("log.old"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}  {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}

/// Logs panics (release builds unwind, so a panic in a worker thread only ends that thread).
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| line(&format!("panic: {info}"))));
}
