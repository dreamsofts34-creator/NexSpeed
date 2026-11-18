//! Single-instance guard without extra dependencies: the first instance listens on a fixed
//! loopback port; a second launch connects, recognises the greeting, and exits. The first
//! instance treats every connection as "show the details window". The port is released by the
//! OS when the process ends, so a crash can never leave a stale lock behind.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::time::Duration;

const PORT: u16 = 47_836;
const GREETING: &[u8; 9] = b"NEXSPEED1";

pub enum Guard {
    /// We are the first instance: keep the listener and call [`serve`].
    Primary(TcpListener),
    /// Another Nex Speed is running and has been asked to show itself: exit.
    Secondary,
    /// The port is taken by something else (or unusable): run without a guard.
    Unguarded,
}

pub fn acquire() -> Guard {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, PORT));
    match TcpListener::bind(addr) {
        Ok(listener) => Guard::Primary(listener),
        Err(_) => {
            let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
                return Guard::Unguarded;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let mut buf = [0u8; GREETING.len()];
            if stream.read_exact(&mut buf).is_ok() && &buf == GREETING {
                Guard::Secondary
            } else {
                Guard::Unguarded
            }
        }
    }
}

/// Answers second launches in a background thread; `on_second_launch` runs for each one.
pub fn serve(listener: TcpListener, on_second_launch: impl Fn() + Send + 'static) {
    let _ = std::thread::Builder::new().name("instance-guard".into()).spawn(move || {
        for mut s in listener.incoming().flatten() {
            if s.write_all(GREETING).is_ok() {
                on_second_launch();
            }
        }
    });
}
