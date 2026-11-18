# Nex Speed — Plan & Architecture

Lightweight, ultra-fast, modern internet speed meter for **Windows 10/11 and Linux**. Open source. Built in Rust with a Slint UI.

---

## 1. Goals

| Goal | Target |
|---|---|
| RAM (overlay only) | < 15 MB |
| CPU (idle, overlay visible) | ~0% (< 0.3%) |
| Binary size | < 6 MB (Windows portable `.exe`; Linux single binary) |
| Startup | < 150 ms to first frame |
| Ease of use | Zero setup. Opens straight to the overlay. |
| Privacy | No telemetry, no account, no cloud. Everything local. |

**Design principle:** the default experience is one small overlay. Advanced features are one click away, and any feature that is not in use consumes zero resources.

---

## 2. Tech Decisions

| Area | Choice | Why |
|---|---|---|
| Language | Rust | Speed, safety, tiny binaries |
| UI | **Slint** | Native rendering, 10–20 MB RAM, modern animations. No WebView (rules out Tauri). |
| Renderer | Slint software renderer by default (femtovg optional) | Works everywhere, no GPU/driver issues |
| License | **GPL-3.0** (open source) | Fully compatible with Slint's GPLv3 option, no fees |
| Platforms | Windows 10/11 (x86_64), Linux (x86_64, aarch64 later) | Per-OS code isolated in `nex-platform` |
| Settings | `settings.toml` (`serde` + `toml`), path via `dirs` | `%APPDATA%\NexSpeed` / `~/.config/nexspeed` |
| History | Custom compact binary file | Minimal size and I/O |
| Tray | `tray-icon` (Windows), `ksni`/StatusNotifierItem (Linux) | Native on each OS |
| Latency check | TCP connect timing (cross-platform) | Avoids raw-ICMP privilege problems on Linux |

### Per-OS backends

| Capability | Windows | Linux |
|---|---|---|
| Live counters | IP Helper `GetIfTable2` / `GetIfEntry2` | `/proc/net/dev` (fast text parse, no deps) |
| Adapter info | IP Helper (type, status, speed) | `/sys/class/net/<if>/` (operstate, type) |
| Autostart | Registry `HKCU\...\Run` | `~/.config/autostart/nexspeed.desktop` |
| Active connections | `GetExtendedTcpTable` / `GetExtendedUdpTable` | `/proc/net/tcp{,6}` + `/proc/<pid>/fd` mapping |
| Per-app bandwidth | ETW (`ferrisetw`), needs admin | Best-effort: `/proc` + conntrack/nf-accounting or eBPF, needs root/CAP_NET_ADMIN |
| Window blur effects | Win11 Mica/Acrylic via DWM; Win10 plain translucency | Compositor-dependent; plain translucency fallback |
| Fullscreen detection | Foreground window rect vs monitor | X11: `_NET_WM_STATE_FULLSCREEN`; Wayland: best-effort |
| Global hotkey | `global-hotkey` | X11 only; Wayland has no standard protocol → tray menu fallback |
| Always-on-top | Native | X11 yes; Wayland compositor-dependent (wlr-layer-shell where available) |

---

## 3. Feature Scope

### Phase 1 — Core (MVP)
- Live download/upload speed overlay (draggable, always-on-top toggle)
- 60-second sparkline graph
- System tray icon with speed in tooltip
- Autostart with the OS
- Auto adapter detection (WiFi / Ethernet / VPN), manual switch
- Units: KB/s, MB/s, Mbps (auto or fixed)
- Dark / Light theme, opacity slider
- Today / this-month data usage counters
- Right-click menu: Dashboard, Settings, Quit

### Phase 2 — Smart
- Dashboard window: usage history charts (day / week / month)
- Data limit alert (native notifications)
- Latency monitor
- Built-in speed test (on demand only)
- Auto-hide overlay in fullscreen apps/games
- Global hotkey to show/hide overlay (where the OS allows)
- Overlay styles: Minimal, Graph, Compact bar

### Phase 3 — Pro (inside an "Advanced" tab)
- Active connections list (app, remote IP, port, state) — both OSes
- Per-app bandwidth usage — **Windows first** (ETW); Linux best-effort afterwards
- CSV export of usage history

### Deliberately skipped
Cloud sync, accounts, ads, telemetry, background DB servers, per-app blocking/limiting (needs WFP/nftables + elevated rights; high risk, revisit later).

---

## 4. Architecture

### 4.1 Workspace layout

