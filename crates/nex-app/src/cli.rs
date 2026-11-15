//! Debug mode: `nexspeed --cli [--all] [--bits]` prints live speed to the terminal.

use std::thread;
use std::time::{Duration, Instant};

use nex_core::{format_speed, NetSource, Sampler, UnitSystem};
use nex_platform::PlatformSource;

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let show_all = args.iter().any(|a| a == "--all");
    let units = if args.iter().any(|a| a == "--bits") { UnitSystem::Bits } else { UnitSystem::Bytes };

    let mut source = PlatformSource::new();
    let mut sampler = Sampler::new();
    let mut readings = Vec::new();

    println!("Nex Speed CLI - press Ctrl+C to quit\n");
    loop {
        match source.read(&mut readings) {
            Ok(()) => {
                if let Some(snap) = sampler.update(Instant::now(), &readings) {
                    println!(
                        "DL {:>12}   UL {:>12}",
                        format_speed(snap.total_rx_bps, units),
                        format_speed(snap.total_tx_bps, units)
                    );
                    if show_all {
                        for a in snap.adapters.iter().filter(|a| a.up) {
                            println!(
                                "    {:<30} {:<9} DL {:>12}   UL {:>12}",
                                a.name,
                                if a.physical { "physical" } else { "virtual" },
                                format_speed(a.rx_bps, units),
                                format_speed(a.tx_bps, units)
                            );
                        }
                    }
                }
            }
            Err(e) => eprintln!("could not read network counters: {e}"),
        }
        thread::sleep(Duration::from_secs(1));
    }
}
