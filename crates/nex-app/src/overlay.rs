//! The floating overlay, tray and details window. Everything UI-related runs on the UI thread
//! from one 1-second timer (reading the counters takes microseconds), so there are no
//! permanent extra threads. Short-lived workers (latency probe, speed test) and tray clicks
//! report back with `invoke_from_event_loop`, so nothing polls.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nex_core::{
    format_bytes, format_speed, last_days, limit_level, limit_percent, next_opacity, next_style,
    style_name, AdapterReading, Ema, NetSource, Sampler, SpeedHistory, UnitSystem,
};
use nex_net::speedtest::{self, Phase, Progress, SpeedTestResult};
use nex_platform::{autostart, connections, fullscreen_app_active, PlatformSource, Tray, TrayCommand, TrayView};
use nex_ui::{chart, ConnRow, Connections, Details, History, Overlay, Colors};
use slint::ComponentHandle;

use crate::settings::{next_limit, Settings};
use crate::usage_store::{today_key, UsageStore};

/// Warn (amber) from this share of the monthly limit.
const WARN_PERCENT: f64 = 80.0;
/// Connections list refresh interval, in ticks, only while its window is open.
const CONNECTIONS_EVERY_TICKS: u32 = 3;
/// Latency probe interval, in 1-second ticks, only while the details window is open.
const PING_EVERY_TICKS: u32 = 3;
const PING_UNKNOWN: u32 = u32::MAX;

struct State {
    source: PlatformSource,
    sampler: Sampler,
    readings: Vec<AdapterReading>,
    history: SpeedHistory,
    ema_rx: Ema,
    ema_tx: Ema,
    peak_rx: f64,
}

struct Shared {
    settings: RefCell<Settings>,
    state: RefCell<State>,
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

    fn refresh_tray(&self) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.update(&self.view());
        }
    }
}

/// Tray-menu command handler installed by the overlay.
type CommandFn = Box<dyn Fn(TrayCommand)>;

thread_local! {
    /// Live on the UI thread; background threads reach them through `dispatch` / `with_details`.
    static COMMAND_HANDLER: RefCell<Option<CommandFn>> = const { RefCell::new(None) };
    static DETAILS: RefCell<Option<slint::Weak<Details>>> = const { RefCell::new(None) };
}

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
        Ok(r) => format!("↓ {:.1} Mbps    ↑ {:.1} Mbps    ·    {:.0} ms", r.down_mbps, r.up_mbps, r.ping_ms),
        Err(e) => {
            let short: String = e.chars().take(48).collect();
            format!("Speed test failed: {short}")
        }
    }
}

fn start_speedtest(details: &Details) {
    details.set_speedtest_running(true);
    details.set_speedtest_text("Starting…".into());
    std::thread::spawn(|| {
        let cancel = AtomicBool::new(false);
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
        });
    });
}

/// Fills the connections window: active, non-loopback TCP connections grouped by application.
fn refresh_connections(win: &Connections) {
    match connections::list() {
        Ok(all) => {
            let mut rows: Vec<(String, String)> = all
                .iter()
                .filter(|c| c.is_active() && !c.is_loopback())
                .map(|c| {
                    let name = if c.process.is_empty() { "Unknown".to_string() } else { c.process.clone() };
                    (name, c.remote.to_string())
                })
                .collect();
            rows.sort();
            let hidden = all.iter().any(|c| c.is_active() && !c.is_loopback() && c.process.is_empty());
            win.set_summary(format!("{} active connections", rows.len()).as_str().into());
            let note = if hidden { "Some applications are hidden (run as administrator / root to see all)" } else { "" };
            win.set_note(note.into());
            let model: Vec<ConnRow> = rows
                .into_iter()
                .map(|(process, remote)| ConnRow { process: process.as_str().into(), remote: remote.as_str().into() })
                .collect();
            win.set_rows(slint::ModelRc::new(slint::VecModel::from(model)));
        }
        Err(e) => {
            win.set_summary("Could not read connections".into());
            win.set_note(e.to_string().as_str().into());
        }
    }
}

/// Pushes theme, opacity and style from the settings into every window.
fn apply_look(s: &Settings, ui: &Overlay, d: &Details, c: &Connections, h: &History) {
    ui.set_style(s.style as i32);
    ui.global::<Colors>().set_opacity(s.opacity_pct as f32 / 100.0);
    ui.global::<Colors>().set_light(s.light);
    d.global::<Colors>().set_light(s.light);
    c.global::<Colors>().set_light(s.light);
    h.global::<Colors>().set_light(s.light);
}

fn iso(date: u32) -> String {
    format!("{:04}-{:02}-{:02}", date / 10_000, date / 100 % 100, date % 100)
}

/// Fills the history window with the last 7 or 30 days of usage.
fn refresh_history(win: &History, sh: &Shared, range: i32) {
    let n = if range >= 30 { 30 } else { 7 };
    let today = today_key();
    let days = sh.usage.borrow().series(today, n);
    let dates = last_days(today, n);
    let (rx, tx) = days.iter().fold((0u64, 0u64), |(a, b), &(r, t)| (a.saturating_add(r), b.saturating_add(t)));
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
        Some((i, d)) => format!("Busiest day: {} ({})", iso(dates[i]), format_bytes(d.0.saturating_add(d.1))),
        None => "No data yet for this period".to_string(),
    };
    win.set_peak_text(peak.as_str().into());
}

