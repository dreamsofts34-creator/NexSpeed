#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitSystem {
    /// B/s, KB/s, MB/s, GB/s (base 1024, what most speed meters show).
    Bytes,
    /// bps, Kbps, Mbps, Gbps (bits, base 1000, what ISPs advertise).
    Bits,
}

/// Formats a speed given in bytes per second, e.g. `5.40 MB/s` or `43.2 Mbps`.
pub fn format_speed(bytes_per_sec: f64, units: UnitSystem) -> String {
    let (value, base, labels): (f64, f64, [&str; 4]) = match units {
        UnitSystem::Bytes => (bytes_per_sec, 1024.0, ["B/s", "KB/s", "MB/s", "GB/s"]),
        UnitSystem::Bits => (bytes_per_sec * 8.0, 1000.0, ["bps", "Kbps", "Mbps", "Gbps"]),
    };
    let mut v = if value.is_finite() && value > 0.0 { value } else { 0.0 };
    let mut i = 0;
    while v >= base && i < labels.len() - 1 {
        v /= base;
        i += 1;
    }
    if i == 0 {
        format!("{:.0} {}", v, labels[0])
    } else if v >= 100.0 {
        format!("{:.0} {}", v, labels[i])
    } else if v >= 10.0 {
        format!("{:.1} {}", v, labels[i])
    } else {
        format!("{:.2} {}", v, labels[i])
    }
}

/// Formats a byte count, e.g. `3.20 GB` (base 1024, like most usage trackers).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", bytes)
    } else if v >= 100.0 {
        format!("{:.0} {}", v, UNITS[i])
    } else if v >= 10.0 {
        format!("{:.1} {}", v, UNITS[i])
    } else {
        format!("{:.2} {}", v, UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
