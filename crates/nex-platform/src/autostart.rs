//! Start with the OS. Windows: registry `Run` key of the current user. Linux: XDG autostart.

use auto_launch::AutoLaunchBuilder;

const APP_NAME: &str = "NexSpeed";

fn build() -> Result<auto_launch::AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(&exe.to_string_lossy())
        .build()
        .map_err(|e| e.to_string())
}

pub fn is_enabled() -> bool {
    build().and_then(|a| a.is_enabled().map_err(|e| e.to_string())).unwrap_or(false)
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let auto = build()?;
    let result = if on { auto.enable() } else { auto.disable() };
    result.map_err(|e| e.to_string())
}
