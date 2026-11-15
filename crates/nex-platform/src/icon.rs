//! Tray icon. The real app icon is embedded as raw straight (non-premultiplied) RGBA8 at 32 and
//! 64 px (no image-decoding crate needed). Other sizes fall back to a simple badge drawn in code.

const TRAY_32: &[u8] = include_bytes!("../assets/tray-32.rgba");
const TRAY_64: &[u8] = include_bytes!("../assets/tray-64.rgba");

fn in_arrow(u: f32, v: f32) -> bool {
    let stem = (0.43..=0.57).contains(&u) && (0.22..=0.52).contains(&v);
    let head = (0.5..=0.78).contains(&v) && (u - 0.5).abs() <= 0.22 * (0.78 - v) / 0.28;
    stem || head
}

pub fn tray_icon_rgba(size: u32) -> Vec<u8> {
    match size {
        32 => TRAY_32.to_vec(),
        64 => TRAY_64.to_vec(),
        _ => drawn_icon_rgba(size),
    }
}

fn drawn_icon_rgba(size: u32) -> Vec<u8> {
    const SS: usize = 3; // 3x3 supersampling for smooth edges
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (mut circle, mut arrow) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    if (u - 0.5).powi(2) + (v - 0.5).powi(2) <= 0.47 * 0.47 {
                        circle += 1;
                        if in_arrow(u, v) {
                            arrow += 1;
                        }
                    }
                }
            }
            if circle == 0 {
                continue;
            }
            let t = arrow as f32 / circle as f32;
            let mix = |accent: f32, dark: f32| (accent * (1.0 - t) + dark * t).round() as u8;
            let i = (y * n + x) * 4;
            out[i] = mix(143.0, 11.0);
            out[i + 1] = mix(180.0, 11.0);
            out[i + 2] = mix(255.0, 13.0);
            out[i + 3] = (circle as f32 / (SS * SS) as f32 * 255.0).round() as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icons_have_the_right_byte_count() {
        assert_eq!(TRAY_32.len(), 32 * 32 * 4);
        assert_eq!(TRAY_64.len(), 64 * 64 * 4);
        assert_eq!(tray_icon_rgba(64).len(), 64 * 64 * 4);
        assert_eq!(tray_icon_rgba(20).len(), 20 * 20 * 4); // fallback drawing
    }

    #[test]
    fn has_right_size_transparent_corners_and_opaque_centre() {
        let px = tray_icon_rgba(32);
        assert_eq!(px.len(), 32 * 32 * 4);
        assert_eq!(px[3], 0); // top-left corner
        let centre = (16 * 32 + 16) * 4;
        assert_eq!(px[centre + 3], 255);
    }
}
