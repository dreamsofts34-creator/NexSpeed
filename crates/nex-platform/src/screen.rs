//! Usable screen areas (monitors without taskbars / panels), primary monitor first.
//! Used to keep the overlay on a real monitor and to snap it to screen edges.
//! An empty list means "unknown": callers then leave the window wherever the OS put it.

use nex_core::Rect;

/// Windows: every monitor's work area (the part not covered by the taskbar).
#[cfg(windows)]
pub fn work_areas() -> Vec<Rect> {
    use std::mem::size_of;

    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};

    /// `MONITORINFOF_PRIMARY`
    const PRIMARY_FLAG: u32 = 1;

    unsafe extern "system" fn collect(monitor: HMONITOR, _hdc: HDC, _clip: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the pointer to the Vec created below; it outlives the whole enumeration.
        let found = unsafe { &mut *(data.0 as *mut Vec<(Rect, bool)>) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT::default(),
            rcWork: RECT::default(),
            dwFlags: 0,
        };
        // SAFETY: `info` is a valid MONITORINFO with `cbSize` set.
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let w = info.rcWork;
            found.push((Rect::new(w.left, w.top, w.right - w.left, w.bottom - w.top), info.dwFlags & PRIMARY_FLAG != 0));
        }
        TRUE
    }

    let mut found: Vec<(Rect, bool)> = Vec::new();
    // SAFETY: the callback only touches the Vec behind the LPARAM, which stays alive during the call.
    let ok = unsafe { EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut found as *mut Vec<(Rect, bool)> as isize)) };
    if !ok.as_bool() {
        return Vec::new();
    }
    found.sort_by_key(|&(_, primary)| !primary);
    found.into_iter().map(|(rect, _)| rect).collect()
}

/// Linux (X11, also XWayland): monitor geometry from `xrandr`, cut to the desktop work area
/// from `xprop -root _NET_WORKAREA` so panels are respected. Missing tools or a pure Wayland
/// session give an empty list.
#[cfg(target_os = "linux")]
pub fn work_areas() -> Vec<Rect> {
    if std::env::var_os("DISPLAY").is_none() {
        return Vec::new();
    }
    let Some(xrandr) = run("xrandr", &["--current"]) else { return Vec::new() };
    let monitors = nex_core::parse_xrandr(&xrandr);
    let work = run("xprop", &["-root", "_NET_WORKAREA"]).and_then(|t| nex_core::parse_net_workarea(&t));
    nex_core::apply_workarea(&monitors, work)
}

#[cfg(target_os = "linux")]
fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
