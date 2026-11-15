//! Is a full-screen app (game, video, presentation) in front?

/// Windows: `SHQueryUserNotificationState` is the shell's own "don't disturb" check.
/// 2 = full-screen app or presentation settings, 3 = exclusive-mode Direct3D full screen,
/// 4 = presentation mode.
#[cfg(windows)]
pub fn fullscreen_app_active() -> bool {
    use windows::Win32::UI::Shell::SHQueryUserNotificationState;
    // SAFETY: no pointers are passed; the call only reads shell state.
    match unsafe { SHQueryUserNotificationState() } {
        Ok(state) => matches!(state.0, 2 | 3 | 4),
        Err(_) => false,
    }
}

/// Linux (X11, also XWayland): asks `xprop` for the active window and its `_NET_WM_STATE`.
/// Pure Wayland sessions have no standard way, so they report `false`. The answer is cached
/// for a few seconds so the two short `xprop` runs don't happen every tick.
#[cfg(target_os = "linux")]
pub fn fullscreen_app_active() -> bool {
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    thread_local! {
        static CACHE: Cell<Option<(Instant, bool)>> = const { Cell::new(None) };
    }
    const MAX_AGE: Duration = Duration::from_secs(3);

    if let Some((at, value)) = CACHE.with(|c| c.get()) {
        if at.elapsed() < MAX_AGE {
            return value;
        }
    }
    let value = x11_fullscreen_active();
    CACHE.with(|c| c.set(Some((Instant::now(), value))));
    value
}

#[cfg(target_os = "linux")]
fn xprop(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("xprop").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(target_os = "linux")]
fn x11_fullscreen_active() -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    let Some(root) = xprop(&["-root", "_NET_ACTIVE_WINDOW"]) else { return false };
    let Some(id) = parse_active_window(&root) else { return false };
    xprop(&["-id", &id, "_NET_WM_STATE"]).is_some_and(|s| has_fullscreen_state(&s))
}

/// `_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3a00007` -> `0x3a00007` (not for "0x0" = none).
#[cfg(any(target_os = "linux", test))]
fn parse_active_window(text: &str) -> Option<String> {
    let id = text.split_once('#')?.1.split_whitespace().next()?;
    (id.starts_with("0x") && id != "0x0").then(|| id.to_owned())
}

/// `_NET_WM_STATE(ATOM) = _NET_WM_STATE_MAXIMIZED_VERT, _NET_WM_STATE_FULLSCREEN`
#[cfg(any(target_os = "linux", test))]
fn has_fullscreen_state(text: &str) -> bool {
    text.split(|c: char| c == ',' || c == '=' || c.is_whitespace()).any(|t| t == "_NET_WM_STATE_FULLSCREEN")
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn fullscreen_app_active() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_active_window_id() {
        assert_eq!(parse_active_window("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3a00007\n"), Some("0x3a00007".into()));
        assert_eq!(parse_active_window("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0"), None);
        assert_eq!(parse_active_window("_NET_ACTIVE_WINDOW:  not found."), None);
    }

    #[test]
    fn detects_fullscreen_state() {
        assert!(has_fullscreen_state("_NET_WM_STATE(ATOM) = _NET_WM_STATE_MAXIMIZED_VERT, _NET_WM_STATE_FULLSCREEN"));
        assert!(!has_fullscreen_state("_NET_WM_STATE(ATOM) = _NET_WM_STATE_MAXIMIZED_VERT"));
        assert!(!has_fullscreen_state("_NET_WM_STATE(ATOM) = _NET_WM_STATE_FULLSCREEN_EXTRA"));
    }
}
