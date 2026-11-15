use std::io;

/// Cumulative counters of one network adapter at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterReading {
    /// Display name, also used as the key between samples.
    pub name: String,
    /// True for real NICs (Ethernet, Wi-Fi). False for VPN/tunnel/virtual adapters,
    /// whose traffic is already counted on the physical adapter carrying it.
    pub physical: bool,
    /// Operationally up.
    pub up: bool,
    /// Total bytes received since the adapter appeared.
    pub rx_bytes: u64,
    /// Total bytes transmitted since the adapter appeared.
    pub tx_bytes: u64,
}

/// A source of adapter counters. Implemented per OS in `nex-platform`.
pub trait NetSource {
    /// Clears `out`, then fills it with one reading per reportable adapter.
    /// The caller reuses `out` between calls to avoid reallocating the Vec.
    fn read(&mut self, out: &mut Vec<AdapterReading>) -> io::Result<()>;
}
