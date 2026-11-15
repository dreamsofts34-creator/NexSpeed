//! Active TCP connections with the owning process. Only called while the Connections window
//! is open, so it costs nothing at idle.

use std::io;

use nex_core::Connection;

/// All TCP connections (IPv4 + IPv6) the OS reports, with process info where it is allowed.
pub fn list() -> io::Result<Vec<Connection>> {
    imp::list()
}

#[cfg(target_os = "linux")]
mod imp {
    //! `/proc/net/tcp{,6}` gives the socket inode of each connection; `/proc/<pid>/fd/*` links
    //! of the form `socket:[inode]` tell which process owns it. Other users' processes are not
    //! readable without root, so those rows simply have no process name.

    use std::collections::HashMap;
    use std::fs;
    use std::io;

    use nex_core::{parse_proc_net_tcp, Connection};

    fn socket_owners() -> HashMap<u64, (u32, String)> {
        let mut map = HashMap::new();
        let Ok(procs) = fs::read_dir("/proc") else { return map };
        for entry in procs.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(fds) = fs::read_dir(entry.path().join("fd")) else { continue };
            let mut name: Option<String> = None;
            for fd in fds.flatten() {
                let Ok(target) = fs::read_link(fd.path()) else { continue };
                let target = target.to_string_lossy();
                let Some(inode) = target
                    .strip_prefix("socket:[")
                    .and_then(|s| s.strip_suffix(']'))
                    .and_then(|s| s.parse::<u64>().ok())
                else {
                    continue;
                };
                let n = name.get_or_insert_with(|| {
                    fs::read_to_string(entry.path().join("comm"))
                        .map(|s| s.trim().to_owned())
                        .unwrap_or_default()
                });
                map.insert(inode, (pid, n.clone()));
            }
        }
        map
    }

    pub fn list() -> io::Result<Vec<Connection>> {
        let mut rows = Vec::new();
        let mut any = false;
        for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
            if let Ok(text) = fs::read_to_string(file) {
                any = true;
                rows.extend(parse_proc_net_tcp(&text));
            }
        }
        if !any {
            return Err(io::Error::new(io::ErrorKind::NotFound, "/proc/net/tcp not readable"));
        }
        let owners = socket_owners();
        Ok(rows
            .into_iter()
            .map(|(local, remote, state, inode)| {
                let (pid, process) = match owners.get(&inode) {
                    Some((pid, name)) => (Some(*pid), name.clone()),
                    None => (None, String::new()),
                };
                Connection { local, remote, state, pid, process }
            })
            .collect())
    }
}

#[cfg(windows)]
mod imp {
    //! IP Helper `GetExtendedTcpTable` (owner-PID variant). Process names come from
    //! `QueryFullProcessImageNameW` with the limited-query right, which works for most
    //! processes without admin; protected ones just show no name.

    use std::io;
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    use nex_core::{windows_port, Connection, TcpState};
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, WIN32_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
        MIB_TCPTABLE_OWNER_PID, TCP_TABLE_CLASS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    const AF_INET: u32 = 2;
    const AF_INET6: u32 = 23;
    const NO_ERROR: u32 = 0;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    /// `TCP_TABLE_OWNER_PID_ALL`
    const TCP_TABLE_OWNER_PID_ALL: TCP_TABLE_CLASS = TCP_TABLE_CLASS(5);

    /// The return type of `GetExtendedTcpTable` differs between `windows` crate versions
    /// (plain `u32` or `WIN32_ERROR`); this makes the code compile with either.
    trait ErrCode {
        fn code(&self) -> u32;
    }
    impl ErrCode for u32 {
        fn code(&self) -> u32 {
            *self
        }
    }
    impl ErrCode for WIN32_ERROR {
        fn code(&self) -> u32 {
            self.0
        }
    }