```
nexspeed/
├─ Cargo.toml                (workspace)
├─ LICENSE                   (GPL-3.0)
├─ crates/
│  ├─ nex-core/              Engine: sampling, math, history, alerts (no UI, no OS deps)
│  ├─ nex-platform/
│  │   └─ src/
│  │       ├─ lib.rs         Traits + cfg-based re-exports
│  │       ├─ windows/       counters, autostart, tray, ETW, DWM, fullscreen
│  │       └─ linux/         /proc parsing, autostart .desktop, tray, X11/Wayland helpers
│  ├─ nex-ui/                Slint files (.slint) + Rust glue / view-models
│  └─ nex-app/               main.rs: wires everything, owns threads and lifecycle
└─ PLAN.md
```

**Dependency direction:** `nex-app → nex-ui, nex-core, nex-platform`; `nex-platform → nex-core` (traits only). `nex-core` has no OS-specific code, so it is fully unit-testable on any machine/CI.

### 4.2 Core trait boundary

```rust
// nex-core
pub trait NetSource {
    fn adapters(&self) -> Vec<Adapter>;
    fn counters(&mut self, id: &AdapterId) -> Counters; // total rx/tx bytes (u64)
}

pub trait SystemIntegration {
    fn set_autostart(&self, on: bool) -> Result<()>;
    fn is_fullscreen_app_active(&self) -> bool;
    fn notify(&self, title: &str, body: &str);
}
```

`nex-platform` provides `WindowsSource` and `LinuxSource` selected with `#[cfg(target_os = ...)]`. Everything above the traits is shared.

### 4.3 Data flow

```
 ┌──────────────┐  1s tick   ┌────────────┐  Snapshot   ┌───────────┐
 │ NetSource    │──────────▶│ Sampler    │───────────▶│ UI thread │
 │ (per-OS)     │  counters  │ (nex-core) │  (channel)  │ (Slint)   │
 └──────────────┘            └─────┬──────┘             └───────────┘
                                   │
                          ┌────────┴─────────┐
                          ▼                  ▼
                    History writer      Alert checker
                    (batched, 60s)      (limits, thresholds)
```

### 4.4 Threading model

- **Main thread:** Slint event loop only. Never blocks.
- **Sampler thread:** one thread, sleeps between ticks (no busy loops). Pushes a small `Snapshot` to the UI via `slint::Weak::upgrade_in_event_loop`.
- **On-demand workers** (spawned only when needed, stopped after): per-app monitor, speed test, latency checker, connection poller.

No async runtime unless the speed test later needs one — keeps size and RAM down.

### 4.5 Speed calculation

```
speed = (counter_now - counter_prev) / elapsed_seconds
```
- 64-bit counters; handle wraparound and adapter reset (negative delta → 0).
- Use real elapsed time (`Instant`), not an assumed 1.0 s.
- Light smoothing: EMA (α ≈ 0.5) for display; raw values kept for graph and history.
- Exclude loopback, down, and virtual adapters by default (Windows: Hyper-V/VMware; Linux: `lo`, `docker*`, `veth*`, `br-*`, `virbr*`); "All adapters" is a setting.
- Linux `/proc/net/dev`: keep the file handle open and `seek(0)` + re-read into a reused buffer each tick (no reopen, no allocation).

### 4.6 Storage

- **`settings.toml`** — adapter, units, theme, opacity, position, limits.
- **History:** fixed-size records `(timestamp_hour, rx_bytes, tx_bytes)` appended hourly in a compact binary file. ~24 bytes/hour → ~200 KB/year. Flushed every 60 s and on exit. Daily/monthly views aggregated on read.

---

## 5. Optimization Strategy

**Build profile**
```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```
Later experiments: `opt-level = "z"` comparison, PGO build. No UPX on Windows (antivirus false positives).

**Runtime**
- Zero allocations in the sampling hot path (preallocated ring buffer `[f32; 60]` for the graph).
- UI updates only when values change; overlay hidden → no rendering, sampler slows to 2–3 s.
- Dashboard closed → per-app, connection polling and charts fully stopped.
- Reusable string buffers for speed formatting.
- Lower process priority; Windows EcoQoS, Linux `nice` level, so the OS treats it as background work.
- Windows: static CRT (`+crt-static`). Linux: dynamically link only essentials; offer an AppImage for portability.
- `default-features = false` on every crate.

**Measure, don't guess:** check RAM/CPU on both OSes and run `cargo bloat` each milestone; milestone fails if §1 targets are missed.

---

## 6. UI / UX

