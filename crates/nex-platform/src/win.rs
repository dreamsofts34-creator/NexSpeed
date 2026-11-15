//! Windows backend: IP Helper `GetIfTable2` (64-bit counters, Vista and later).
//!
//! Research notes (verified against docs and real-world implementations):
//! * `GetIfTable2` returns the table via an out-pointer and it MUST be released with
//!   `FreeMibTable`.
//! * Every NDIS lightweight filter bound to an adapter appears as its own row with
//!   IDENTICAL counters (e.g. "Wi-Fi-QoS Packet Scheduler-0000"). Those rows have the
//!   `FilterInterface` flag and must be dropped, or speed is counted several times.
//! * Do NOT keep only `HardwareInterface` rows: tunnel adapters (e.g. WireGuard) carry
//!   real traffic but are not hardware. We keep them as readings, just marked
//!   `physical = false`, so the engine can avoid double counting.
//! * `InterfaceAndOperStatusFlags` is a bitfield byte: bit0 Hardware, bit1 Filter,
//!   bit2 ConnectorPresent.

use std::io;

use nex_core::{AdapterReading, NetSource};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const FLAG_HARDWARE: u8 = 1 << 0;
const FLAG_FILTER: u8 = 1 << 1;
const FLAG_CONNECTOR_PRESENT: u8 = 1 << 2;

#[derive(Default)]
pub struct WindowsSource;

impl WindowsSource {
    pub fn new() -> Self {
        Self
    }
}

/// `None` = row must be ignored. `Some(physical)` otherwise.
fn classify(if_type: u32, flags: u8) -> Option<bool> {
    if if_type == IF_TYPE_SOFTWARE_LOOPBACK || flags & FLAG_FILTER != 0 {
        return None;
    }
    Some(flags & (FLAG_HARDWARE | FLAG_CONNECTOR_PRESENT) != 0)
}

fn alias_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_owned()
}

impl NetSource for WindowsSource {
    fn read(&mut self, out: &mut Vec<AdapterReading>) -> io::Result<()> {
        out.clear();
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();

        // SAFETY: GetIfTable2 allocates the table and stores its address in `table`.
        let rc = unsafe { GetIfTable2(&mut table) };
        if rc.is_err() || table.is_null() {
            return Err(io::Error::from_raw_os_error(rc.0 as i32));
        }

        // SAFETY: on success the table holds `NumEntries` contiguous rows. It is read
        // only before the single FreeMibTable call below.
        unsafe {
            let count = (*table).NumEntries as usize;
            let rows = (*table).Table.as_ptr();
            for i in 0..count {
                let row = &*rows.add(i);
                let Some(physical) = classify(row.Type, row.InterfaceAndOperStatusFlags._bitfield)
                else {
                    continue;
                };
                let name = alias_to_string(&row.Alias);
                if name.is_empty() {
                    continue;
                }
                out.push(AdapterReading {
                    name,
                    physical,
                    up: row.OperStatus == IfOperStatusUp,
                    rx_bytes: row.InOctets,
                    tx_bytes: row.OutOctets,
                });
            }
            FreeMibTable(table as *const core::ffi::c_void);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Rows taken from a real Windows 11 machine: (alias, Type, flags byte).
    #[test]
    fn filter_rows_and_loopback_are_dropped() {
        assert_eq!(classify(71, 0b0000_0010), None); // Wi-Fi-QoS Packet Scheduler-0000
        assert_eq!(classify(24, 0b0000_0000), None); // Loopback Pseudo-Interface 1
    }

    #[test]
    fn physical_and_tunnel_adapters_are_kept_with_correct_flag() {
        assert_eq!(classify(71, 0b0000_0101), Some(true)); // Wi-Fi
        assert_eq!(classify(6, 0b0000_0101), Some(true)); // Ethernet
        assert_eq!(classify(53, 0b0000_0000), Some(false)); // wt0 (WireGuard)
    }
}
