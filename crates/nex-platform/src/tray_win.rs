//! Windows tray via `tray-icon`. Must be created on the thread that runs the Win32 message
//! loop; Slint's winit loop runs on the main thread and pumps those messages.

use std::cell::RefCell;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::icon::tray_icon_rgba;
use crate::tray::{CommandHandler, TrayCommand, TrayView};

pub struct Tray {
    icon: TrayIcon,
    /// What the menu and tooltip currently show, so unchanged values cost no Win32 calls.
    last: RefCell<TrayView>,
    usage: MenuItem,
    overlay: CheckMenuItem,
    auto_hide: CheckMenuItem,
    autostart: CheckMenuItem,
    bits: CheckMenuItem,
    limit: MenuItem,
    style: MenuItem,
    theme: MenuItem,
    opacity: MenuItem,
}

impl Tray {
    pub fn new(handler: CommandHandler, view: &TrayView) -> Result<Self, String> {
        let usage = MenuItem::new(&view.usage, false, None);
        let details = MenuItem::new("Open details", true, None);
        let history = MenuItem::new("Usage history…", true, None);
        let connections = MenuItem::new("Active connections…", true, None);
        let settings = MenuItem::new("Settings…", true, None);
        let overlay = CheckMenuItem::new("Show overlay", true, view.overlay_visible, None);
        let auto_hide = CheckMenuItem::new("Hide in full-screen apps", true, view.auto_hide, None);
        let autostart = CheckMenuItem::new("Start with Windows", true, view.autostart, None);
        let bits = CheckMenuItem::new("Show speed in bits (Mbps)", true, view.bits, None);
        let limit = MenuItem::new(&view.limit_label, true, None);
        let style = MenuItem::new(&view.style_label, true, None);
        let theme = MenuItem::new(&view.theme_label, true, None);
        let opacity = MenuItem::new(&view.opacity_label, true, None);
        let reset = MenuItem::new("Reset overlay position", true, None);
        let quit = MenuItem::new("Quit", true, None);

        let menu = Menu::new();
        let err = |e: tray_icon::menu::Error| e.to_string();
        menu.append(&usage).map_err(err)?;
        menu.append(&details).map_err(err)?;
        menu.append(&history).map_err(err)?;
        menu.append(&connections).map_err(err)?;
        menu.append(&settings).map_err(err)?;
        menu.append(&PredefinedMenuItem::separator()).map_err(err)?;
        menu.append(&overlay).map_err(err)?;
        menu.append(&auto_hide).map_err(err)?;
        menu.append(&autostart).map_err(err)?;
        menu.append(&bits).map_err(err)?;
        menu.append(&limit).map_err(err)?;
        menu.append(&PredefinedMenuItem::separator()).map_err(err)?;
        menu.append(&style).map_err(err)?;
        menu.append(&theme).map_err(err)?;
        menu.append(&opacity).map_err(err)?;
        menu.append(&reset).map_err(err)?;
        menu.append(&PredefinedMenuItem::separator()).map_err(err)?;
        menu.append(&quit).map_err(err)?;

        let ids = [
            (details.id().clone(), TrayCommand::ShowDetails),
            (history.id().clone(), TrayCommand::ShowHistory),
            (connections.id().clone(), TrayCommand::ShowConnections),
            (settings.id().clone(), TrayCommand::ShowSettings),
            (overlay.id().clone(), TrayCommand::ToggleOverlay),
            (auto_hide.id().clone(), TrayCommand::ToggleAutoHide),
            (autostart.id().clone(), TrayCommand::ToggleAutostart),
            (bits.id().clone(), TrayCommand::ToggleUnits),
            (limit.id().clone(), TrayCommand::CycleLimit),
            (style.id().clone(), TrayCommand::CycleStyle),
            (theme.id().clone(), TrayCommand::CycleTheme),
            (opacity.id().clone(), TrayCommand::CycleOpacity),
            (reset.id().clone(), TrayCommand::ResetPosition),
            (quit.id().clone(), TrayCommand::Quit),
        ];
        let menu_handler = handler.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some((_, cmd)) = ids.iter().find(|(id, _)| *id == event.id) {
                menu_handler(*cmd);
            }
        }));
        // Left click opens the details window; right click still shows the menu.
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                handler(TrayCommand::ShowDetails);
            }
        }));

        let rgba = tray_icon_rgba(32);
        let icon = Icon::from_rgba(rgba, 32, 32).map_err(|e| e.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(&view.tooltip)
            .with_icon(icon)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            icon: tray,
            last: RefCell::new(view.clone()),
            usage,
            overlay,
            auto_hide,
            autostart,
            bits,
            limit,
            style,
            theme,
            opacity,
        })
    }

    /// Re-applies everything, e.g. after a menu click: a check item flips itself natively, and
    /// if the app refused the change the menu would otherwise stay out of sync.
    pub fn force_update(&self, view: &TrayView) {
        *self.last.borrow_mut() = TrayView::default();
        self.update(view);
    }

    /// Only touches the menu / tooltip when something visible changed, so calling this every
    /// second while nothing happens costs nothing.
    pub fn update(&self, view: &TrayView) {
        let mut last = self.last.borrow_mut();
        let menu_changed = !last.same_menu(view);
        let tooltip_changed = last.tooltip != view.tooltip;
        if !menu_changed && !tooltip_changed {
            return;
        }
        if menu_changed {
            self.usage.set_text(&view.usage);
            self.overlay.set_checked(view.overlay_visible);
            self.auto_hide.set_checked(view.auto_hide);
            self.autostart.set_checked(view.autostart);
            self.bits.set_checked(view.bits);
            self.limit.set_text(&view.limit_label);
            self.style.set_text(&view.style_label);
            self.theme.set_text(&view.theme_label);
            self.opacity.set_text(&view.opacity_label);
        }
        if tooltip_changed {
            let _ = self.icon.set_tooltip(Some(&view.tooltip));
        }
        *last = view.clone();
    }
}
