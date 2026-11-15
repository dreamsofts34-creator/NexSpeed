//! Tiny CPU renderer for the speed graph (smooth curve, soft fill, live dot).
//! Output is premultiplied RGBA8. Pure Rust, no GPU, no extra crates.

/// Graph size in pixels (2x the logical 196x30 area, so it stays crisp).
pub const GRAPH_W: u32 = 392;
pub const GRAPH_H: u32 = 60;

/// Never scale the chart below this, so an idle connection draws a flat line, not noise.
const MIN_SCALE: f32 = 64.0 * 1024.0;

pub struct Palette {
    pub down: [u8; 3],
    pub up: [u8; 3],
}

pub const DARK: Palette = Palette { down: [0x8F, 0xB4, 0xFF], up: [0xD8, 0xD2, 0xC4] };
pub const LIGHT: Palette = Palette { down: [0x2F, 0x63, 0xD9], up: [0x8A, 0x7F, 0x63] };

pub fn palette(light: bool) -> &'static Palette {
    if light {
        &LIGHT
    } else {
        &DARK
    }
}

/// Source-over blend of `rgb` with coverage `a` onto a premultiplied pixel.
fn over(out: &mut [u8], idx: usize, rgb: [u8; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let inv = 1.0 - a;
    for c in 0..3 {
        let v = rgb[c] as f32 * a + out[idx + c] as f32 * inv;
        out[idx + c] = v.round().min(255.0) as u8;
    }
    let va = 255.0 * a + out[idx + 3] as f32 * inv;
    out[idx + 3] = va.round().min(255.0) as u8;
}

/// Smooth curve through the sample points (flat tangent at each point, no overshoot).
fn curve_y(ys: &[f32], x: f32, width: f32) -> f32 {
    let n = ys.len();
    let pos = (x / (width - 1.0)).clamp(0.0, 1.0) * (n - 1) as f32;
    let i = (pos.floor() as usize).min(n - 2);
    let t = pos - i as f32;
    let s = t * t * (3.0 - 2.0 * t);
    ys[i] + (ys[i + 1] - ys[i]) * s
}

fn stroke(out: &mut [u8], w: usize, h: usize, ys: &[f32], rgb: [u8; 3], thickness: f32) {
    let wf = w as f32;
    let hf = h as f32;
    for x in 0..w {
        let ya = curve_y(ys, x as f32, wf);
        let yb = curve_y(ys, x as f32 + 1.0, wf);
        let (lo, hi) = (ya.min(yb), ya.max(yb));
        let y0 = (lo - thickness).floor().max(0.0) as usize;
        let y1 = (hi + thickness).ceil().min(hf) as usize;
        for py in y0..y1 {
            let c = py as f32 + 0.5;
            // Distance from the pixel centre to the vertical span the curve covers in this column.
            let dist = if c < lo { lo - c } else if c > hi { c - hi } else { 0.0 };
            let cov = (thickness * 0.5 + 0.5 - dist).clamp(0.0, 1.0);
            over(out, (py * w + x) * 4, rgb, cov);
        }
    }
}

/// A filled circle: centre, radius, colour, opacity.
struct Dot {
    cx: f32,
    cy: f32,
    r: f32,
    rgb: [u8; 3],
    alpha: f32,
}

fn dot(out: &mut [u8], w: usize, h: usize, d: Dot) {
    let Dot { cx, cy, r, rgb, alpha } = d;
    let x0 = (cx - r - 1.0).floor().max(0.0) as usize;
    let x1 = ((cx + r + 1.0).ceil() as usize).min(w);
    let y0 = (cy - r - 1.0).floor().max(0.0) as usize;
    let y1 = ((cy + r + 1.0).ceil() as usize).min(h);
    for py in y0..y1 {
        for px in x0..x1 {
            let d = ((px as f32 + 0.5 - cx).powi(2) + (py as f32 + 0.5 - cy).powi(2)).sqrt();
            over(out, (py * w + px) * 4, rgb, (r + 0.5 - d).clamp(0.0, 1.0) * alpha);
        }
    }
}

/// Renders download (`down`) and upload (`up`) history into `out` (`w * h * 4` bytes).
pub fn render_sparkline(w: usize, h: usize, down: &[f32], up: &[f32], pal: &Palette, out: &mut [u8]) {
    assert_eq!(out.len(), w * h * 4, "buffer size mismatch");
    out.fill(0);
    if w < 8 || h < 8 || down.len() < 2 || up.len() < 2 {
        return;
    }
    let wf = w as f32;
    let hf = h as f32;
    let pad = 4.0;
    let peak = down.iter().chain(up.iter()).copied().fold(MIN_SCALE, f32::max) * 1.1;
    let to_y = |v: f32| (hf - pad) - (v / peak).clamp(0.0, 1.0) * (hf - 2.0 * pad);
    let dy: Vec<f32> = down.iter().map(|&v| to_y(v)).collect();
    let uy: Vec<f32> = up.iter().map(|&v| to_y(v)).collect();

    // Soft fill under the download curve, fading to transparent at the bottom.
    for x in 0..w {
        let yc = curve_y(&dy, x as f32 + 0.5, wf);
        for py in (yc.floor().max(0.0) as usize)..h {
            let c = py as f32 + 0.5;
            if c < yc {
                continue;
            }
            let fade = 1.0 - ((c - yc) / (hf - yc).max(1.0)).clamp(0.0, 1.0);
            over(out, (py * w + x) * 4, pal.down, 0.22 * fade);
        }
    }

    stroke(out, w, h, &uy, pal.up, 2.0);
    stroke(out, w, h, &dy, pal.down, 3.0);

    // Live point at the right end.
    let cx = wf - 6.0;
    let cy = curve_y(&dy, cx, wf);
    dot(out, w, h, Dot { cx, cy, r: 6.5, rgb: pal.down, alpha: 0.22 });
    dot(out, w, h, Dot { cx, cy, r: 3.0, rgb: pal.down, alpha: 1.0 });
}

/// Stacked daily bars: download at the bottom, upload on top. `days` is `(rx, tx)` bytes per day,
/// oldest first. Bars share one scale (the biggest day); a day with traffic is at least 2 px tall.
pub fn render_bars(w: usize, h: usize, days: &[(u64, u64)], pal: &Palette, out: &mut [u8]) {
    assert_eq!(out.len(), w * h * 4, "buffer size mismatch");
    out.fill(0);
    if w < 8 || h < 8 || days.is_empty() {
        return;
    }
    let n = days.len();
    let base = h - 3; // baseline row
    let top_pad = 4usize;
    let avail = (base - top_pad) as f64;
    let max = days.iter().map(|&(r, t)| r as f64 + t as f64).fold(0.0, f64::max);
    let slot = w as f64 / n as f64;
    let gap = (slot * 0.25).clamp(1.0, 6.0);
    let fill = |out: &mut [u8], x0: usize, x1: usize, y0: usize, y1: usize, rgb: [u8; 3]| {
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                over(out, (y * w + x) * 4, rgb, 1.0);
            }
        }
    };
    // Baseline.
    fill(out, 0, w, base, base + 1, [pal.up[0], pal.up[1], pal.up[2]]);
    for out_a in out.as_chunks_mut::<4>().0.iter_mut().skip(base * w).take(w) {
        // Soften the baseline: scale premultiplied pixel to ~35 % alpha.
        for c in out_a.iter_mut() {
            *c = (*c as f32 * 0.35) as u8;
        }
    }
    if max <= 0.0 {
        return;
    }
    for (i, &(rx, tx)) in days.iter().enumerate() {
        let x0 = (i as f64 * slot + gap / 2.0).round() as usize;
        let x1 = (((i + 1) as f64 * slot) - gap / 2.0).round().max(x0 as f64 + 1.0) as usize;
        let total = rx as f64 + tx as f64;
        if total <= 0.0 {
            continue;
        }
        let bar_h = ((total / max) * avail).round().max(2.0) as usize;
        let down_h = (((rx as f64 / total) * bar_h as f64).round() as usize).min(bar_h);
        let up_h = bar_h - down_h;
        fill(out, x0, x1, base - down_h, base, pal.down);
        fill(out, x0, x1, base - down_h - up_h, base - down_h, pal.up);
    }
}

