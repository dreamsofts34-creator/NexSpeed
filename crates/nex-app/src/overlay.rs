//! The floating overlay, tray and details window. Everything UI-related runs on the UI thread
//! from one 1-second timer (reading the counters takes microseconds), so there are no
//! permanent extra threads. Short-lived workers (latency probe, speed test) and tray clicks
//! report back with `invoke_from_event_loop`, so nothing polls.

use std::cell::{Cell, RefCell};
use std::net::TcpListener;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nex_core::{
    clamp_style, format_bytes, format_speed, last_days, limit_fraction, limit_level, limit_percent,
    next_opacity, next_style, restore_position, snap_position, style_name, AdapterReading, Ema,
    NetSource, Sampler, SpeedHistory, UnitSystem, OPACITY_STEPS,
};
use nex_net::speedtest::{self, Phase, Progress, SpeedTestResult};
use nex_platform::{
    autostart, connections, fullscreen_app_active, screen, PlatformSource, Tray, TrayCommand,
    TrayView,
};
use nex_ui::{chart, Colors, ConnRow, Connections, Details, History, Overlay, Preferences};
use slint::ComponentHandle;

use crate::applog;
use crate::instance;
use crate::settings::{next_limit, Settings, LIMIT_STEPS_GB};
use crate::usage_store::{today_key, UsageStore};

/// Connections list refresh interval, in ticks, only while its window is open.
const CONNECTIONS_EVERY_TICKS: u32 = 3;
/// Latency probe interval, in 1-second ticks, only while the details window is open.
const PING_EVERY_TICKS: u32 = 3;
const PING_UNKNOWN: u32 = u32::MAX;
/// Free space kept between the overlay and a screen edge, in logical pixels.
const EDGE_MARGIN: f32 = 8.0;
/// The overlay snaps to a screen edge when released this close to it, in logical pixels.
const SNAP_DISTANCE: f32 = 20.0;
/// Overlay width in logical pixels (see overlay.slint); used before the window has a size.
const OVERLAY_WIDTH: f32 = 232.0;
const OVERLAY_HEIGHT: f32 = 84.0;

struct State {
    source: PlatformSource,
    sampler: Sampler,
    readings: Vec<AdapterReading>,
    history: SpeedHistory,
    ema_rx: Ema,
    ema_tx: Ema,
    peak_rx: f64,
}

/// The newest numbers as shown text, kept so a window that is opened between two ticks can be
/// filled immediately instead of showing zeros for up to a second.
struct Display {
    down: String,
    up: String,
    peak: String,
    adapter: String,
    online: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            down: "0 B/s".to_string(),
            up: "0 B/s".to_string(),
            peak: "0 B/s".to_string(),
            adapter: "No connection".to_string(),
            online: false,
        }
    }
}

struct Shared {
    settings: RefCell<Settings>,
    state: RefCell<State>,
    display: RefCell<Display>,
    tray: RefCell<Option<Tray>>,
    units: Cell<UnitSystem>,
    autostart: Cell<bool>,
    tooltip: RefCell<String>,
    usage: RefCell<UsageStore>,
    /// Overlay hidden because a full-screen app is in front (not a user choice).
    suppressed: Cell<bool>,
    tick: Cell<u32>,
    /// `(month YYYYMM, last limit level)` already notified about; `None` until the first tick.
    notified: Cell<Option<(u32, u8)>>,
    /// `Some(light)` while the overlay shows the flat "nothing happening" graph for that theme,
    /// so an idle connection does not redraw the same picture every second.
    idle_graph: Cell<Option<bool>>,
    /// Written by the full-screen watcher thread (never poll xprop / Win32 on the UI thread).
    fs_active: Arc<AtomicBool>,
    /// Tells the watcher whether anyone needs the answer (overlay shown and auto-hide on).
    watch_enabled: Arc<AtomicBool>,
    /// The user pressed the hotkey / menu while a full-screen app hides the overlay: show it
    /// until that full-screen app is gone.
    force_show: Cell<bool>,
    ping_ms: Arc<AtomicU32>,
    ping_busy: Arc<AtomicBool>,
}

fn limit_label(gb: u32) -> String {
    if gb == 0 {
        "Monthly limit: off".to_string()
    } else {
        format!("Monthly limit: {gb} GB")
    }
}

impl Shared {
    fn month_percent(&self) -> Option<f64> {
        let used = self.usage.borrow().month_bytes(today_key());
        limit_percent(used, self.settings.borrow().monthly_limit_gb)
    }

    fn usage_line(&self) -> String {
        let base = self.usage.borrow().summary(today_key());
        match self.month_percent() {
            Some(p) => format!("{base}  ({p:.0}% of limit)"),
            None => base,
        }
    }

    fn view(&self) -> TrayView {
        let s = self.settings.borrow();
        TrayView {
            overlay_visible: s.overlay_visible,
            auto_hide: s.auto_hide_fullscreen,
            autostart: self.autostart.get(),
            bits: self.units.get() == UnitSystem::Bits,
            usage: self.usage_line(),
            limit_label: limit_label(s.monthly_limit_gb),
            style_label: format!("Style: {}", style_name(s.style)),
            theme_label: format!("Theme: {}", if s.light { "Light" } else { "Dark" }),
            opacity_label: format!("Opacity: {}%", s.opacity_pct),
            tooltip: self.tooltip.borrow().clone(),
        }
    }