**Overlay (default):** rounded translucent card, `↓ 12.4 MB/s` and `↑ 1.2 MB/s`, small sparkline. Drag to move, position remembered, snaps to screen edges. Windows 11: optional Mica/Acrylic. Windows 10 and Linux: clean translucent fallback.

**Right-click menu:** Dashboard · Settings · Style · Pin on top · Quit.

**Dashboard:** sidebar — *Overview*, *History*, *Apps*, *Connections*, *Tools* (speed test, latency), *Settings*. Apps/Connections collect data only while open.

**Simplicity rules**
1. First launch shows the overlay immediately with sensible defaults.
2. Every setting has a default; nothing is mandatory.
3. Max two clicks to reach any feature.
4. Advanced items live under a clearly labelled "Advanced" section.
5. If an OS can't support a feature (e.g. hotkeys on Wayland), hide it or show a short explanation, never a broken control.

---

## 7. Milestones

| # | Milestone | Deliverable |
|---|---|---|
| M1 | Workspace + `nex-core` + both `NetSource` backends | CLI prints live speed on Windows and Linux |
| M2 | Slint overlay | Draggable overlay with live speed + sparkline |
| M3 | Tray, autostart, settings | Persistent config, tray tooltip, units/theme |
| M4 | Usage history | Daily/monthly counters, history file |
| M5 | Dashboard + charts | History view, alerts, latency |
| M6 | Speed test + fullscreen auto-hide + hotkey | Phase 2 complete |
| M7 | Connections list + CSV export + per-app (Windows ETW) | Phase 3 complete |
| M8 | Packaging & release | Windows: portable `.exe` + zip/winget. Linux: AppImage, `.deb`, Flatpak. CI builds, README |

**CI:** GitHub Actions matrix (windows-latest, ubuntu-latest): build, `clippy`, tests, size check, release artifacts.

---

## 8. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Wayland limits (always-on-top, hotkeys, positioning) | Support X11 fully; on Wayland use layer-shell where available and document limits |
| Linux tray fragmentation (GNOME has no tray by default) | StatusNotifierItem + clear note about the AppIndicator extension; overlay works without tray |
| Per-app usage needs elevated rights (ETW / eBPF) | Optional feature; prompt only when the Apps tab is opened; app fully works without it |
| Windows 10 lacks Win11 visual effects | Graceful fallback to translucent style |
| Slint lacks built-in charts | Draw sparklines/charts with `Path` elements fed from Rust models |
| Antivirus false positives on Windows | No packers; sign releases when possible |
| Overlay over fullscreen games | Detect fullscreen; auto-hide by default |
| Virtual adapters inflating numbers | Filtering rules + manual override |
| Cross-platform code drift | All OS code behind traits, CI builds both OSes on every PR |

---

## 9. Dependencies (planned)

Shared: `slint`, `serde`, `toml`, `dirs`.
Windows: `windows` (only needed Win32 features), `tray-icon`, `global-hotkey`, `ferrisetw` (Phase 3).
Linux: `ksni` (tray), `x11rb` (fullscreen/always-on-top helpers), `procfs` or hand-written `/proc` parser.
Every addition must justify its size cost.

---

## 10. Open Source Setup

- **License:** GPL-3.0 (`LICENSE` file at root, SPDX headers in sources).
- **Repo basics:** README with screenshots/GIF, `CONTRIBUTING.md`, issue templates, `CHANGELOG.md`.
- **Release channels:** GitHub Releases first; winget/Scoop (Windows), Flatpak/AUR (Linux) later.

---

## 11. Open Questions

1. Overlay default position: near the taskbar clock / panel, or a screen corner?
2. Linux distro focus for first packaging: Ubuntu/Debian (`.deb`), Arch (AUR), or AppImage for all?
3. Display server priority on Linux: X11 first, or Wayland-first?

---

## 12. Research Notes (from M1)

Findings that changed the implementation:

