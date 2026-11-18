# Changelog

## 0.2.0

### Audit report round (L1–L11, P1–P4)
- **Safety:** `usage.bin` is written atomically (temp file, fsync, `.bak`), a corrupt file is kept as `.corrupt`
  instead of being overwritten; only one instance runs (a second launch opens the details window); panics are
  logged to `nexspeed.log` (rotated at 256 KB); panic strategy is unwind so one failed thread cannot kill the app.
- **Correctness:** IPv4-mapped IPv6 connections are classified correctly; speed values round up across unit
  boundaries (no "1024 KB/s"); duplicate dates in usage data are summed; a failed adapter read shows
  "No connection" instead of freezing; the hotkey shows the overlay even while a full-screen app is in front;
  the graph uses raw speeds and the label says "Session peak"; limit notifications no longer fire for old crossings
  when a limit is set later.
- **Speed test:** parallel streams with warm-up skipped, partial results kept when one direction fails (shown as
  "—"), data use capped at 150 MB.
- **Performance:** full-screen detection and the connections scan run on worker threads (no UI stalls); the
  connections list updates in place.
- **UX:** new Settings window (tray menu, details window, or right-click on the overlay) so the app is fully usable
  without a tray; light-theme contrast fixed to WCAG AA; graph caption with max speed; x button on every style;
  `--help` / `--version`; Linux tray left click opens details; data stored in the local app-data folder.
- **CI/Release:** Linux built on Ubuntu 22.04, stale runs cancelled, tag must match `Cargo.toml` version,
  SHA-256 checksums published with releases.

### UI / UX
- Redesigned details, history and connections windows on shared widgets (`widgets.slint`): hover and pressed
  states, cards, stat tiles, segmented 7 / 30 day switch, zebra-striped connection list with an empty state.
- Details window: monthly-limit progress bar (amber at 80 %, red at 100 %), live data immediately when it opens,
  one button that starts and cancels the speed test, data-use hint, shorter export confirmation.
- Overlay: numbers in fixed-width boxes (no jumping units), dimmed numbers when offline, red border when the
  limit is reached, double-click opens the details, x button has hover feedback.
- Tray: "Usage history", "Active connections" and "Reset overlay position" entries.

### Fixes and improvements
- Overlay position is validated against the real monitors (Windows work areas, Linux xrandr + `_NET_WORKAREA`):
  a saved position on an unplugged monitor goes back to the top-right corner of the primary monitor.
- Overlay snaps to screen edges (8 px margin) when released close to them; panels and the taskbar are respected.
- The x button hides the overlay to the tray instead of quitting (it quits only when there is no tray icon).
- Speed test can be cancelled; upload is capped at 50 MB (test uses at most about 100 MB in total).
- Settings: out-of-range values (opacity, style) are corrected; an unreadable `settings.toml` is moved to
  `settings.toml.bad` instead of being silently overwritten.
- Data-limit notifications carry the app name and icon.
- Usage summary can no longer overflow.
- Windows exe: file version resource (internal name, original file name, copyright).

### Performance
- The overlay graph is not redrawn while the connection is idle.
- The Windows tray only touches the menu and tooltip when something changed.
- App icon (tray, `.exe`, window title bars, Linux `.desktop`/`.deb`).
- M7: usage CSV export, active connections list (Windows + Linux).
- Usage history window (7 / 30 day bars), light/dark theme, overlay opacity, three overlay styles.
- Data-limit notifications at 80 % and 100 %, global hotkey Ctrl+Alt+N, Linux (X11) full-screen detection,
  tray left-click opens details.
- Packaging: CI with clippy, release workflow (Windows zip, Linux tar.gz and .deb), GPL-3.0 `LICENSE`.

## 0.1.0
- Overlay, tray, details window, usage tracking, latency, speed test, monthly limit warning.