    /// Cheap: the tray only touches the OS when something visible changed.
    fn refresh_tray(&self) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.update(&self.view());
        }
    }

    /// Lets the full-screen watcher sleep when nobody needs its answer.
    fn sync_watch(&self) {
        let s = self.settings.borrow();
        self.watch_enabled.store(
            s.overlay_visible && s.auto_hide_fullscreen,
            Ordering::Relaxed,
        );
    }

    /// Re-applies the whole menu (after a click, which may have flipped a check mark natively).
    fn sync_tray(&self) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.force_update(&self.view());
        }
    }
}

/// Tray-menu command handler installed by the overlay.
type CommandFn = Box<dyn Fn(TrayCommand)>;

thread_local! {
    /// Live on the UI thread; background threads reach them through `dispatch` / `with_details`.
    static COMMAND_HANDLER: RefCell<Option<CommandFn>> = const { RefCell::new(None) };
    static DETAILS: RefCell<Option<slint::Weak<Details>>> = const { RefCell::new(None) };
    /// Cancel flag of the speed test that is running right now (UI thread only).
    static SPEEDTEST_CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    /// The connections list model, updated in place so scroll position and hover survive refreshes.
    static CONN_MODEL: RefCell<Option<Rc<slint::VecModel<ConnRow>>>> = const { RefCell::new(None) };
}

/// A connections refresh is running on a worker thread.
static CONN_BUSY: AtomicBool = AtomicBool::new(false);

/// Safe to call from any thread.
fn dispatch(cmd: TrayCommand) {
    let _ = slint::invoke_from_event_loop(move || {
        COMMAND_HANDLER.with(|h| {
            if let Some(f) = h.borrow().as_ref() {
                f(cmd);
            }
        });
    });
}

/// Runs `f` on the UI thread with the details window (if it still exists). Any thread.
fn with_details(f: impl FnOnce(&Details) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        DETAILS.with(|d| {
            if let Some(details) = d.borrow().as_ref().and_then(|w| w.upgrade()) {
                f(&details);
            }
        });
    });
}

fn progress_text(p: Progress) -> String {
    match p {
        Progress::Started(Phase::Latency) => "Measuring latency…".to_string(),
        Progress::Started(Phase::Download) => "Testing download…".to_string(),
        Progress::Started(Phase::Upload) => "Testing upload…".to_string(),
        Progress::Speed(Phase::Download, m) => format!("Download {m:.1} Mbps…"),
        Progress::Speed(Phase::Upload, m) => format!("Upload {m:.1} Mbps…"),
        Progress::Speed(Phase::Latency, _) => "Measuring latency…".to_string(),
    }
}

fn result_text(r: &Result<SpeedTestResult, String>) -> String {
    match r {
        Ok(r) => {
            let mbps = |v: Option<f64>| v.map_or("—".to_string(), |m| format!("{m:.1}"));
            format!(
                "↓ {} Mbps    ↑ {} Mbps    ·    {:.0} ms",
                mbps(r.down_mbps),
                mbps(r.up_mbps),
                r.ping_ms
            )
        }
        Err(e) if e.as_str() == speedtest::CANCELLED => "Speed test cancelled".to_string(),
        Err(e) => {
            let short: String = e.chars().take(48).collect();
            format!("Speed test failed: {short}")
        }
    }
}

fn speedtest_hint() -> String {
    format!("Uses up to about {} MB of data", speedtest::MAX_DATA_MB)
}

fn start_speedtest(details: &Details) {
    let cancel = Arc::new(AtomicBool::new(false));
    SPEEDTEST_CANCEL.with(|c| *c.borrow_mut() = Some(cancel.clone()));
    details.set_speedtest_running(true);
    details.set_speedtest_text("Starting…".into());
    std::thread::spawn(move || {
        let result = speedtest::run(
            &|p| {
                let text = progress_text(p);
                with_details(move |d| d.set_speedtest_text(text.as_str().into()));
            },
            &cancel,
        );
        let text = result_text(&result);
        with_details(move |d| {
            d.set_speedtest_text(text.as_str().into());
            d.set_speedtest_running(false);
            SPEEDTEST_CANCEL.with(|c| *c.borrow_mut() = None);
        });
    });
}

fn cancel_speedtest(details: &Details) {
    SPEEDTEST_CANCEL.with(|c| {
        if let Some(flag) = c.borrow().as_ref() {
            flag.store(true, Ordering::Relaxed);
        }
    });
    details.set_speedtest_text("Cancelling…".into());
}

/// `Saved nexspeed-usage-20261117.csv to Downloads`
fn export_message(path: &Path) -> String {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let file = name(path);
    let folder = path.parent().map(name).unwrap_or_default();
    if folder.is_empty() {
        format!("Saved {file}")
    } else {
        format!("Saved {file} to {folder}")
    }
}

/// What the connections window shows, built on a worker thread (scanning /proc can take a while).
struct ConnData {
    rows: Vec<(String, String)>,
    summary: String,
    note: String,
}

fn load_connections() -> ConnData {
    match connections::list() {
        Ok(all) => {
            let active = |c: &&nex_core::Connection| c.is_active() && !c.is_loopback();
            let mut rows: Vec<(String, String)> = all
                .iter()
                .filter(active)
                .map(|c| {
                    let name = if c.process.is_empty() {
                        "Unknown".to_string()
                    } else {
                        c.process.clone()
                    };
                    (name, c.remote_display())
                })
                .collect();
            rows.sort();
            let hidden = all.iter().filter(active).any(|c| c.process.is_empty());
            let summary = match rows.len() {
                1 => "1 active connection".to_string(),
                n => format!("{n} active connections"),
            };
            let note = if hidden {
                "Some applications are hidden (run as administrator / root to see all)"
            } else {
                ""
            };
            ConnData {
                rows,
                summary,
                note: note.to_string(),
            }
        }
        Err(e) => ConnData {
            rows: Vec::new(),
            summary: "Could not read connections".to_string(),
            note: e.to_string(),
        },
    }
}

