//! Linux tray via StatusNotifierItem (`ksni`). Runs its own D-Bus thread. If no
//! StatusNotifierWatcher exists (e.g. stock GNOME without the AppIndicator extension),
//! `new` returns an error and the app keeps working without a tray.

use std::cell::RefCell;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::MenuItem;

use crate::icon::tray_icon_rgba;
use crate::tray::{CommandHandler, TrayCommand, TrayView};

struct SniTray {
    view: TrayView,
    handler: CommandHandler,
}

fn checkmark(label: &str, checked: bool, cmd: TrayCommand) -> MenuItem<SniTray> {
    CheckmarkItem {
        label: label.into(),
        checked,
        activate: Box::new(move |t: &mut SniTray| (t.handler)(cmd)),
        ..Default::default()
    }
    .into()
}

fn action(label: &str, cmd: TrayCommand) -> MenuItem<SniTray> {
    StandardItem {
        label: label.into(),
        activate: Box::new(move |t: &mut SniTray| (t.handler)(cmd)),
        ..Default::default()
    }
    .into()
}

impl ksni::Tray for SniTray {
    fn id(&self) -> String {
        "nexspeed".into()
    }

    fn title(&self) -> String {
        "Nex Speed".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // SNI wants ARGB32, bytes ordered A,R,G,B.
        let data = tray_icon_rgba(32)
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[3], p[0], p[1], p[2]])
            .collect();
        vec![ksni::Icon { width: 32, height: 32, data }]
    }

    /// Left click on the icon opens the details window (like on Windows).
    fn activate(&mut self, _x: i32, _y: i32) {
        (self.handler)(TrayCommand::ShowDetails);
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Nex Speed".into(),
            description: String::new(),
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let v = &self.view;
        vec![
            StandardItem { label: v.usage.clone(), enabled: false, ..Default::default() }.into(),
            action("Open details", TrayCommand::ShowDetails),
            action("Usage history…", TrayCommand::ShowHistory),
            action("Active connections…", TrayCommand::ShowConnections),
            action("Settings…", TrayCommand::ShowSettings),
            MenuItem::Separator,
            checkmark("Show overlay", v.overlay_visible, TrayCommand::ToggleOverlay),
            checkmark("Hide in full-screen apps", v.auto_hide, TrayCommand::ToggleAutoHide),
            checkmark("Start with system", v.autostart, TrayCommand::ToggleAutostart),
            checkmark("Show speed in bits (Mbps)", v.bits, TrayCommand::ToggleUnits),
            action(&v.limit_label, TrayCommand::CycleLimit),
            MenuItem::Separator,
            action(&v.style_label, TrayCommand::CycleStyle),
            action(&v.theme_label, TrayCommand::CycleTheme),
            action(&v.opacity_label, TrayCommand::CycleOpacity),
            action("Reset overlay position", TrayCommand::ResetPosition),
            MenuItem::Separator,
            action("Quit", TrayCommand::Quit),
        ]
    }
}

pub struct Tray {
    handle: Handle<SniTray>,
    last: RefCell<TrayView>,
}

impl Tray {
    pub fn new(handler: CommandHandler, view: &TrayView) -> Result<Self, String> {
        let handle = SniTray { view: view.clone(), handler }.spawn().map_err(|e| e.to_string())?;
        Ok(Self { handle, last: RefCell::new(view.clone()) })
    }

    /// Same as [`Tray::update`]; kept so both platforms offer the same API.
    pub fn force_update(&self, view: &TrayView) {
        *self.last.borrow_mut() = TrayView::default();
        self.update(view);
    }

    /// Only talks to D-Bus when something visible in the menu changed (the tooltip is static
    /// here), so calling this every second costs nothing.
    pub fn update(&self, view: &TrayView) {
        let mut last = self.last.borrow_mut();
        if last.same_menu(view) {
            return;
        }
        *last = view.clone();
        let v = view.clone();
        let _ = self.handle.update(move |t: &mut SniTray| t.view = v);
    }
}
