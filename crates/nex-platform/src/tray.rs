use std::sync::Arc;

/// What the user can ask for from the tray menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    ShowDetails,
    ToggleOverlay,
    ToggleAutoHide,
    ToggleAutostart,
    ToggleUnits,
    CycleLimit,
    CycleStyle,
    CycleTheme,
    CycleOpacity,
    Quit,
}

/// Everything the tray needs to draw itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayView {
    pub overlay_visible: bool,
    /// Hide the overlay while a full-screen app is in front.
    pub auto_hide: bool,
    pub autostart: bool,
    /// Show bits per second (Mbps) instead of bytes per second.
    pub bits: bool,
    /// Read-only first menu line, e.g. `Today 3.20 GB  ·  Month 48.7 GB`.
    pub usage: String,
    /// e.g. `Monthly limit: 100 GB` (click cycles through the choices).
    pub limit_label: String,
    /// e.g. `Style: Graph` (click cycles).
    pub style_label: String,
    /// e.g. `Theme: Dark`.
    pub theme_label: String,
    /// e.g. `Opacity: 85%`.
    pub opacity_label: String,
    /// Hover text (live speed). Windows only; the Linux tray ignores it.
    pub tooltip: String,
}

impl TrayView {
    /// True when nothing that is visible in the menu differs (the tooltip is ignored).
    pub fn same_menu(&self, other: &TrayView) -> bool {
        self.overlay_visible == other.overlay_visible
            && self.auto_hide == other.auto_hide
            && self.autostart == other.autostart
            && self.bits == other.bits
            && self.usage == other.usage
            && self.limit_label == other.limit_label
            && self.style_label == other.style_label
            && self.theme_label == other.theme_label
            && self.opacity_label == other.opacity_label
    }
}

/// Called from whatever thread the OS delivers menu clicks on, so it must be `Send + Sync`.
pub type CommandHandler = Arc<dyn Fn(TrayCommand) + Send + Sync + 'static>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_menu_ignores_tooltip_only() {
        let a = TrayView { tooltip: "a".into(), ..Default::default() };
        let b = TrayView { tooltip: "b".into(), ..Default::default() };
        assert!(a.same_menu(&b));
        let c = TrayView { bits: true, ..Default::default() };
        assert!(!a.same_menu(&c));
    }
}
