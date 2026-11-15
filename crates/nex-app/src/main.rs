// Release builds on Windows are GUI apps (no console window). Use a debug build for `--cli`.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod cli;
mod overlay;
mod settings;
mod usage_store;

fn main() {
    if std::env::args().any(|a| a == "--cli") {
        cli::run();
        return;
    }
    if let Err(e) = overlay::run() {
        eprintln!("Nex Speed failed to start: {e}");
        std::process::exit(1);
    }
}
