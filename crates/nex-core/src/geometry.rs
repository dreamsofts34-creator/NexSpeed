//! Screen geometry helpers (pure, unit-tested): keep the overlay on a real monitor, snap it to
//! screen edges and pick a sensible default position. All values are physical pixels.

/// A rectangle: top-left corner plus size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.w)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h)
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// The overlapping part of two rectangles, or `None` when they do not overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right > x && bottom > y {
            Some(Rect::new(x, y, right - x, bottom - y))
        } else {
            None
        }
    }
}

/// Squared distance from a point to a rectangle (0 when inside).
fn distance_sq(r: &Rect, px: i32, py: i32) -> i64 {
    let (px, py) = (px as i64, py as i64);
    let dx = if px < r.x as i64 {
        r.x as i64 - px
    } else if px >= r.right() as i64 {
        px - r.right() as i64 + 1
    } else {
        0
    };
    let dy = if py < r.y as i64 {
        r.y as i64 - py
    } else if py >= r.bottom() as i64 {
        py - r.bottom() as i64 + 1
    } else {
        0
    };
    dx * dx + dy * dy
}

fn centre(pos: (i32, i32), size: (i32, i32)) -> (i32, i32) {
    (pos.0.saturating_add(size.0 / 2), pos.1.saturating_add(size.1 / 2))
}

/// The area holding the window's centre, otherwise the nearest one.
fn area_for(pos: (i32, i32), size: (i32, i32), areas: &[Rect]) -> Option<&Rect> {
    let (cx, cy) = centre(pos, size);
    areas
        .iter()
        .find(|r| r.contains(cx, cy))
        .or_else(|| areas.iter().min_by_key(|r| distance_sq(r, cx, cy)))
}

/// True when the window's centre lies on one of the areas (i.e. the user can still see it).
pub fn is_on_screen(pos: (i32, i32), size: (i32, i32), areas: &[Rect]) -> bool {
    let (cx, cy) = centre(pos, size);
    areas.iter().any(|r| r.contains(cx, cy))
}

/// Moves the window just enough to lie completely inside `area`.
fn clamp_into(pos: (i32, i32), size: (i32, i32), area: &Rect) -> (i32, i32) {
    let max_x = area.right().saturating_sub(size.0).max(area.x);
    let max_y = area.bottom().saturating_sub(size.1).max(area.y);
    (pos.0.clamp(area.x, max_x), pos.1.clamp(area.y, max_y))
}

/// Top-right corner of `area`, `margin` pixels from the edges.
pub fn default_position(size: (i32, i32), area: &Rect, margin: i32) -> (i32, i32) {
    let x = area.right().saturating_sub(size.0).saturating_sub(margin).max(area.x);
    (x, area.y.saturating_add(margin))
}

/// Position to use when showing the overlay.
/// * `saved` still on a screen -> kept (nudged fully inside its monitor).
/// * `saved` on a monitor that is gone, or none saved -> top-right of the first area
///   (callers pass the primary monitor first).
/// * No screen information at all (`areas` empty) -> `saved` unchanged, `None` stays `None`.
pub fn restore_position(
    saved: Option<(i32, i32)>,
    size: (i32, i32),
    areas: &[Rect],
    margin: i32,
) -> Option<(i32, i32)> {
    let Some(first) = areas.first() else { return saved };
    match saved {
        Some(p) if is_on_screen(p, size, areas) => {
            let area = area_for(p, size, areas).unwrap_or(first);
            Some(clamp_into(p, size, area))
        }
        _ => Some(default_position(size, first, margin)),
    }
}

/// Snaps the window to the edges of its monitor when it is within `threshold` pixels of them,
/// leaving `margin` pixels free. Each axis is handled on its own.
pub fn snap_position(
    pos: (i32, i32),
    size: (i32, i32),
    areas: &[Rect],
    threshold: i32,
    margin: i32,
) -> (i32, i32) {
    let Some(area) = area_for(pos, size, areas) else { return pos };
    let axis = |p: i32, len: i32, lo: i32, hi: i32| -> i32 {
        let (p, len, lo, hi) = (p as i64, len as i64, lo as i64 + margin as i64, hi as i64 - margin as i64);
        let t = threshold as i64;
        let snapped = if (p - lo).abs() <= t {
            lo
        } else if (p + len - hi).abs() <= t {
            hi - len
        } else {
            p
        };
        snapped as i32
    };
    (axis(pos.0, size.0, area.x, area.right()), axis(pos.1, size.1, area.y, area.bottom()))
}