/// Builds a Slint image from the history arrays.
pub fn sparkline_image(down: &[f32], up: &[f32], light: bool) -> slint::Image {
    sparkline_image_sized(GRAPH_W, GRAPH_H, down, up, light)
}

/// Same, at an explicit pixel size (the details window uses a bigger graph).
pub fn sparkline_image_sized(w: u32, h: u32, down: &[f32], up: &[f32], light: bool) -> slint::Image {
    let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    render_sparkline(w as usize, h as usize, down, up, palette(light), buf.make_mut_bytes());
    slint::Image::from_rgba8_premultiplied(buf)
}

/// Usage bars as a Slint image (see [`render_bars`]).
pub fn bars_image(w: u32, h: u32, days: &[(u64, u64)], light: bool) -> slint::Image {
    let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    render_bars(w as usize, h as usize, days, palette(light), buf.make_mut_bytes());
    slint::Image::from_rgba8_premultiplied(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_sum(buf: &[u8]) -> u64 {
        buf.as_chunks::<4>().0.iter().map(|p| p[3] as u64).sum()
    }

    #[test]
    fn curve_passes_through_first_and_last_points() {
        let ys = [10.0, 30.0, 20.0];
        assert!((curve_y(&ys, 0.0, 100.0) - 10.0).abs() < 1e-4);
        assert!((curve_y(&ys, 99.0, 100.0) - 20.0).abs() < 1e-4);
    }

    #[test]
    fn renders_something_for_activity() {
        let (w, h) = (96, 32);
        let mut buf = vec![0u8; w * h * 4];
        let down: Vec<f32> = (0..60).map(|i| (i as f32) * 50_000.0).collect();
        let up = vec![1000.0f32; 60];
        render_sparkline(w, h, &down, &up, &DARK, &mut buf);
        assert!(alpha_sum(&buf) > 0);
    }

    #[test]
    fn idle_still_draws_a_baseline() {
        let (w, h) = (96, 32);
        let mut buf = vec![0u8; w * h * 4];
        render_sparkline(w, h, &[0.0; 60], &[0.0; 60], &DARK, &mut buf);
        assert!(alpha_sum(&buf) > 0);
    }

    #[test]
    fn premultiplied_channels_never_exceed_alpha() {
        let (w, h) = (96, 32);
        let mut buf = vec![0u8; w * h * 4];
        let down: Vec<f32> = (0..60).map(|i| ((i * 37) % 11) as f32 * 80_000.0).collect();
        render_sparkline(w, h, &down, &[5000.0; 60], &DARK, &mut buf);
        assert!(buf.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }

    fn px(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn bars_scale_to_the_biggest_day_and_stack() {
        let (w, h) = (80usize, 50usize);
        let mut buf = vec![0u8; w * h * 4];
        // Two days: 10 units (all download) and 5 units (half upload).
        render_bars(w, h, &[(10, 0), (3, 2)], &DARK, &mut buf);
        // Day 0 occupies x 0..40: tall bar with download colour just above the baseline.
        let base = h - 3;
        assert_eq!(px(&buf, w, 20, base - 1), [0x8F, 0xB4, 0xFF, 255]);
        // Day 1 is half as tall: its bar is empty near the top of day 0's bar.
        assert_eq!(px(&buf, w, 60, 6)[3], 0);
        assert_eq!(px(&buf, w, 20, 6)[3], 255);
        // Day 1 has an upload segment above the download one.
        let col: Vec<[u8; 4]> = (0..h).map(|y| px(&buf, w, 60, y)).collect();
        assert!(col.iter().any(|p| p[..3] == [0xD8, 0xD2, 0xC4] && p[3] == 255));
    }

    #[test]
    fn bars_empty_or_zero_data_only_draws_baseline() {
        let (w, h) = (80usize, 50usize);
        let mut buf = vec![0u8; w * h * 4];
        render_bars(w, h, &[], &DARK, &mut buf);
        assert_eq!(alpha_sum(&buf), 0);
        render_bars(w, h, &[(0, 0), (0, 0)], &DARK, &mut buf);
        assert!(alpha_sum(&buf) > 0);
        assert!(buf.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }
}