fn apply_connections(win: &Connections, data: ConnData) {
    win.set_summary(data.summary.as_str().into());
    win.set_note(data.note.as_str().into());
    let rows: Vec<ConnRow> = data
        .rows
        .into_iter()
        .map(|(process, remote)| ConnRow {
            process: process.as_str().into(),
            remote: remote.as_str().into(),
        })
        .collect();
    CONN_MODEL.with(|cell| {
        let mut slot = cell.borrow_mut();
        match slot.as_ref() {
            Some(model) => model.set_vec(rows),
            None => {
                let model = Rc::new(slint::VecModel::from(rows));
                let dynamic: Rc<dyn slint::Model<Data = ConnRow>> = model.clone();
                win.set_rows(slint::ModelRc::from(dynamic));
                *slot = Some(model);
            }
        }
    });
}

/// Refreshes the connections window without blocking the UI: the scan runs on a short-lived
/// thread (at most one at a time) and the result is applied on the UI thread.
fn refresh_connections(win: &Connections) {
    if CONN_BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    let weak = win.as_weak();
    std::thread::spawn(move || {
        let data = load_connections();
        CONN_BUSY.store(false, Ordering::Release);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(win) = weak.upgrade() {
                apply_connections(&win, data);
            }
        });
    });
}

/// Pushes theme, opacity and style from the settings into every window.
fn apply_look(
    s: &Settings,
    ui: &Overlay,
    d: &Details,
    c: &Connections,
    h: &History,
    p: &Preferences,
) {
    ui.set_style(s.style as i32);
    ui.global::<Colors>()
        .set_opacity(s.opacity_pct as f32 / 100.0);
    ui.global::<Colors>().set_light(s.light);
    d.global::<Colors>().set_light(s.light);
    c.global::<Colors>().set_light(s.light);
    h.global::<Colors>().set_light(s.light);
    p.global::<Colors>().set_light(s.light);
}

/// All windows, as weak handles, so closures can reach any of them.
#[derive(Clone)]
struct Windows {
    ui: slint::Weak<Overlay>,
    details: slint::Weak<Details>,
    conns: slint::Weak<Connections>,
    history: slint::Weak<History>,
    prefs: slint::Weak<Preferences>,
}

/// Shows the current settings in the settings window.
fn push_prefs(p: &Preferences, s: &Settings, sh: &Shared) {
    let index = |found: Option<usize>| found.unwrap_or(0) as i32;
    p.set_style(s.style as i32);
    p.set_theme(i32::from(s.light));
    p.set_opacity_step(index(
        OPACITY_STEPS.iter().position(|&o| o == s.opacity_pct),
    ));
    p.set_units(i32::from(s.bits));
    p.set_limit(index(
        LIMIT_STEPS_GB.iter().position(|&g| g == s.monthly_limit_gb),
    ));
    p.set_auto_hide(i32::from(!s.auto_hide_fullscreen));
    p.set_autostart(i32::from(!sh.autostart.get()));
}

/// After a setting changed: theme / opacity / style everywhere, redraw what depends on them.
fn refresh_look(sh: &Shared, w: &Windows) {
    if let (Some(ui), Some(d), Some(c), Some(h), Some(p)) = (
        w.ui.upgrade(),
        w.details.upgrade(),
        w.conns.upgrade(),
        w.history.upgrade(),
        w.prefs.upgrade(),
    ) {
        // The theme may have changed: force the overlay graph to be redrawn.
        sh.idle_graph.set(None);
        let s = sh.settings.borrow();
        apply_look(&s, &ui, &d, &c, &h, &p);
        push_prefs(&p, &s, sh);
        if h.window().is_visible() {
            refresh_history(&h, sh, h.get_range());
        }
    }
}

/// A choice made in the settings window (the value is the selected index).
enum Change {
    Style(i32),
    Theme(i32),
    Opacity(i32),
    Units(i32),
    Limit(i32),
    AutoHide(i32),
    Autostart(i32),
}

fn apply_change(sh: &Shared, w: &Windows, change: Change) {
    {
        let mut s = sh.settings.borrow_mut();
        match change {
            Change::Style(i) => s.style = clamp_style(i.clamp(0, 255) as u8),
            Change::Theme(i) => s.light = i == 1,
            Change::Opacity(i) => {
                if let Some(&o) = OPACITY_STEPS.get(i as usize) {
                    s.opacity_pct = o;
                }
            }
            Change::Units(i) => {
                s.bits = i == 1;
                sh.units.set(if s.bits {
                    UnitSystem::Bits
                } else {
                    UnitSystem::Bytes
                });
            }
            Change::Limit(i) => {
                if let Some(&g) = LIMIT_STEPS_GB.get(i as usize) {
                    s.monthly_limit_gb = g;
                }
            }
            Change::AutoHide(i) => {
                s.auto_hide_fullscreen = i == 0;
                if !s.auto_hide_fullscreen && sh.suppressed.get() {
                    sh.suppressed.set(false);
                    if let (true, Some(ui)) = (s.overlay_visible, w.ui.upgrade()) {
                        show_overlay(&ui, &s);
                    }
                }
            }
            Change::Autostart(i) => {
                let want = i == 0;
                match autostart::set_enabled(want) {
                    Ok(()) => sh.autostart.set(want),
                    Err(e) => applog::line(&format!("could not change autostart: {e}")),
                }
            }
        }
        s.save();
    }
    sh.sync_watch();
    refresh_look(sh, w);
    sh.sync_tray();
}

