//! OS-specific code: network counters, autostart, tray icon. Everything OS-specific stays here.

pub mod autostart;
pub mod connections;
mod fullscreen;
pub mod hotkey;
pub mod icon;
mod tray;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod tray_linux;
#[cfg(windows)]
mod tray_win;
#[cfg(windows)]
mod win;

#[cfg(target_os = "linux")]
pub use linux::LinuxSource as PlatformSource;
#[cfg(windows)]
pub use win::WindowsSource as PlatformSource;

pub use fullscreen::fullscreen_app_active;
pub use tray::{CommandHandler, TrayCommand, TrayView};
#[cfg(target_os = "linux")]
pub use tray_linux::Tray;
#[cfg(windows)]
pub use tray_win::Tray;

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("Nex Speed supports Windows and Linux only.");