/// Native notification when the monthly limit crosses 80 % / 100 %. Short-lived thread, ignored on failure.
fn notify_limit(level: u8, percent: f64, limit_gb: u32) {
    let body = if level >= 2 {
        format!("You have reached your {limit_gb} GB monthly limit.")
    } else {
        format!("You have used {percent:.0}% of your {limit_gb} GB monthly limit.")
    };
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new().summary("Nex Speed").body(&body).show();
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

/// Ignore wildly off positions (monitor unplugged, corrupt file).
fn saved_position(s: &Settings) -> Option<slint::PhysicalPosition> {
    match (s.pos_x, s.pos_y) {
        (Some(x), Some(y)) if x.abs() < 20_000 && y.abs() < 20_000 => {
            Some(slint::PhysicalPosition::new(x, y))
        }
        _ => None,
    }
}

fn show_overlay(ui: &Overlay, settings: &Settings) {
    let _ = ui.show();
    if let Some(pos) = saved_position(settings) {
        ui.window().set_position(pos);
    }
}

/// Starts a short-lived thread that measures latency; at most one at a time.
fn probe_latency(sh: &Shared) {
    if sh.ping_busy.swap(true, Ordering::AcqRel) {
        return;
    }
    let (ms, busy) = (sh.ping_ms.clone(), sh.ping_busy.clone());
    std::thread::spawn(move || {
        let v = nex_net::ping::tcp_latency_ms().map(|x| x.round() as u32).unwrap_or(PING_UNKNOWN);
        ms.store(v, Ordering::Relaxed);
        busy.store(false, Ordering::Release);
    });
}

pub fn run() -> Result<(), slint::PlatformError> {
    let settings = Settings::load();
    let units = if settings.bits { UnitSystem::Bits } else { UnitSystem::Bytes };

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
        tray: RefCell::new(None),
        units: Cell::new(units),
        autostart: Cell::new(autostart::is_enabled()),
        tooltip: RefCell::new("Nex Speed".to_string()),
        usage: RefCell::new(UsageStore::load()),
        suppressed: Cell::new(false),
        tick: Cell::new(0),
        notified: Cell::new(None),
        ping_ms: Arc::new(AtomicU32::new(PING_UNKNOWN)),
        ping_busy: Arc::new(AtomicBool::new(false)),
    });

    let ui = Overlay::new()?;
    let details = Details::new()?;
    DETAILS.with(|d| *d.borrow_mut() = Some(details.as_weak()));
    details.on_start_speedtest({
        let weak = details.as_weak();
        move || {
            if let Some(d) = weak.upgrade() {
                start_speedtest(&d);
            }
        }
    });

    details.on_export_csv({
        let (weak, sh) = (details.as_weak(), shared.clone());
        move || {
            let Some(d) = weak.upgrade() else { return };
            let text = match sh.usage.borrow().export_csv(today_key()) {
                Ok(path) => format!("Saved {}", path.display()),
                Err(e) => format!("Export failed: {e}"),
            };
            d.set_export_text(text.as_str().into());
        }
    });
    let conns = Connections::new()?;
    let history = History::new()?;
    apply_look(&shared.settings.borrow(), &ui, &details, &conns, &history);
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
    let (weak, sh) = (ui.as_weak(), shared.clone());
    ui.on_drag_ended(move || {
        if let Some(ui) = weak.upgrade() {
            let mut s = sh.settings.borrow_mut();
            remember_position(&ui, &mut s);
            s.save();
        }
    });
    ui.on_quit(|| dispatch(TrayCommand::Quit));

    // Tray menu commands, executed on the UI thread.
    let (weak, sh) = (ui.as_weak(), shared.clone());
    let dweak = details.as_weak();
    let (cmd_conns, cmd_hist) = (conns.as_weak(), history.as_weak());
    COMMAND_HANDLER.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |cmd| {
            let Some(ui) = weak.upgrade() else { return };
            match cmd {
                TrayCommand::ShowDetails => {
                    if let Some(d) = dweak.upgrade() {
                        let _ = d.show();
                    }
                }
                TrayCommand::ToggleOverlay => {
                    let mut s = sh.settings.borrow_mut();
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
                TrayCommand::ToggleAutoHide => {
                    let mut s = sh.settings.borrow_mut();
                    s.auto_hide_fullscreen = !s.auto_hide_fullscreen;
                    if !s.auto_hide_fullscreen && sh.suppressed.get() {
                        sh.suppressed.set(false);
                        if s.overlay_visible {
                            show_overlay(&ui, &s);
                        }
                    }
                    s.save();
                }
                TrayCommand::ToggleAutostart => {
                    let want = !sh.autostart.get();
                    if autostart::set_enabled(want).is_ok() {
                        sh.autostart.set(want);
                    }
                }
                TrayCommand::ToggleUnits => {
                    let bits = sh.units.get() == UnitSystem::Bytes;
                    sh.units.set(if bits { UnitSystem::Bits } else { UnitSystem::Bytes });
                    let mut s = sh.settings.borrow_mut();
                    s.bits = bits;
                    s.save();
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
            if matches!(cmd, TrayCommand::CycleStyle | TrayCommand::CycleTheme | TrayCommand::CycleOpacity) {
                if let (Some(d), Some(c), Some(h)) = (dweak.upgrade(), cmd_conns.upgrade(), cmd_hist.upgrade()) {
                    apply_look(&sh.settings.borrow(), &ui, &d, &c, &h);
                    if h.window().is_visible() {
                        refresh_history(&h, &sh, h.get_range());
                    }
                }
            }
            sh.refresh_tray();
        }));
    });

    // Show first, then restore the saved position (the window must exist to be moved).
    if shared.settings.borrow().overlay_visible {
        ui.show()?;
        if let Some(pos) = saved_position(&shared.settings.borrow()) {
            ui.window().set_position(pos);
        }
    }

    // Ctrl+Alt+N shows/hides the overlay. Not available on Wayland; the tray menu still works.
    let _hotkey = match nex_platform::hotkey::register(|| dispatch(TrayCommand::ToggleOverlay)) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("global hotkey unavailable: {e}");
            None
        }
    };

    // Tray: if it can't be created (e.g. no StatusNotifier host on Linux) the overlay still works.
    match Tray::new(Arc::new(dispatch), &shared.view()) {
        Ok(tray) => *shared.tray.borrow_mut() = Some(tray),
        Err(e) => eprintln!("tray unavailable: {e}"),
    }

    let dweak = details.as_weak();
    let cweak = conns.as_weak();
    let hweak = history.as_weak();
    let (weak, sh) = (ui.as_weak(), shared.clone());
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
        let tick = sh.tick.get().wrapping_add(1);
        sh.tick.set(tick);

        // Full-screen app in front: hide the overlay, bring it back afterwards.
        let want_hidden = {
            let s = sh.settings.borrow();
            s.overlay_visible && s.auto_hide_fullscreen && fullscreen_app_active()
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
            if st.source.read(&mut st.readings).is_err() {
                return;
            }
            let Some(snap) = st.sampler.update(Instant::now(), &st.readings) else { return };
            let (drx, dtx) = (snap.total_rx_delta, snap.total_tx_delta);
            let rx = st.ema_rx.update(snap.total_rx_bps);
            let tx = st.ema_tx.update(snap.total_tx_bps);
            st.history.push(rx, tx);
            if rx > st.peak_rx {
                st.peak_rx = rx;
            }
            let units = sh.units.get();
            let (adapter, online) = match snap.primary_adapter() {
                Some(a) => (a.name.clone(), true),
                None => ("No connection".to_string(), false),
            };
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

        sh.usage.borrow_mut().record(today_key(), drx, dtx);
        let usage_line = sh.usage_line();
        *sh.tooltip.borrow_mut() = format!("Down {down}   Up {up}\n{usage_line}");
        let percent = sh.month_percent();
        let warn = percent.is_some_and(|p| p >= WARN_PERCENT);
        let light = sh.settings.borrow().light;

        // Data-limit notification: only when a level (80 % / 100 %) is newly crossed this month.
        let level = limit_level(percent);
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
        sh.notified.set(Some((month, level)));

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
                ui.set_warn(warn);
                ui.set_graph(chart::sparkline_image(&st.history.rx, &st.history.tx, light));
            }
        }

        // The details window (and the latency probe) only cost anything while it is open.
        if let Some(d) = dweak.upgrade() {
            if d.window().is_visible() {
                if tick % PING_EVERY_TICKS == 0 {
                    probe_latency(&sh);
                }
                let st = sh.state.borrow();
                let (dv, du) = split_value_unit(&down);
                d.set_adapter_name(adapter.as_str().into());
                d.set_online(online);
                d.set_down_value(dv.into());
                d.set_down_unit(du.into());
                d.set_up_text(up.as_str().into());
                d.set_peak_text(peak.as_str().into());
                d.set_graph(chart::sparkline_image_sized(560, 144, &st.history.rx, &st.history.tx, light));
                let usage = sh.usage.borrow();
                let today = today_key();
                d.set_today_text(format_bytes(usage.today_bytes(today)).as_str().into());
                d.set_month_text(format_bytes(usage.month_bytes(today)).as_str().into());
                let ping = sh.ping_ms.load(Ordering::Relaxed);
                d.set_ping_text(if ping == PING_UNKNOWN { "—".into() } else { format!("{ping} ms").as_str().into() });
                let limit_gb = sh.settings.borrow().monthly_limit_gb;
                d.set_limit_warn(warn);
                d.set_limit_text(match percent {
                    Some(p) => format!("{p:.0}% of your {limit_gb} GB monthly limit").as_str().into(),
                    None => "No monthly limit (set one in the tray menu)".into(),
                });
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
    });

    // `run_event_loop_until_quit` keeps running while the overlay is hidden (tray only).
    slint::run_event_loop_until_quit()?;
    drop(timer);
    Ok(())
}