1. **Windows filter rows duplicate counters.** `GetIfTable2` returns an extra row per NDIS filter (QoS, WFP, Native WiFi...) with identical counters to the real adapter. They carry the `FilterInterface` flag (bit 1) and are dropped; otherwise speed is counted several times.
2. **Don't filter by `HardwareInterface` alone.** VPN/tunnel adapters carry real traffic but aren't hardware. They are kept as readings and tagged `physical = false`.
3. **No VPN double counting.** Total speed = sum of *physical, up* adapters (VPN traffic already passes through the physical NIC). If no physical adapter is up, fall back to all up adapters.
4. **Linux physical detection** uses the presence of `/sys/class/net/<if>/device`.
5. **`/proc/net/dev` parsing** splits on the last colon (alias names like `eth0:0`); tx bytes is the 9th counter.
6. **Counter resets** (driver reload, adapter replug) are treated as 0 speed (`saturating_sub`), never a burst. Vanished adapters drop their baseline.
7. **Deviation from section 4.5:** Linux reopens `/proc/net/dev` each tick instead of `seek(0)`; revisit only if profiling shows it matters.
8. **Crate versions checked:** `windows` 0.62 (`GetIfTable2` returns `WIN32_ERROR`, needs features Foundation + IpHelper + Ndis).

---

## 13. Research Notes (from M2)

1. **No `Path` in the Slint UI.** Slint's software renderer has had gaps with `Path` (issue #5237), and we chose the software renderer for tiny RAM and no GPU. The graph is rendered in Rust (`nex-ui/src/chart.rs`) into a pixel buffer and shown as an `Image`. Pure CPU, ~25k pixels per second.
2. **Features:** `slint` with `default-features = false` and `backend-winit` + `renderer-software` + `software-renderer-systemfonts`. The default `backend-default` would pick Qt on Linux when installed; we explicitly avoid it.
3. **Transparent window** relies on Slint's software renderer premultiplied-alpha path (fixed upstream in 2023). If a platform ignores transparency, the only effect is square corners.
4. **No sampler thread.** One `slint::Timer` (1 s) on the UI thread does read + sample + redraw. Reading counters takes microseconds, so a thread would only add idle cost. This replaces the thread in section 4.4 for the overlay.
5. **Dragging** uses a `TouchArea` that reports pointer deltas; Rust converts to physical pixels with `scale_factor()` and calls `Window::set_position`.
6. **Close:** hover shows a small x (context menus are left for M3 after verifying support on frameless windows).
7. **CI** builds and tests on Windows and Linux, so compile errors on either OS surface immediately.

---

## 14. Research Notes (from M3)

1. **tray-icon is GTK-only on Linux** and needs a GTK event loop next to Slint's winit loop, so Linux uses `ksni` (StatusNotifierItem over D-Bus, own thread). Windows uses `tray-icon`, which only needs the Win32 loop that Slint already runs. The tooltip is unsupported on Linux, so only Windows shows live speed on hover.
2. **No polling for tray events.** Menu clicks arrive on arbitrary threads and are forwarded with `slint::invoke_from_event_loop` into a thread-local handler on the UI thread.
3. **Hiding the overlay must not quit the app.** `Window::run()` stops when the last window closes, so the app uses `show()` + `run_event_loop_until_quit()` and quits explicitly.
4. **Autostart** uses the `auto-launch` 0.5 API (current-user registry `Run` key on Windows, XDG autostart `.desktop` on Linux). Version 0.6 documents a default that touches the system registry and needs admin, so 0.5 is pinned. Caveat: an AppImage's path changes per run; autostart for AppImage needs a stable copy of the file.
5. **Settings** are TOML with a default for every field; saved via temp file + rename. Saved positions beyond +-20000 px are ignored.
6. **Tray icon is generated in code** (no asset files).
7. **Known gaps:** left-click on the Windows tray icon does nothing yet; stale saved position after a monitor change is only guarded by the +-20000 px check, not by real screen bounds.

---

## 15. Research Notes (from M4)

1. **Daily buckets instead of hourly** (deviation from section 4.6): the first views are Today/Month, daily records are 20 bytes (~7 KB/year), and week/month charts only need days. Hourly can be added later behind a format version bump (`NXU1` -> `NXU2`).
2. **Local dates come from `chrono::Local`** (clock feature only) so day rollover follows the user's timezone on both OSes. `nex-core` stays dependency-free: the app passes the `YYYYMMDD` key in.
3. **No lost bytes:** the sampler only moves its baseline when it emits a snapshot, so bytes counted between two snapshots are always accounted exactly once. Counter resets add 0.
4. **Usage counts the same adapters as the speed total** (physical, up; fallback all up), so VPN traffic is not counted twice.
5. **Writes are rare and safe:** flushed about once a minute and on quit, via temp file + rename. A corrupt or truncated file decodes to `None` and the log starts fresh.
6. **Shown in the tray** (menu line + Windows tooltip). The big dashboard view comes with the flyout window milestone.

---

## 16. Research Notes (from M5)

