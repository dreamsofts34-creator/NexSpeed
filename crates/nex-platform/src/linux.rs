//! Linux backend: `/proc/net/dev` for counters, `/sys/class/net` for adapter info.
//!
//! Research notes:
//! * `/proc/net/dev` has two header lines, then `name: <8 rx counters> <8 tx counters>`;
//!   rx bytes is the 1st counter, tx bytes the 9th. The split is done on the LAST colon
//!   because an interface name can itself contain one (IP alias like `eth0:0`).
//! * A physical NIC has a `/sys/class/net/<if>/device` entry; virtual ones (veth, docker0,
//!   bridges, tunnels) do not.
//! * The file is reopened on every read instead of seek(0)+read, which keeps the result
//!   obviously fresh. It costs one syscall per second.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use nex_core::{AdapterReading, NetSource};

const PROC_NET_DEV: &str = "/proc/net/dev";
const SYS_NET: &str = "/sys/class/net";

#[derive(Default)]
pub struct LinuxSource {
    physical_cache: HashMap<String, bool>,
    buf: String,
}

impl LinuxSource {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Yields `(interface, rx_bytes, tx_bytes)` for each data line.
fn parse_proc_net_dev(text: &str) -> impl Iterator<Item = (&str, u64, u64)> {
    text.lines().skip(2).filter_map(|line| {
        let (name, counters) = line.rsplit_once(':')?;
        let mut it = counters.split_ascii_whitespace();
        let rx = it.next()?.parse().ok()?;
        // After the first counter, skipping 7 more lands on the 9th: transmit bytes.
        let tx = it.nth(7)?.parse().ok()?;
        Some((name.trim(), rx, tx))
    })
}

fn is_up(name: &str) -> bool {
    // "unknown" is what tun/wireguard-style devices report while carrying traffic.
    match fs::read_to_string(Path::new(SYS_NET).join(name).join("operstate")) {
        Ok(s) => matches!(s.trim(), "up" | "unknown"),
        Err(_) => false,
    }
}

impl NetSource for LinuxSource {
    fn read(&mut self, out: &mut Vec<AdapterReading>) -> io::Result<()> {
        out.clear();
        self.buf.clear();
        File::open(PROC_NET_DEV)?.read_to_string(&mut self.buf)?;

        for (name, rx, tx) in parse_proc_net_dev(&self.buf) {
            if name == "lo" {
                continue;
            }
            let physical = *self
                .physical_cache
                .entry(name.to_owned())
                .or_insert_with(|| Path::new(SYS_NET).join(name).join("device").exists());
            out.push(AdapterReading {
                name: name.to_owned(),
                physical,
                up: is_up(name),
                rx_bytes: rx,
                tx_bytes: tx,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1000 10 0 0 0 0 0 0 1000 10 0 0 0 0 0 0
  eth0: 123456789 1000 0 0 0 0 0 0 987654321 900 0 0 0 0 0 0
eth0:0:5 1 0 0 0 0 0 0 7 1 0 0 0 0 0 0
";

    #[test]
    fn parses_rx_and_tx_bytes() {
        let v: Vec<_> = parse_proc_net_dev(SAMPLE).collect();
        assert_eq!(v[1], ("eth0", 123_456_789, 987_654_321));
    }

    #[test]
    fn splits_on_last_colon_for_alias_names() {
        let v: Vec<_> = parse_proc_net_dev(SAMPLE).collect();
        assert_eq!(v[2], ("eth0:0", 5, 7));
    }
}
