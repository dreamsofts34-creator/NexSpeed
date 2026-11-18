//! Active TCP connections: shared types and the pure parsing helpers. Reading them from the OS
//! lives in `nex-platform`; everything here is testable on any machine.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpState {
    Established,
    SynSent,
    SynRecv,
    FinWait1,
    FinWait2,
    TimeWait,
    Close,
    CloseWait,
    LastAck,
    Listen,
    Closing,
    Unknown,
}

impl TcpState {
    /// Linux `/proc/net/tcp` state column (hex byte, values from the kernel's `tcp_states.h`).
    pub fn from_linux(code: u8) -> Self {
        match code {
            1 => Self::Established,
            2 => Self::SynSent,
            3 => Self::SynRecv,
            4 => Self::FinWait1,
            5 => Self::FinWait2,
            6 => Self::TimeWait,
            7 => Self::Close,
            8 => Self::CloseWait,
            9 => Self::LastAck,
            10 => Self::Listen,
            11 => Self::Closing,
            _ => Self::Unknown,
        }
    }

    /// Windows `MIB_TCP_STATE` (1 = closed ... 12 = delete TCB).
    pub fn from_windows(code: u32) -> Self {
        match code {
            1 => Self::Close,
            2 => Self::Listen,
            3 => Self::SynSent,
            4 => Self::SynRecv,
            5 => Self::Established,
            6 => Self::FinWait1,
            7 => Self::FinWait2,
            8 => Self::CloseWait,
            9 => Self::Closing,
            10 => Self::LastAck,
            11 => Self::TimeWait,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Established => "Established",
            Self::SynSent => "Connecting",
            Self::SynRecv => "Connecting",
            Self::FinWait1 | Self::FinWait2 | Self::Closing | Self::LastAck => "Closing",
            Self::TimeWait => "Time wait",
            Self::Close => "Closed",
            Self::CloseWait => "Close wait",
            Self::Listen => "Listening",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub local: SocketAddr,
    pub remote: SocketAddr,
    pub state: TcpState,
    pub pid: Option<u32>,
    /// Process name, empty when unknown (e.g. another user's process without elevated rights).
    pub process: String,
}

impl Connection {
    /// Outgoing traffic worth listing: connected to a real remote address (not a listener).
    pub fn is_active(&self) -> bool {
        let ip = self.remote.ip().to_canonical();
        self.state == TcpState::Established && !ip.is_unspecified()
    }

    /// Also true for IPv4 loopback in IPv6 clothing (`::ffff:127.0.0.1`).
    pub fn is_loopback(&self) -> bool {
        self.remote.ip().to_canonical().is_loopback()
    }

    /// Remote address for display: `::ffff:1.2.3.4` is shown as `1.2.3.4:443`.
    pub fn remote_display(&self) -> String {
        match self.remote.ip().to_canonical() {
            std::net::IpAddr::V4(v4) => format!("{v4}:{}", self.remote.port()),
            std::net::IpAddr::V6(v6) => format!("[{v6}]:{}", self.remote.port()),
        }
    }
}

/// One `/proc/net/tcp` row: `(local, remote, state, inode)`.
pub type ProcTcpRow = (SocketAddr, SocketAddr, TcpState, u64);

/// `0100007F:0035` (IPv4) or `<32 hex digits>:0035` (IPv6). The kernel prints each 32-bit word
/// in host byte order, so the bytes are taken back with `to_ne_bytes` (right on any endianness).
fn parse_proc_addr(s: &str) -> Option<SocketAddr> {
    let (ip, port) = s.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let ip = match ip.len() {
        8 => IpAddr::V4(Ipv4Addr::from(u32::from_str_radix(ip, 16).ok()?.to_ne_bytes())),
        32 => {
            let mut bytes = [0u8; 16];
            for (i, chunk) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let word = u32::from_str_radix(ip.get(i * 8..i * 8 + 8)?, 16).ok()?;
                chunk.copy_from_slice(&word.to_ne_bytes());
            }
            IpAddr::V6(Ipv6Addr::from(bytes))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

/// Parses the text of `/proc/net/tcp` or `/proc/net/tcp6` (header line is skipped; bad lines too).
pub fn parse_proc_net_tcp(text: &str) -> Vec<ProcTcpRow> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut f = line.split_ascii_whitespace();
            f.next()?; // "0:"
            let local = parse_proc_addr(f.next()?)?;
            let remote = parse_proc_addr(f.next()?)?;
            let state = TcpState::from_linux(u8::from_str_radix(f.next()?, 16).ok()?);
            // tx:rx queue, tr:tm->when, retrnsmt, uid, timeout, then the socket inode.
            let inode = f.nth(5)?.parse().ok()?;
            Some((local, remote, state, inode))
        })
        .collect()
}

/// Windows IP Helper stores ports as a network-order value in the low 16 bits of a `u32`.
pub fn windows_port(raw: u32) -> u16 {
    u16::from_be(raw as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0035 00000000:0000 0A 00000000:00000000 00:00000000 00000000   101        0 12345 1 0000000000000000 100 0 0 10 0
   1: 0F02000A:C350 5DB8D822:01BB 01 00000000:00000000 02:000A5F2C 00000000  1000        0 67890 2 0000000000000000 20 4 30 10 -1
garbage line
";

    #[test]
    fn parses_ipv4_rows() {
        let rows = parse_proc_net_tcp(TCP);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "127.0.0.1:53".parse().unwrap());
        assert_eq!(rows[0].2, TcpState::Listen);
        assert_eq!(rows[0].3, 12345);
        assert_eq!(rows[1].0, "10.0.2.15:50000".parse().unwrap());
        assert_eq!(rows[1].1, "34.216.184.93:443".parse().unwrap());
        assert_eq!(rows[1].2, TcpState::Established);
        assert_eq!(rows[1].3, 67890);
    }

    #[test]
    fn parses_ipv6_loopback() {
        // ::1 as printed by the kernel on a little-endian machine.
        let a = parse_proc_addr("00000000000000000000000001000000:0050");
        if cfg!(target_endian = "little") {
            assert_eq!(a, Some("[::1]:80".parse().unwrap()));
        }
        assert!(parse_proc_addr("xyz:0050").is_none());
        assert!(parse_proc_addr("0100007F").is_none());
    }

    #[test]
    fn state_codes() {
        assert_eq!(TcpState::from_linux(1), TcpState::Established);
        assert_eq!(TcpState::from_windows(5), TcpState::Established);
        assert_eq!(TcpState::from_windows(2), TcpState::Listen);
        assert_eq!(TcpState::from_linux(99), TcpState::Unknown);
        assert_eq!(TcpState::Established.label(), "Established");
    }

    #[test]
    fn windows_ports_are_byte_swapped() {
        // Port 443 = 0x01BB; network order in the low 16 bits reads 0xBB01 as a native u16.
        assert_eq!(windows_port(u16::to_be(443) as u32), 443);
    }

    #[test]
    fn active_means_established_with_a_real_peer() {
        let mk = |remote: &str, state| Connection {
            local: "10.0.0.2:5000".parse().unwrap(),
            remote: remote.parse().unwrap(),
            state,
            pid: None,
            process: String::new(),
        };
        assert!(mk("1.2.3.4:443", TcpState::Established).is_active());
        assert!(!mk("1.2.3.4:443", TcpState::TimeWait).is_active());
        assert!(!mk("0.0.0.0:0", TcpState::Established).is_active());
        assert!(mk("127.0.0.1:80", TcpState::Established).is_loopback());
        assert!(mk("[::ffff:127.0.0.1]:80", TcpState::Established).is_loopback());
        assert!(!mk("[::ffff:1.2.3.4]:443", TcpState::Established).is_loopback());
        assert_eq!(mk("[::ffff:1.2.3.4]:443", TcpState::Established).remote_display(), "1.2.3.4:443");
        assert_eq!(mk("[2001:db8::1]:443", TcpState::Established).remote_display(), "[2001:db8::1]:443");
    }
}