1. **Details window** (`details.slint`, 340x400, fixed size, system title bar) is a second `Window` in the same `.slint` compilation unit, re-exported from `overlay.slint` so the build still compiles a single entry file.
2. **It only costs anything while open:** the timer checks `Window::is_visible()` and skips the (bigger, 560x144) graph render and all property updates when the window is closed. Closing it just hides it; the app keeps running because the event loop is `run_event_loop_until_quit`.
3. **Adapter label** comes from `Snapshot::primary_adapter()`: busiest physical adapter that is up, else busiest up adapter, else "No connection".
4. **Deliberately left out for now:** Details/Speed-test buttons and the monthly limit bar from the mockup. They arrive with the speed test and data-limit alert milestone, so nothing in the UI is a dead control.
5. **Not done yet:** theme switching (details window is dark only), left-click on the tray icon to open details (needs the exact `tray-icon` click-event API to be verified).

---

## 17. Research Notes (from M6)

1. **Latency = TCP handshake time** to 1.1.1.1:443 (fallback 8.8.8.8:443), no DNS, no ICMP privileges. Measured by a short-lived thread every 3 s, only while the details window is open, at most one at a time.
2. **Speed test** uses Cloudflare's public endpoints (`GET /__down?bytes=N`, `POST /__up`, latency via `__down?bytes=0`), through `ureq` 3 (blocking, rustls, no async runtime). It only runs on click, on its own thread. Each direction is capped at 8 s: download asks for 50 MB but stops at the budget, upload sends 2 MB chunks until the budget. Upload data is xorshift noise so path compression cannot inflate it. Cost: ~1 MB extra binary size for TLS; nothing at idle.
3. **Full-screen auto-hide (Windows)** uses `SHQueryUserNotificationState` (2 = full-screen app / presentation settings, 3 = exclusive Direct3D, 4 = presentation mode). Cheap shell call once per tick. Temporarily hides the overlay; the user's own "Show overlay" choice is never changed. Linux is not detected yet.
4. **Data limit** is a cycle item in the tray (Off, 50, 100, 200, 500, 1000 GB) instead of a text field, so no extra UI widgets are needed. At 80% the overlay border turns amber and the details window shows the percentage. **Native toast notifications are intentionally not used yet**: they need another crate with different behaviour per OS and an app id on Windows; revisit after the first compile/test round.
5. **Global hotkey is not implemented**: it works differently on Windows, X11 and Wayland, and the tray already covers show/hide. Left for a later milestone.
6. **Dead-control rule kept:** every control that exists does something. The details window's speed-test button and limit line are live.

---

## 18. Research Notes (from M7, part 1)

1. **Scope of this step:** CSV export and the connections list. Per-app bandwidth (Windows ETW via `ferrisetw`) is **not done yet**: it needs admin rights and cannot be verified without a Windows machine, so it stays the next step of M7.
2. **CSV** (`nex-core/export.rs`): plain byte counts per day (`date,download_bytes,upload_bytes,total_bytes`) so spreadsheets can sum and chart them. Written to the Downloads folder (home as fallback) from a button in the details window; the result path is shown under the button, so no tray changes were needed.
3. **Connections, Linux:** `/proc/net/tcp{,6}` rows are parsed in `nex-core` (pure, tested). Socket inodes are mapped to processes by scanning `/proc/<pid>/fd` (`socket:[inode]`) and `comm`. Other users' processes are unreadable without root and show as "Unknown". Verified against the real `/proc` on the dev machine.
4. **Connections, Windows:** `GetExtendedTcpTable` (IPv4, owner-PID) + `QueryFullProcessImageNameW`. **Written without a Windows compiler: first CI run on Windows must confirm it builds.** A small `ErrCode` trait hides whether the `windows` crate returns `u32` or `WIN32_ERROR`. IPv6 uses the second (`AF_INET6`) table, best effort.
5. **Cost:** the list is only read while the Connections window is open (every 3 s); nothing runs otherwise.
6. **Window sizes:** details window grew from 470 to 540 px for the two new buttons.

---

## 19. Research Notes (M7 part 2 and M8)