/// Parses `xrandr --current` output into the geometry of every active monitor, primary first.
/// Lines look like `eDP-1 connected primary 1920x1080+0+0 (normal ...) 344mm x 193mm`;
/// negative offsets are printed as `+-1920`. Disconnected or switched-off outputs are skipped.
pub fn parse_xrandr(text: &str) -> Vec<Rect> {
    let mut found: Vec<(Rect, bool)> = Vec::new();
    for line in text.lines() {
        let mut tokens = line.split_whitespace();
        let (Some(_name), Some("connected")) = (tokens.next(), tokens.next()) else { continue };
        let mut primary = false;
        for token in tokens {
            if token == "primary" {
                primary = true;
            } else if let Some(rect) = parse_geometry(token) {
                found.push((rect, primary));
                break;
            }
        }
    }
    found.sort_by_key(|&(_, primary)| !primary);
    found.into_iter().map(|(r, _)| r).collect()
}

/// `1920x1080+0+0` -> rectangle. Offsets may be negative (`+-1920`).
fn parse_geometry(token: &str) -> Option<Rect> {
    let (w, rest) = token.split_once('x')?;
    let mut parts = rest.split('+');
    let h = parts.next()?;
    let x = parts.next()?;
    let y = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let (w, h): (i32, i32) = (w.parse().ok()?, h.parse().ok()?);
    if w <= 0 || h <= 0 {
        return None;
    }
    Some(Rect::new(x.parse().ok()?, y.parse().ok()?, w, h))
}

/// `_NET_WORKAREA(CARDINAL) = 0, 0, 1920, 1040, 0, 0, 1920, 1040` -> the first desktop's `0,0,1920,1040`.
pub fn parse_net_workarea(text: &str) -> Option<Rect> {
    let numbers = text.split_once('=')?.1;
    let mut it = numbers.split(',').map(|n| n.trim().parse::<i32>());
    let (x, y, w, h) = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    if w > 0 && h > 0 {
        Some(Rect::new(x, y, w, h))
    } else {
        None
    }
}

