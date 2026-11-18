# Nex Speed

Lightweight internet speed meter for Windows 10/11 and Linux. Rust + Slint. GPL-3.0.

See [PLAN.md](PLAN.md) for the plan and architecture.

## Status: M8 (feature complete except per-app bandwidth)

```
cargo test --workspace
cargo run -p nex-app --release            # overlay + tray icon
cargo run -p nex-app -- --cli --all       # terminal mode (debug build; release is a GUI app on Windows)
```

- Drag the overlay to move it: it snaps to screen edges and its position is remembered (a position on a monitor that is gone resets to the top-right corner). Double-click opens the details. The x button (on hover) hides the overlay to the tray.
- **Ctrl+Alt+N** shows/hides the overlay (Windows and X11; not on Wayland, use the tray menu there).
- Tray (left click opens details, right click opens the menu): usage line, details, usage history, active connections, show/hide overlay, hide in full-screen apps, start with system, bits/bytes, monthly limit, **Style** (Graph / Minimal / Compact), **Theme** (Dark / Light), **Opacity** (100/85/70/50 %), reset overlay position, quit.
- Details window: adapter, download number, 60 s graph, ping, today/month usage, monthly-limit status, speed test (Cloudflare, about 8 s per direction, at most about 100 MB of data in total, cancellable).
  Buttons: **History** (7 / 30 day usage bars), **Connections** (live list of active TCP connections per app), **Export CSV** (daily usage into your Downloads folder).
- Monthly limit: at 80 % the overlay border turns amber, at 100 % red; a desktop notification appears when 80 % and 100 % are crossed. The details window shows a progress bar.
- "Hide in full-screen apps": Windows (shell state) and Linux X11 (needs the `xprop` tool; pure Wayland is not detected).
- Settings window: tray menu → Settings, the Settings button in the details window, or right-click the overlay. Works without a tray icon.
- Files: `settings.toml` in `%APPDATA%\NexSpeed` (Windows) / `~/.config/nexspeed` (Linux). Usage (`usage.bin`, with `.bak`) and `nexspeed.log` live in the local app-data folder (`%LOCALAPPDATA%\NexSpeed` / `~/.local/share/nexspeed`); older data is migrated automatically.
- Only one instance runs; starting the app again opens the details window of the running one.
- Usage counts all traffic on your adapters, including LAN traffic. The speed test uses at most about 150 MB per run.
- Wayland: overlay positioning, the global hotkey and full-screen detection are limited by the compositor; use the tray menu or the Settings window.
- `nexspeed --help` / `--version` print usage and version.
- Linux: `xprop` (full-screen detection) and `xrandr` (monitor layout for edge snapping) are optional; without them those two features are simply off.
- Linux tray needs a StatusNotifier host (KDE, XFCE and others have one; stock GNOME needs the AppIndicator extension). Without one the overlay still works.
- Not included: per-app bandwidth (needs admin/ETW).

## Build and release

CI (`.github/workflows/ci.yml`) builds, runs clippy and tests on Windows and Linux. Push a tag like `v0.2.0` and `release.yml` publishes a Windows portable zip, a Linux tar.gz and a `.deb`.
Linux build packages: `libfontconfig1-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libxcb1-dev libgl1-mesa-dev pkg-config`.
Windows: `cargo build --release -p nex-app` gives `target\release\nexspeed.exe` (static CRT, no installer needed).

License: GPL-3.0-or-later, see [LICENSE](LICENSE).