fn iso(date: u32) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date / 10_000,
        date / 100 % 100,
        date % 100
    )
}

/// Fills the history window with the last 7 or 30 days of usage.
fn refresh_history(win: &History, sh: &Shared, range: i32) {
    let n = if range >= 30 { 30 } else { 7 };
    let today = today_key();
    let days = sh.usage.borrow().series(today, n);
    let dates = last_days(today, n);
    let (rx, tx) = days.iter().fold((0u64, 0u64), |(a, b), &(r, t)| {
        (a.saturating_add(r), b.saturating_add(t))
    });
    let light = sh.settings.borrow().light;
    win.set_chart(chart::bars_image(800, 312, &days, light));
    win.set_total_text(format_bytes(rx.saturating_add(tx)).as_str().into());
    win.set_down_text(format_bytes(rx).as_str().into());
    win.set_up_text(format_bytes(tx).as_str().into());
    win.set_from_text(iso(dates[0]).as_str().into());
    win.set_to_text(iso(dates[n - 1]).as_str().into());
    let busiest = days
        .iter()
        .enumerate()
        .max_by_key(|e| e.1 .0.saturating_add(e.1 .1))
        .filter(|e| e.1 .0 > 0 || e.1 .1 > 0);
    let peak = match busiest {
        Some((i, d)) => format!(
            "Busiest day: {} ({})",
            iso(dates[i]),
            format_bytes(d.0.saturating_add(d.1))
        ),
        None => "No data yet for this period".to_string(),
    };
    win.set_peak_text(peak.as_str().into());
}

/// Fills the details window from the shared state. Called every tick while it is open and once
/// right before it is shown. Must not run while `settings` is mutably borrowed.
fn update_details(d: &Details, sh: &Shared) {
    let disp = sh.display.borrow();
    let st = sh.state.borrow();
    let light = sh.settings.borrow().light;
    let (down_value, down_unit) = split_value_unit(&disp.down);
    d.set_adapter_name(disp.adapter.as_str().into());
    d.set_online(disp.online);
    d.set_down_value(down_value.into());
    d.set_down_unit(down_unit.into());
    d.set_up_text(disp.up.as_str().into());
    d.set_peak_text(disp.peak.as_str().into());
    d.set_graph(chart::sparkline_image_sized(
        560,
        144,
        &st.history.rx,
        &st.history.tx,
        light,
    ));
    let highest = st
        .history
        .rx
        .iter()
        .chain(st.history.tx.iter())
        .fold(0.0f32, |a, &b| a.max(b));
    d.set_graph_caption(
        format!(
            "Last 60 s · max {}",
            format_speed(highest as f64, sh.units.get())
        )
        .as_str()
        .into(),
    );

    let usage = sh.usage.borrow();
    let today = today_key();
    d.set_today_text(format_bytes(usage.today_bytes(today)).as_str().into());
    d.set_month_text(format_bytes(usage.month_bytes(today)).as_str().into());
    let ping = sh.ping_ms.load(Ordering::Relaxed);
    d.set_ping_text(if ping == PING_UNKNOWN {
        "—".into()
    } else {
        format!("{ping} ms").as_str().into()
    });

    let percent = sh.month_percent();
    let limit_gb = sh.settings.borrow().monthly_limit_gb;
    d.set_has_limit(percent.is_some());
    d.set_limit_alert(limit_level(percent) as i32);
    d.set_limit_fraction(limit_fraction(percent));
    d.set_limit_text(match percent {
        Some(p) => format!("{p:.0}% of your {limit_gb} GB monthly limit")
            .as_str()
            .into(),
        None => "No monthly limit. Choose one in the tray menu.".into(),
    });
}

/// Native notification when the monthly limit crosses 80 % / 100 %. Short-lived thread, ignored on failure.
fn notify_limit(level: u8, percent: f64, limit_gb: u32) {
    let body = if level >= 2 {
        format!("You have reached your {limit_gb} GB monthly limit.")
    } else {
        format!("You have used {percent:.0}% of your {limit_gb} GB monthly limit.")
    };
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .summary("Nex Speed")
            .body(&body)
            .appname("Nex Speed")
            .icon("nexspeed")
            .show();
    });
}

/// `"5.40 MB/s"` -> (`"5.40"`, `"MB/s"`)
fn split_value_unit(s: &str) -> (&str, &str) {
    s.split_once(' ').unwrap_or((s, ""))
}

fn remember_position(ui: &Overlay, settings: &mut Settings) {
    let p = ui.window().position();
    settings.pos_x = Some(p.x);
    settings.pos_y = Some(p.y);
}

/// Ignore wildly off positions (corrupt file).
fn saved_position(s: &Settings) -> Option<(i32, i32)> {
    match (s.pos_x, s.pos_y) {
        (Some(x), Some(y)) if x.abs() < 20_000 && y.abs() < 20_000 => Some((x, y)),
        _ => None,
    }
}

/// Overlay size in physical pixels (a sensible guess while the window has no size yet).
fn overlay_size(ui: &Overlay) -> (i32, i32) {
    let size = ui.window().size();
    if size.width == 0 || size.height == 0 {
        let scale = ui.window().scale_factor();
        return (
            (OVERLAY_WIDTH * scale).round() as i32,
            (OVERLAY_HEIGHT * scale).round() as i32,
        );
    }
    (size.width as i32, size.height as i32)
}

