//! Persistent settings in `settings.toml`. Every field has a default, so a missing,
//! partial or corrupt file never breaks startup.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[cfg(windows)]
pub const APP_DIR: &str = "NexSpeed"; // %APPDATA%\NexSpeed
#[cfg(not(windows))]
pub const APP_DIR: &str = "nexspeed"; // ~/.config/nexspeed

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// Show bits per second (Mbps) instead of bytes per second (MB/s).
    pub bits: bool,
    pub overlay_visible: bool,
    /// Hide the overlay while a full-screen app (game, video) is in front.
    pub auto_hide_fullscreen: bool,
    /// Monthly data limit in GB; 0 = no limit.
    pub monthly_limit_gb: u32,
    /// Light theme instead of dark.
    pub light: bool,
    /// Overlay background opacity in percent (see `nex_core::OPACITY_STEPS`).
    pub opacity_pct: u8,
    /// Overlay style: 0 Graph, 1 Minimal, 2 Compact.
    pub style: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_y: Option<i32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bits: false,
            overlay_visible: true,
            auto_hide_fullscreen: true,
            monthly_limit_gb: 0,
            light: false,
            opacity_pct: 100,
            style: 0,
            pos_x: None,
            pos_y: None,
        }
    }
}

/// Choices the tray's "Monthly limit" item cycles through (0 = off).
pub const LIMIT_STEPS_GB: [u32; 6] = [0, 50, 100, 200, 500, 1000];

/// Next choice after `current`; unknown values restart the cycle.
pub fn next_limit(current: u32) -> u32 {
    match LIMIT_STEPS_GB.iter().position(|&g| g == current) {
        Some(i) => LIMIT_STEPS_GB[(i + 1) % LIMIT_STEPS_GB.len()],
        None => LIMIT_STEPS_GB[0],
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(APP_DIR).join("settings.toml"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Best effort: a failure to save must never crash the app. Writes a temp file first
    /// and renames it, so a crash mid-write can't leave a half-written settings file.
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let Ok(text) = toml::to_string_pretty(self) else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("toml.tmp");
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings {
            bits: true,
            overlay_visible: false,
            auto_hide_fullscreen: false,
            monthly_limit_gb: 200,
            light: true,
            opacity_pct: 70,
            style: 2,
            pos_x: Some(-20),
            pos_y: Some(300),
        };
        let back: Settings = toml::from_str(&toml::to_string_pretty(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn partial_file_uses_defaults_for_the_rest() {
        let s: Settings = toml::from_str("bits = true").unwrap();
        assert!(s.bits);
        assert!(s.overlay_visible);
        assert_eq!(s.pos_x, None);
        assert_eq!((s.light, s.opacity_pct, s.style), (false, 100, 0));
    }

    #[test]
    fn limit_cycles_and_wraps() {
        assert_eq!(next_limit(0), 50);
        assert_eq!(next_limit(1000), 0);
        assert_eq!(next_limit(77), 0);
    }

    #[test]
    fn default_has_no_saved_position() {
        let s = Settings::default();
        assert_eq!((s.pos_x, s.pos_y), (None, None));
    }
}
