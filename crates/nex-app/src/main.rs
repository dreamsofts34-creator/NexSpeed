// Release builds on Windows are GUI apps (no console window). Use a debug build for `--cli`.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod applog;
mod cli;
mod instance;
mod overlay;
mod settings;
mod usage_store;

const HELP: &str = "Nex Speed - tiny internet speed meter

USAGE:
    nexspeed [OPTIONS]

OPTIONS:
    --cli        Print live speed in the terminal instead of opening the overlay
      --all      (with --cli) also list every adapter
      --bits     (with --cli) show bits per second
    -h, --help   Show this help
    -V, --version  Show the version
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return;
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("nexspeed {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--cli") {
        cli::run();
        return;
    }
    applog::install_panic_hook();
    let listener = match instance::acquire() {
        instance::Guard::Primary(l) => Some(l),
        instance::Guard::Secondary => return,
        instance::Guard::Unguarded => None,
    };
    if let Err(e) = overlay::run(listener) {
        applog::line(&format!("Nex Speed failed to start: {e}"));
        std::process::exit(1);
    }
}