/// Moves the overlay to `saved` if that is still on a monitor, otherwise to the top-right corner
/// of the primary monitor. Returns where it went (nothing when the screen layout is unknown and
/// there was no saved position).
fn place_overlay(ui: &Overlay, saved: Option<(i32, i32)>) -> Option<(i32, i32)> {
    let areas = screen::work_areas();
    let margin = (EDGE_MARGIN * ui.window().scale_factor()).round() as i32;
    let (x, y) = restore_position(saved, overlay_size(ui), &areas, margin)?;
    ui.window().set_position(slint::PhysicalPosition::new(x, y));
    Some((x, y))
}

/// After a drag: snaps to the screen edges when close to them. Returns the new position if it moved.
fn snap_overlay(ui: &Overlay) -> Option<(i32, i32)> {
    let areas = screen::work_areas();
    if areas.is_empty() {
        return None;
    }
    let scale = ui.window().scale_factor();
    let p = ui.window().position();
    let from = (p.x, p.y);
    let to = snap_position(
        from,
        overlay_size(ui),
        &areas,
        (SNAP_DISTANCE * scale).round() as i32,
        (EDGE_MARGIN * scale).round() as i32,
    );
    if to == from {
        return None;
    }
    ui.window()
        .set_position(slint::PhysicalPosition::new(to.0, to.1));
    Some(to)
}

fn show_overlay(ui: &Overlay, settings: &Settings) {
    let _ = ui.show();
    place_overlay(ui, saved_position(settings));
}

/// Starts a short-lived thread that measures latency; at most one at a time.
fn probe_latency(sh: &Shared) {
    if sh.ping_busy.swap(true, Ordering::AcqRel) {
        return;
    }
    let (ms, busy) = (sh.ping_ms.clone(), sh.ping_busy.clone());
    std::thread::spawn(move || {
        let v = nex_net::ping::tcp_latency_ms()
            .map(|x| x.round() as u32)
            .unwrap_or(PING_UNKNOWN);
        ms.store(v, Ordering::Relaxed);
        busy.store(false, Ordering::Release);
    });
}