    fn process_name(pid: u32) -> String {
        if pid == 0 {
            return String::new();
        }
        // SAFETY: plain Win32 calls; the handle is closed before returning and `buf`/`len`
        // outlive the call.
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return String::new();
            };
            let mut buf = [0u16; 520];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
            let _ = CloseHandle(handle);
            if ok.is_err() {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]);
            path.rsplit(['\\', '/']).next().unwrap_or("").to_owned()
        }
    }

    /// Fetches the raw table for one address family, retrying if it grows between calls.
    fn fetch(af: u32) -> io::Result<Vec<u8>> {
        let mut size: u32 = 0;
        let mut buf: Vec<u8> = Vec::new();
        for _ in 0..4 {
            let ptr = if buf.is_empty() { None } else { Some(buf.as_mut_ptr() as *mut core::ffi::c_void) };
            // SAFETY: `ptr` is either null (size query) or points to `size` writable bytes.
            let rc = unsafe { GetExtendedTcpTable(ptr, &mut size, false, af, TCP_TABLE_OWNER_PID_ALL, 0) }.code();
            match rc {
                NO_ERROR if !buf.is_empty() => return Ok(buf),
                NO_ERROR | ERROR_INSUFFICIENT_BUFFER => buf = vec![0u8; size as usize],
                other => return Err(io::Error::from_raw_os_error(other as i32)),
            }
        }
        Err(io::Error::new(io::ErrorKind::Other, "TCP table kept changing size"))
    }

    pub fn list() -> io::Result<Vec<Connection>> {
        let mut names: HashMap<u32, String> = HashMap::new();
        let mut out = parse_v4(&fetch(AF_INET)?, &mut names);
        // IPv6 is best effort: a failure here must not hide the IPv4 list.
        if let Ok(buf) = fetch(AF_INET6) {
            out.extend(parse_v6(&buf, &mut names));
        }
        Ok(out)
    }

    fn name_for(names: &mut HashMap<u32, String>, pid: u32) -> String {
        names.entry(pid).or_insert_with(|| process_name(pid)).clone()
    }

    fn parse_v4(buf: &[u8], names: &mut HashMap<u32, String>) -> Vec<Connection> {
        // SAFETY: the buffer was filled by GetExtendedTcpTable (header + `dwNumEntries` rows);
        // `read_unaligned` because a Vec<u8> has no alignment guarantee.
        unsafe {
            let table = buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
            let count = std::ptr::addr_of!((*table).dwNumEntries).read_unaligned() as usize;
            let rows = std::ptr::addr_of!((*table).table) as *const MIB_TCPROW_OWNER_PID;
            (0..count)
                .map(|i| {
                    let r = rows.add(i).read_unaligned();
                    Connection {
                        local: SocketAddr::new(
                            IpAddr::V4(Ipv4Addr::from(r.dwLocalAddr.to_ne_bytes())),
                            windows_port(r.dwLocalPort),
                        ),
                        remote: SocketAddr::new(
                            IpAddr::V4(Ipv4Addr::from(r.dwRemoteAddr.to_ne_bytes())),
                            windows_port(r.dwRemotePort),
                        ),
                        state: TcpState::from_windows(r.dwState),
                        pid: Some(r.dwOwningPid),
                        process: name_for(names, r.dwOwningPid),
                    }
                })
                .collect()
        }
    }

    fn parse_v6(buf: &[u8], names: &mut HashMap<u32, String>) -> Vec<Connection> {
        // SAFETY: same as `parse_v4`, for the IPv6 table layout.
        unsafe {
            let table = buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID;
            let count = std::ptr::addr_of!((*table).dwNumEntries).read_unaligned() as usize;
            let rows = std::ptr::addr_of!((*table).table) as *const MIB_TCP6ROW_OWNER_PID;
            (0..count)
                .map(|i| {
                    let r = rows.add(i).read_unaligned();
                    Connection {
                        local: SocketAddr::new(IpAddr::V6(Ipv6Addr::from(r.ucLocalAddr)), windows_port(r.dwLocalPort)),
                        remote: SocketAddr::new(IpAddr::V6(Ipv6Addr::from(r.ucRemoteAddr)), windows_port(r.dwRemotePort)),
                        state: TcpState::from_windows(r.dwState),
                        pid: Some(r.dwOwningPid),
                        process: name_for(names, r.dwOwningPid),
                    }
                })
                .collect()
        }
    }
}
