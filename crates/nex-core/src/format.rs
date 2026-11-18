#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitSystem {
    /// B/s, KB/s, MB/s, GB/s (base 1024, what most speed meters show).
    Bytes,
    /// bps, Kbps, Mbps, Gbps (bits, base 1000, what ISPs advertise).
    Bits,
}

/// Rounds `v` the way it will be printed (whole number for the base unit and for
/// values >= 100, one decimal for >= 10, two otherwise), so unit boundaries are
/// decided on the displayed value: 1023.6 B/s must become `1.00 KB/s`, not `1024 B/s`.
fn display_round(v: f64, unit_index: usize) -> f64 {
    if unit_index == 0 || v >= 100.0 {
        v.round()
    } else if v >= 10.0 {
        (v * 10.0).round() / 10.0
    } else {
        (v * 100.0).round() / 100.0
    }
}

/// Picks the unit: returns the scaled value and the index into the unit labels.
fn scale(value: f64, base: f64, last: usize) -> (f64, usize) {
    let mut v = if value.is_finite() && value > 0.0 { value } else { 0.0 };
    let mut i = 0;
    while i < last && display_round(v, i) >= base {
        v /= base;
        i += 1;
    }
    (v, i)
}

fn print_scaled(v: f64, i: usize) -> String {
    let shown = display_round(v, i);
    if i == 0 || shown >= 100.0 {
        format!("{:.0}", shown)
    } else if shown >= 10.0 {
        format!("{:.1}", shown)
    } else {
        format!("{:.2}", shown)
    }
}

/// Formats a speed given in bytes per second, e.g. `5.40 MB/s` or `43.2 Mbps`.
pub fn format_speed(bytes_per_sec: f64, units: UnitSystem) -> String {
    let (value, base, labels): (f64, f64, [&str; 4]) = match units {
        UnitSystem::Bytes => (bytes_per_sec, 1024.0, ["B/s", "KB/s", "MB/s", "GB/s"]),
        UnitSystem::Bits => (bytes_per_sec * 8.0, 1000.0, ["bps", "Kbps", "Mbps", "Gbps"]),
    };
    let (v, i) = scale(value, base, labels.len() - 1);
    format!("{} {}", print_scaled(v, i), labels[i])
}

/// Formats a byte count, e.g. `3.20 GB` (base 1024, like most usage trackers).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (v, i) = scale(bytes as f64, 1024.0, UNITS.len() - 1);
    format!("{} {}", print_scaled(v, i), UNITS[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_at_unit_boundaries_moves_to_the_next_unit() {
        assert_eq!(format_speed(1023.6, UnitSystem::Bytes), "1.00 KB/s");
        assert_eq!(format_speed(1023.4, UnitSystem::Bytes), "1023 B/s");
        // 999.6 Kbps is 124 950 bytes/s
        assert_eq!(format_speed(124_950.0, UnitSystem::Bits), "1.00 Mbps");
        assert_eq!(format_speed(1_048_575.0, UnitSystem::Bytes), "1.00 MB/s");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1_048_575), "1.00 MB");
        // 99.96 prints as 100, without a decimal.
        assert_eq!(format_speed(99.96 * 1024.0, UnitSystem::Bytes), "100 KB/s");
    }

    #[test]
    fn byte_counts() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes((3.2 * 1024.0 * 1024.0 * 1024.0) as u64), "3.20 GB");
    }

    #[test]
    fn bytes_units() {
        assert_eq!(format_speed(0.0, UnitSystem::Bytes), "0 B/s");
        assert_eq!(format_speed(1536.0, UnitSystem::Bytes), "1.50 KB/s");
        assert_eq!(format_speed(5.4 * 1024.0 * 1024.0, UnitSystem::Bytes), "5.40 MB/s");
    }

    #[test]
    fn bits_units() {
        assert_eq!(format_speed(125_000.0, UnitSystem::Bits), "1.00 Mbps");
    }

    #[test]
    fn bad_input_is_zero() {
        assert_eq!(format_speed(f64::NAN, UnitSystem::Bytes), "0 B/s");
        assert_eq!(format_speed(-5.0, UnitSystem::Bytes), "0 B/s");
    }
}