1. **Not compiled by the author of this step.** Everything below was written without access to crates.io, so the first CI run is the real compile test. Pure logic was unit-tested standalone (core: 32 tests, chart renderer, X11 parsers).
2. **History window** (`history.slint`): stacked bars rendered in Rust (`chart::render_bars`), 7 or 30 days, refreshed every 10 s only while open. Date math is pure (`nex_core::prefs::last_days`, leap years tested).
3. **Look:** a Slint global `Colors` holds both palettes. Each window tree has its own global instance, so `apply_look` sets it on every window. Overlay opacity only affects the overlay background. Overlay styles change the window height by binding (Graph 84, Minimal 48, Compact 30 px). If a platform ignores that resize, the fallback is to set the size from Rust.
4. **Notifications** use `notify-rust` (Windows toast, Linux D-Bus). A level (80 % / 100 %) notifies once per month; the first tick after start only records the level, so restarting never re-notifies. Windows toasts without an installed app id may show under PowerShell's name.
5. **Hotkey** uses `global-hotkey` 0.6, fixed to Ctrl+Alt+N, created on the main thread. A failure (Wayland, key already taken) is logged and ignored.
6. **Linux full-screen** shells out to `xprop` (2 short runs, cached 3 s). No new crate. Missing `xprop` or Wayland means "not full-screen".
7. **Tray left-click** (Windows) opens details via `TrayIconEvent`; the menu now opens on right click only.
8. **Packaging:** static CRT on Windows (`.cargo/config.toml`), `cargo-deb` metadata in `nex-app`, release workflow on `v*` tags. AppImage and Flatpak are not done.
9. **Still open:** per-app bandwidth (ETW), saved-position validation against real monitor bounds, edge snapping, hourly history, Windows exe icon/version resource.

---

## 20. App icon

1. **Source:** `crates/nex-app/assets/icons/nexspeed-1024.png` (master, transparent). White download arrow, gold upload arrow and speed gauge on a blue gradient rounded badge with a light rim (first version had a near-black background, replaced because it looked too heavy). The supplied picture had a painted checkerboard instead of real transparency, so the badge was cut out and given a clean rounded-rectangle alpha.
2. **Generated files:** `nexspeed.ico` (16-256 px, Windows exe), PNGs 48/128/256/512/1024 (Linux hicolor icons, `.deb`), `nex-platform/assets/tray-{32,64}.rgba` (raw tray icon, no decoder crate), `nex-ui/ui/icon.png` (window icon).
3. **Wiring:** `nex-app/build.rs` embeds the `.ico` with `winres` (Windows only, failure is only a warning); the tray uses the embedded raw RGBA; Slint windows use `icon: @image-url`, with the build configured to embed pixels for the software renderer; `.deb` installs the icons and the `.desktop` file uses `Icon=nexspeed`.
4. **Unverified until CI:** `winres` (needs the Windows SDK resource compiler, present on GitHub's `windows-latest`) and the Slint `EmbedForSoftwareRenderer` build setting.
5. **Small sizes:** at 16 px the gauge becomes a blur but the two arrows stay readable. A simplified 16 px variant could be added later.

---

## 21. Polish and fixes pass

Closes the "still open" items of section 19 where they could be done without a Windows machine, and gives every window the same look.

1. **Monitor-aware placement.** `nex-core/geometry.rs` (pure, tested): `restore_position` keeps a saved position when its centre is still on a monitor (nudged fully inside) and otherwise uses the top-right corner of the primary monitor; `snap_position` snaps to edges within 20 px and keeps an 8 px margin. Monitor data comes from `nex-platform/screen.rs`: Windows `EnumDisplayMonitors` + `GetMonitorInfoW` (work area, so the taskbar is excluded), Linux `xrandr --current` cut to `_NET_WORKAREA`. Unknown layout (Wayland without XWayland, missing tools) means "do nothing", as before. The `windows` crate gets the `Win32_Graphics_Gdi` feature.
2. **UI.** `widgets.slint` holds `NxButton`, `NxSegment`, `NxCard`, `NxStat`, `NxProgress`; all windows follow the theme and use layouts instead of hard-coded coordinates. `Colors` gained `danger`, `hover`, `pressed` (and primary-button variants). The chart palette in `chart.rs` still matches `down` / `up`.
3. **Behaviour.** x hides to the tray (quits only without a tray); double-click opens details; opening a window fills it at once from `Shared::display`; the speed test can be cancelled and uploads at most 50 MB; limit level 2 is a red border.
4. **Robustness.** Settings are sanitised on load and a broken file is kept as `settings.toml.bad`; Windows tray updates are diffed (`force_update` after a click re-syncs check marks).
5. **Not done (needs hardware or a Windows machine):** per-app bandwidth (ETW), hourly history, AppImage / Flatpak. **Not compiled by the author of this step**: the first CI run on Windows and Linux is the compile test (pure logic has unit tests: core now 47).

