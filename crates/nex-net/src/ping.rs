//! Latency without ICMP (raw ICMP needs privileges on Linux): time a TCP handshake to
//! well-known public resolvers by IP, so DNS is not part of the measurement.

use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

const TARGETS: [&str; 2] = ["1.1.1.1:443", "8.8.8.8:443"];
const TIMEOUT: Duration = Duration::from_millis(1500);

/// Round-trip time of a TCP handshake in milliseconds, or `None` if no target answers.
pub fn tcp_latency_ms() -> Option<f64> {
    for target in TARGETS {
        let Ok(addr) = target.parse::<SocketAddr>() else { continue };
        let start = Instant::now();
        if TcpStream::connect_timeout(&addr, TIMEOUT).is_ok() {
            return Some(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_valid_socket_addresses() {
        assert!(TARGETS.iter().all(|t| t.parse::<SocketAddr>().is_ok()));
    }
}
