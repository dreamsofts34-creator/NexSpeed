# Nex Speed

Lightweight internet speed meter for Windows 10/11 and Linux. Rust + Slint. GPL-3.0.

See [PLAN.md](PLAN.md) for the plan and architecture.

## Status: M8 (feature complete except per-app bandwidth)

```
cargo test --workspace
cargo run -p nex-app --release            # overlay + tray icon
cargo run -p nex-app -- --cli --all       # terminal mode (debug build; release is a GUI app on Windows)
```

- Drag the overlay to move it (position is remembered). Hover it for the close button.
- **Ctrl+Alt+N** shows/hides the overlay (Windows and X11; not on Wayland, use the tray menu there).
- Tray (left click opens details, right click opens the menu): usage line, details, show/hide overlay, hide in full-screen apps, start with system, bits/bytes, monthly limit, **Style** (Graph / Minimal / Compact), **Theme** (Dark / Light), **Opacity** (100/85/70/50 %), quit.
- Details window: adapter, download number, 60 s graph, ping, today/month usage, monthly-limit status, speed test (Cloudflare, about 8 s per direction, up to about 50 MB download).
  Buttons: **History** (7 / 30 day usage bars), **Connections** (live list of active TCP connections per app), **Export CSV** (daily usage into your Downloads folder).
- Monthly limit: at 80 % the overlay border turns amber; a desktop notification appears when 80 % and 100 % are crossed.
- "Hide in full-screen apps": Windows (shell state) and Linux X11 (needs the `xprop` tool; pure Wayland is not detected).
- Settings: `%APPDATA%\NexSpeed\settings.toml` (Windows), `~/.config/nexspeed/settings.toml` (Linux). Usage: `usage.bin` next to it (`%APPDATA%\NexSpeed` / `~/.local/share/nexspeed`).
- Linux tray needs a StatusNotifier host (KDE, XFCE and others have one; stock GNOME needs the AppIndicator extension). Without one the overlay still works.
- Not included: per-app bandwidth (needs admin/ETW).

## Build and release

CI (`.github/workflows/ci.yml`) builds, runs clippy and tests on Windows and Linux. Push a tag like `v0.2.0` and `release.yml` publishes a Windows portable zip, a Linux tar.gz and a `.deb`.
Linux build packages: `libfontconfig1-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libxcb1-dev libgl1-mesa-dev pkg-config`.
Windows: `cargo build --release -p nex-app` gives `target\release\nexspeed.exe` (static CRT, no installer needed).

License: GPL-3.0-or-later, see [LICENSE](LICENSE).