pub fn run(guard: Option<TcpListener>) -> Result<(), slint::PlatformError> {
    let settings = Settings::load();
    let units = if settings.bits {
        UnitSystem::Bits
    } else {
        UnitSystem::Bytes
    };

    let mut state = State {
        source: PlatformSource::new(),
        sampler: Sampler::new(),
        readings: Vec::new(),
        history: SpeedHistory::default(),
        ema_rx: Ema::new(0.5),
        ema_tx: Ema::new(0.5),
        peak_rx: 0.0,
    };
    // Take the baseline now, so the first timer tick already has a speed to show.
    if state.source.read(&mut state.readings).is_ok() {
        state.sampler.update(Instant::now(), &state.readings);
    }

    let shared = Rc::new(Shared {
        settings: RefCell::new(settings),
        state: RefCell::new(state),
        display: RefCell::new(Display::default()),
        tray: RefCell::new(None),
        units: Cell::new(units),
        autostart: Cell::new(autostart::is_enabled()),
        tooltip: RefCell::new("Nex Speed".to_string()),
        usage: RefCell::new(UsageStore::load()),
        suppressed: Cell::new(false),
        tick: Cell::new(0),
        notified: Cell::new(None),
        idle_graph: Cell::new(None),
        fs_active: Arc::new(AtomicBool::new(false)),
        watch_enabled: Arc::new(AtomicBool::new(false)),
        force_show: Cell::new(false),
        ping_ms: Arc::new(AtomicU32::new(PING_UNKNOWN)),
        ping_busy: Arc::new(AtomicBool::new(false)),
    });

    let ui = Overlay::new()?;
    let details = Details::new()?;
    DETAILS.with(|d| *d.borrow_mut() = Some(details.as_weak()));
    details.set_speedtest_text(speedtest_hint().as_str().into());
    details.on_start_speedtest({
        let weak = details.as_weak();
        move || {
            if let Some(d) = weak.upgrade() {
                start_speedtest(&d);
            }
        }
    });
    details.on_cancel_speedtest({
        let weak = details.as_weak();
        move || {
            if let Some(d) = weak.upgrade() {
                cancel_speedtest(&d);
            }
        }
    });

    details.on_export_csv({
        let (weak, sh) = (details.as_weak(), shared.clone());
        move || {
            let Some(d) = weak.upgrade() else { return };
            let text = match sh.usage.borrow().export_csv(today_key()) {
                Ok(path) => export_message(&path),
                Err(e) => format!("Export failed: {e}"),
            };
            d.set_export_text(text.as_str().into());
        }
    });
    let conns = Connections::new()?;
    let history = History::new()?;
    let prefs = Preferences::new()?;
    apply_look(
        &shared.settings.borrow(),
        &ui,
        &details,
        &conns,
        &history,
        &prefs,
    );
    push_prefs(&prefs, &shared.settings.borrow(), &shared);
    let wins = Windows {
        ui: ui.as_weak(),
        details: details.as_weak(),
        conns: conns.as_weak(),
        history: history.as_weak(),
        prefs: prefs.as_weak(),
    };
    {
        // Settings window: every choice is applied at once and saved.
        let apply: Rc<dyn Fn(Change)> = {
            let (sh, w) = (shared.clone(), wins.clone());
            Rc::new(move |c| apply_change(&sh, &w, c))
        };
        let a = apply.clone();
        prefs.on_style_selected(move |i| a(Change::Style(i)));
        let a = apply.clone();
        prefs.on_theme_selected(move |i| a(Change::Theme(i)));
        let a = apply.clone();
        prefs.on_opacity_selected(move |i| a(Change::Opacity(i)));
        let a = apply.clone();
        prefs.on_units_selected(move |i| a(Change::Units(i)));
        let a = apply.clone();
        prefs.on_limit_selected(move |i| a(Change::Limit(i)));
        let a = apply.clone();
        prefs.on_auto_hide_selected(move |i| a(Change::AutoHide(i)));
        let a = apply;
        prefs.on_autostart_selected(move |i| a(Change::Autostart(i)));
        prefs.on_reset_position(|| dispatch(TrayCommand::ResetPosition));
        prefs.on_quit_app(|| dispatch(TrayCommand::Quit));
    }
    details.on_open_settings(|| dispatch(TrayCommand::ShowSettings));
    ui.on_open_settings(|| dispatch(TrayCommand::ShowSettings));
    details.on_open_connections({
        let weak = conns.as_weak();
        move || {
            if let Some(c) = weak.upgrade() {
                refresh_connections(&c);
                let _ = c.show();
            }
        }
    });

    details.on_open_history({
        let (weak, sh) = (history.as_weak(), shared.clone());
        move || {
            if let Some(h) = weak.upgrade() {
                refresh_history(&h, &sh, h.get_range());
                let _ = h.show();
            }
        }
    });
    history.on_range_changed({
        let (weak, sh) = (history.as_weak(), shared.clone());
        move |range| {
            if let Some(h) = weak.upgrade() {
                refresh_history(&h, &sh, range);
            }
        }
    });

    // Dragging: the .slint file reports pointer deltas in logical pixels.
    let weak = ui.as_weak();
    ui.on_move_by(move |dx, dy| {
        if let Some(ui) = weak.upgrade() {
            let window = ui.window();
            let scale = window.scale_factor();
            let pos = window.position();
            window.set_position(slint::PhysicalPosition::new(
                pos.x + (dx * scale).round() as i32,
                pos.y + (dy * scale).round() as i32,
            ));
        }
    });
    // Released: snap to a nearby screen edge, then remember where the overlay ended up.
    let (weak, sh) = (ui.as_weak(), shared.clone());
    ui.on_drag_ended(move || {
        if let Some(ui) = weak.upgrade() {
            let snapped = snap_overlay(&ui);
            let mut s = sh.settings.borrow_mut();
            match snapped {
                Some((x, y)) => {
                    s.pos_x = Some(x);
                    s.pos_y = Some(y);
                }
                None => remember_position(&ui, &mut s),
            }
            s.save();
        }
    });
    // The x button hides the overlay; the tray (or the hotkey) brings it back. Without a tray
    // icon there would be no way back, so closing quits in that case.
    let sh = shared.clone();
    ui.on_close_clicked(move || {
        let has_tray = sh.tray.borrow().is_some();
        dispatch(if has_tray {
            TrayCommand::ToggleOverlay
        } else {
            TrayCommand::Quit
        });
    });
    ui.on_open_details(|| dispatch(TrayCommand::ShowDetails));

    // Tray menu commands, executed on the UI thread.
    let (weak, sh) = (ui.as_weak(), shared.clone());
    let dweak = details.as_weak();
    let (cmd_conns, cmd_hist, cmd_prefs) = (conns.as_weak(), history.as_weak(), prefs.as_weak());
    let cmd_wins = wins.clone();
    COMMAND_HANDLER.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |cmd| {
            let Some(ui) = weak.upgrade() else { return };
            match cmd {
                TrayCommand::ShowDetails => {
                    if let Some(d) = dweak.upgrade() {
                        update_details(&d, &sh);
                        let _ = d.show();
                    }
                }
                TrayCommand::ShowSettings => {
                    if let Some(p) = cmd_prefs.upgrade() {
                        push_prefs(&p, &sh.settings.borrow(), &sh);
                        let _ = p.show();
                    }
                }
                TrayCommand::ShowHistory => {
                    if let Some(h) = cmd_hist.upgrade() {
                        refresh_history(&h, &sh, h.get_range());
                        let _ = h.show();
                    }
                }
                TrayCommand::ShowConnections => {
                    if let Some(c) = cmd_conns.upgrade() {
                        refresh_connections(&c);
                        let _ = c.show();
                    }
                }
                TrayCommand::ToggleOverlay => {
                    let mut s = sh.settings.borrow_mut();
                    if s.overlay_visible && sh.suppressed.get() {
                        // A full-screen app hides the overlay, but the user asked for it:
                        // show it until that app is gone.
                        sh.force_show.set(true);
                        sh.suppressed.set(false);
                        show_overlay(&ui, &s);
                        drop(s);
                        sh.sync_watch();
                        sh.sync_tray();
                        return;
                    }
                    sh.force_show.set(false);
                    s.overlay_visible = !s.overlay_visible;
                    if s.overlay_visible {
                        show_overlay(&ui, &s);
                    } else {
                        if !sh.suppressed.get() {
                            remember_position(&ui, &mut s);
                        }
                        sh.suppressed.set(false);
                        let _ = ui.hide();
                    }
                    s.save();
                }
                TrayCommand::ResetPosition => {
                    let mut s = sh.settings.borrow_mut();
                    s.pos_x = None;
                    s.pos_y = None;
                    if s.overlay_visible && !sh.suppressed.get() {
                        if let Some((x, y)) = place_overlay(&ui, None) {
                            s.pos_x = Some(x);
                            s.pos_y = Some(y);
                        }
                    }
                    s.save();
                }
                TrayCommand::ToggleAutoHide => {
                    let off = sh.settings.borrow().auto_hide_fullscreen;
                    apply_change(&sh, &cmd_wins, Change::AutoHide(i32::from(off)));
                }
                TrayCommand::ToggleAutostart => {
                    let off = sh.autostart.get();
                    apply_change(&sh, &cmd_wins, Change::Autostart(i32::from(off)));
                }
                TrayCommand::ToggleUnits => {
                    let bits = sh.units.get() == UnitSystem::Bytes;
                    apply_change(&sh, &cmd_wins, Change::Units(i32::from(bits)));
                }
                TrayCommand::CycleLimit => {
                    let mut s = sh.settings.borrow_mut();
                    s.monthly_limit_gb = next_limit(s.monthly_limit_gb);
                    s.save();
                }
                TrayCommand::CycleStyle => {
                    let mut s = sh.settings.borrow_mut();
                    s.style = next_style(s.style);
                    s.save();
                }
                TrayCommand::CycleTheme => {
                    let mut s = sh.settings.borrow_mut();
                    s.light = !s.light;
                    s.save();
                }
                TrayCommand::CycleOpacity => {
                    let mut s = sh.settings.borrow_mut();
                    s.opacity_pct = next_opacity(s.opacity_pct);
                    s.save();
                }
                TrayCommand::Quit => {
                    let mut s = sh.settings.borrow_mut();
                    if s.overlay_visible && !sh.suppressed.get() {
                        remember_position(&ui, &mut s);
                    }
                    s.save();
                    sh.usage.borrow_mut().flush();
                    let _ = slint::quit_event_loop();
                }
            }
            if matches!(
                cmd,
                TrayCommand::CycleStyle
                    | TrayCommand::CycleTheme
                    | TrayCommand::CycleOpacity
                    | TrayCommand::CycleLimit
            ) {
                refresh_look(&sh, &cmd_wins);
            }
            sh.sync_watch();
            sh.sync_tray();
        }));
    });

    // Show first, then restore the saved position (the window must exist to be moved).
    if shared.settings.borrow().overlay_visible {
        ui.show()?;
        place_overlay(&ui, saved_position(&shared.settings.borrow()));
    }

    // Ctrl+Alt+N shows/hides the overlay. Not available on Wayland; the tray menu still works.
    let _hotkey = match nex_platform::hotkey::register(|| dispatch(TrayCommand::ToggleOverlay)) {
        Ok(h) => Some(h),
        Err(e) => {
            applog::line(&format!("global hotkey unavailable: {e}"));
            None
        }
    };

    // Tray: if it can't be created (e.g. no StatusNotifier host on Linux) the overlay still works.
    match Tray::new(Arc::new(dispatch), &shared.view()) {
        Ok(tray) => *shared.tray.borrow_mut() = Some(tray),
        Err(e) => applog::line(&format!("tray unavailable: {e}")),
    }

    // Full-screen detection can be slow (xprop / Win32 queries), so it runs on its own thread and
    // the UI thread only reads the answer. It sleeps while nobody needs it.
    shared.sync_watch();
    {
        let (fs, enabled) = (shared.fs_active.clone(), shared.watch_enabled.clone());
        std::thread::spawn(move || loop {
            let now = enabled.load(Ordering::Relaxed) && fullscreen_app_active();
            fs.store(now, Ordering::Relaxed);
            std::thread::sleep(Duration::from_secs(2));
        });
    }

    // A second launch of the app asks this instance to show its details window.
    if let Some(listener) = guard {
        instance::serve(listener, || dispatch(TrayCommand::ShowDetails));
    }

    let dweak = details.as_weak();
    let cweak = conns.as_weak();
    let hweak = history.as_weak();
    let (weak, sh) = (ui.as_weak(), shared.clone());
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            let tick = sh.tick.get().wrapping_add(1);
            sh.tick.set(tick);

            // Full-screen app in front: hide the overlay, bring it back afterwards.
            let fullscreen = sh.fs_active.load(Ordering::Relaxed);
            if !fullscreen {
                sh.force_show.set(false);
            }
            let want_hidden = {
                let s = sh.settings.borrow();
                s.overlay_visible && s.auto_hide_fullscreen && fullscreen && !sh.force_show.get()
            };
            if want_hidden != sh.suppressed.get() {
                sh.suppressed.set(want_hidden);
                if let Some(ui) = weak.upgrade() {
                    if want_hidden {
                        remember_position(&ui, &mut sh.settings.borrow_mut());
                        let _ = ui.hide();
                    } else if sh.settings.borrow().overlay_visible {
                        show_overlay(&ui, &sh.settings.borrow());
                    }
                }
            }

            let (down, up, drx, dtx, adapter, online, peak) = {
                let mut guard = sh.state.borrow_mut();
                let st = &mut *guard;
                // A failed read means "offline": keep ticking, so the overlay shows zeros and
                // "No connection" instead of freezing on the last speeds.
                let snap = if st.source.read(&mut st.readings).is_ok() {
                    match st.sampler.update(Instant::now(), &st.readings) {
                        Some(snap) => Some(snap),
                        None => return, // clock went backwards or too soon: keep what is shown
                    }
                } else {
                    None
                };
                let (raw_rx, raw_tx, drx, dtx, adapter, online) = match &snap {
                    Some(snap) => {
                        let (adapter, online) = match snap.primary_adapter() {
                            Some(a) => (a.name.clone(), true),
                            None => ("No connection".to_string(), false),
                        };
                        (
                            snap.total_rx_bps,
                            snap.total_tx_bps,
                            snap.total_rx_delta,
                            snap.total_tx_delta,
                            adapter,
                            online,
                        )
                    }
                    None => (0.0, 0.0, 0, 0, "No connection".to_string(), false),
                };
                // The graph and the session peak use the raw speeds; only the numbers are smoothed.
                st.history.push(raw_rx, raw_tx);
                if raw_rx > st.peak_rx {
                    st.peak_rx = raw_rx;
                }
                let rx = st.ema_rx.update(raw_rx);
                let tx = st.ema_tx.update(raw_tx);
                let units = sh.units.get();
                (
                    format_speed(rx, units),
                    format_speed(tx, units),
                    drx,
                    dtx,
                    adapter,
                    online,
                    format_speed(st.peak_rx, units),
                )
            };

            // Remember the newest numbers for windows that open before the next tick.
            {
                let mut d = sh.display.borrow_mut();
                d.down.clone_from(&down);
                d.up.clone_from(&up);
                d.peak.clone_from(&peak);
                d.adapter.clone_from(&adapter);
                d.online = online;
            }

            sh.usage.borrow_mut().record(today_key(), drx, dtx);
            let usage_line = sh.usage_line();
            *sh.tooltip.borrow_mut() = format!("Down {down}   Up {up}\n{usage_line}");
            let percent = sh.month_percent();
            let level = limit_level(percent);
            let light = sh.settings.borrow().light;

            // Data-limit notification: only when a level (80 % / 100 %) is newly crossed this month.
            let month = today_key() / 100;
            let previous = match sh.notified.get() {
                Some((m, l)) if m == month => Some(l),
                Some(_) => Some(0), // new month: start over
                None => None,       // first tick: remember the level, don't notify at startup
            };
            if let (Some(prev), Some(p)) = (previous, percent) {
                if level > prev {
                    notify_limit(level, p, sh.settings.borrow().monthly_limit_gb);
                }
            }
            // Without a limit there is nothing to remember: a limit set later starts from its
            // current level instead of notifying about an old crossing.
            if percent.is_some() {
                sh.notified.set(Some((month, level)));
            } else {
                sh.notified.set(None);
            }

            // Only draw while the overlay is on screen.
            if sh.settings.borrow().overlay_visible && !sh.suppressed.get() {
                if let Some(ui) = weak.upgrade() {
                    let st = sh.state.borrow();
                    let (dv, du) = split_value_unit(&down);
                    let (uv, uu) = split_value_unit(&up);
                    ui.set_down_value(dv.into());
                    ui.set_down_unit(du.into());
                    ui.set_up_value(uv.into());
                    ui.set_up_unit(uu.into());
                    ui.set_alert(level as i32);
                    ui.set_online(online);
                    // An idle connection draws the same flat line every second: skip the redraw.
                    let flat = st
                        .history
                        .rx
                        .iter()
                        .chain(st.history.tx.iter())
                        .all(|&v| v < 1.0);
                    if !(flat && sh.idle_graph.get() == Some(light)) {
                        ui.set_graph(chart::sparkline_image(
                            &st.history.rx,
                            &st.history.tx,
                            light,
                        ));
                    }
                    sh.idle_graph.set(if flat { Some(light) } else { None });
                }
            }

            // The details window (and the latency probe) only cost anything while it is open.
            if let Some(d) = dweak.upgrade() {
                if d.window().is_visible() {
                    if tick % PING_EVERY_TICKS == 0 {
                        probe_latency(&sh);
                    }
                    update_details(&d, &sh);
                }
            }
            if tick % 10 == 0 {
                if let Some(h) = hweak.upgrade() {
                    if h.window().is_visible() {
                        refresh_history(&h, &sh, h.get_range());
                    }
                }
            }
            // The connections list is only read while its window is open.
            if tick % CONNECTIONS_EVERY_TICKS == 0 {
                if let Some(c) = cweak.upgrade() {
                    if c.window().is_visible() {
                        refresh_connections(&c);
                    }
                }
            }
            sh.refresh_tray();
        },
    );

    // `run_event_loop_until_quit` keeps running while the overlay is hidden (tray only).
    slint::run_event_loop_until_quit()?;
    drop(timer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_value_and_unit() {
        assert_eq!(split_value_unit("5.40 MB/s"), ("5.40", "MB/s"));
        assert_eq!(split_value_unit("0"), ("0", ""));
    }

    #[test]
    fn export_message_names_file_and_folder() {
        let p = Path::new("/home/u/Downloads").join("nexspeed-usage-20261117.csv");
        assert_eq!(
            export_message(&p),
            "Saved nexspeed-usage-20261117.csv to Downloads"
        );
        assert_eq!(export_message(Path::new("x.csv")), "Saved x.csv");
    }

    #[test]
    fn cancelled_speed_test_is_not_an_error_message() {
        assert_eq!(
            result_text(&Err(speedtest::CANCELLED.to_string())),
            "Speed test cancelled"
        );
        assert!(result_text(&Err("boom".to_string())).starts_with("Speed test failed"));
    }

    #[test]
    fn saved_position_ignores_garbage() {
        let s = Settings {
            pos_x: Some(50_000),
            pos_y: Some(0),
            ..Settings::default()
        };
        assert_eq!(saved_position(&s), None);
        let s = Settings {
            pos_x: Some(-30),
            pos_y: Some(40),
            ..Settings::default()
        };
        assert_eq!(saved_position(&s), Some((-30, 40)));
    }
}
