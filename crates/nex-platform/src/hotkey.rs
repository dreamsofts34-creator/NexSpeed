//! Global hotkey (Ctrl+Alt+N) that shows/hides the overlay. Works on Windows and X11; on Wayland
//! there is no standard protocol, so registering fails and the tray menu stays the way to toggle.
//!
//! On Windows the manager must be created on the thread that runs the Win32 message loop
//! (the main thread, where Slint's loop runs). Keep the returned value alive for as long as
//! the hotkey should work.

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

pub struct Hotkey {
    _manager: GlobalHotKeyManager,
}

/// `on_press` is called on whatever thread the OS delivers the event on.
pub fn register(on_press: impl Fn() + Send + Sync + 'static) -> Result<Hotkey, String> {
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyN);
    let id = hotkey.id();
    manager.register(hotkey).map_err(|e| e.to_string())?;
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.id == id && event.state == HotKeyState::Pressed {
            on_press();
        }
    }));
    Ok(Hotkey { _manager: manager })
}