/// Removes panel/dock space from monitors: each monitor is cut to the desktop work area.
/// A monitor that would end up empty keeps its full size.
pub fn apply_workarea(monitors: &[Rect], work: Option<Rect>) -> Vec<Rect> {
    monitors
        .iter()
        .map(|m| work.as_ref().and_then(|w| m.intersect(w)).unwrap_or(*m))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);
    const SIZE: (i32, i32) = (232, 84);

    #[test]
    fn rect_basics() {
        assert!(SCREEN.contains(0, 0));
        assert!(!SCREEN.contains(1920, 0));
        assert_eq!(SCREEN.intersect(&Rect::new(1900, 1000, 100, 100)), Some(Rect::new(1900, 1000, 20, 80)));
        assert_eq!(SCREEN.intersect(&Rect::new(1920, 0, 10, 10)), None);
    }

    #[test]
    fn default_is_top_right_with_margin() {
        assert_eq!(default_position(SIZE, &SCREEN, 12), (1920 - 232 - 12, 12));
        // Window wider than the screen never goes left of the screen.
        assert_eq!(default_position((3000, 10), &SCREEN, 12).0, 0);
    }

    #[test]
    fn saved_position_on_screen_is_kept() {
        assert_eq!(restore_position(Some((100, 200)), SIZE, &[SCREEN], 12), Some((100, 200)));
    }

    #[test]
    fn saved_position_is_nudged_fully_inside() {
        // Hangs over the right edge, centre still on screen.
        assert_eq!(restore_position(Some((1800, 20)), SIZE, &[SCREEN], 12), Some((1920 - 232, 20)));
        assert_eq!(restore_position(Some((-100, -30)), SIZE, &[SCREEN], 12), Some((0, 0)));
        // Centre already past the right edge: treated as lost, back to the default corner.
        assert_eq!(restore_position(Some((1850, 20)), SIZE, &[SCREEN], 12), Some((1676, 12)));
    }

    #[test]
    fn saved_position_on_unplugged_monitor_goes_to_default() {
        // Second monitor was at x = 1920..3840 and is gone.
        assert_eq!(restore_position(Some((2500, 100)), SIZE, &[SCREEN], 12), Some((1676, 12)));
        assert_eq!(restore_position(Some((-19_000, 0)), SIZE, &[SCREEN], 12), Some((1676, 12)));
    }

    #[test]
    fn nothing_saved_uses_first_area_and_no_info_changes_nothing() {
        let second = Rect::new(1920, 0, 1280, 1024);
        assert_eq!(restore_position(None, SIZE, &[SCREEN, second], 12), Some((1676, 12)));
        assert_eq!(restore_position(None, SIZE, &[], 12), None);
        assert_eq!(restore_position(Some((5, 6)), SIZE, &[], 12), Some((5, 6)));
    }

    #[test]
    fn saved_position_on_second_monitor_stays_there() {
        let second = Rect::new(1920, 0, 1280, 1024);
        assert_eq!(restore_position(Some((2000, 100)), SIZE, &[SCREEN, second], 12), Some((2000, 100)));
    }

    #[test]
    fn snaps_to_edges_within_threshold_only() {
        // Near the left/top edges: snaps to margin.
        assert_eq!(snap_position((14, 20), SIZE, &[SCREEN], 24, 8), (8, 8));
        // Near the right/bottom edges.
        assert_eq!(snap_position((1920 - 232 - 5, 1080 - 84 - 30), SIZE, &[SCREEN], 24, 8), (1920 - 232 - 8, 1080 - 84 - 8));
        // Far from every edge: unchanged.
        assert_eq!(snap_position((500, 400), SIZE, &[SCREEN], 24, 8), (500, 400));
        // Axes are independent: only x snaps.
        assert_eq!(snap_position((10, 400), SIZE, &[SCREEN], 24, 8), (8, 400));
    }

    #[test]
    fn snap_uses_the_monitor_the_window_is_on() {
        let second = Rect::new(1920, 0, 1280, 1024);
        let snapped = snap_position((1920 + 1280 - 232 - 3, 500), SIZE, &[SCREEN, second], 24, 8);
        assert_eq!(snapped, (1920 + 1280 - 232 - 8, 500));
    }

    #[test]
    fn snap_without_screens_is_a_no_op() {
        assert_eq!(snap_position((1, 2), SIZE, &[], 24, 8), (1, 2));
    }

    const XRANDR: &str = "\
Screen 0: minimum 320 x 200, current 3840 x 1080, maximum 16384 x 16384
HDMI-1 connected 1920x1080+1920+0 (normal left inverted right x axis y axis) 527mm x 296mm
   1920x1080     60.00*+  50.00
eDP-1 connected primary 1920x1080+0+0 (normal left inverted right x axis y axis) 344mm x 193mm
   1920x1080     60.01*+  48.01
DP-2 disconnected (normal left inverted right x axis y axis)
DP-3 connected (normal left inverted right x axis y axis)
";

    #[test]
    fn xrandr_lists_active_monitors_primary_first() {
        assert_eq!(parse_xrandr(XRANDR), vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1920, 1080)]);
    }

    #[test]
    fn xrandr_negative_offsets_and_garbage() {
        let text = "DP-1 connected 1280x1024+-1280+0 (normal) 1mm x 1mm\nnonsense\n\nDP-9 connected 0x0+0+0 (x)";
        assert_eq!(parse_xrandr(text), vec![Rect::new(-1280, 0, 1280, 1024)]);
        assert!(parse_xrandr("").is_empty());
    }

    #[test]
    fn workarea_parsing_and_clipping() {
        let w = parse_net_workarea("_NET_WORKAREA(CARDINAL) = 0, 0, 1920, 1040, 0, 0, 1920, 1040\n").unwrap();
        assert_eq!(w, Rect::new(0, 0, 1920, 1040));
        assert_eq!(parse_net_workarea("_NET_WORKAREA:  not found."), None);
        assert_eq!(parse_net_workarea("_NET_WORKAREA(CARDINAL) = 0, 0, 0, 0"), None);

        let clipped = apply_workarea(&[SCREEN, Rect::new(5000, 0, 800, 600)], Some(w));
        assert_eq!(clipped, vec![Rect::new(0, 0, 1920, 1040), Rect::new(5000, 0, 800, 600)]);
        assert_eq!(apply_workarea(&[SCREEN], None), vec![SCREEN]);
    }
}
