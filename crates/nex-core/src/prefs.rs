//! Small pure helpers for user preferences, calendar dates and limit alerts.

/// Overlay background opacity choices (percent) the tray cycles through.
pub const OPACITY_STEPS: [u8; 4] = [100, 85, 70, 50];

/// Next opacity after `current`; unknown values restart at 100.
pub fn next_opacity(current: u8) -> u8 {
    match OPACITY_STEPS.iter().position(|&o| o == current) {
        Some(i) => OPACITY_STEPS[(i + 1) % OPACITY_STEPS.len()],
        None => OPACITY_STEPS[0],
    }
}

/// Overlay styles: 0 = Graph, 1 = Minimal (numbers only), 2 = Compact (one line).
pub const STYLE_COUNT: u8 = 3;

pub fn next_style(current: u8) -> u8 {
    if current + 1 >= STYLE_COUNT {
        0
    } else {
        current + 1
    }
}

pub fn style_name(style: u8) -> &'static str {
    match style {
        1 => "Minimal",
        2 => "Compact",
        _ => "Graph",
    }
}

/// 0 = below 80 %, 1 = 80 % or more, 2 = 100 % or more. `None` (no limit) is level 0.
pub fn limit_level(percent: Option<f64>) -> u8 {
    match percent {
        Some(p) if p >= 100.0 => 2,
        Some(p) if p >= 80.0 => 1,
        _ => 0,
    }
}

fn is_leap(y: u32) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// The day before `date` (`YYYYMMDD`).
pub fn prev_day(date: u32) -> u32 {
    let (y, m, d) = (date / 10_000, date / 100 % 100, date % 100);
    if d > 1 {
        date - 1
    } else if m > 1 {
        y * 10_000 + (m - 1) * 100 + days_in_month(y, m - 1)
    } else {
        (y.saturating_sub(1)) * 10_000 + 1_200 + 31
    }
}

/// `n` dates ending at `today`, oldest first (`today` is the last element).
pub fn last_days(today: u32, n: usize) -> Vec<u32> {
    let mut v = Vec::with_capacity(n);
    let mut d = today;
    for _ in 0..n {
        v.push(d);
        d = prev_day(d);
    }
    v.reverse();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_cycles() {
        assert_eq!(next_opacity(100), 85);
        assert_eq!(next_opacity(50), 100);
        assert_eq!(next_opacity(33), 100);
    }

    #[test]
    fn style_cycles_and_names() {
        assert_eq!(next_style(0), 1);
        assert_eq!(next_style(2), 0);
        assert_eq!(style_name(0), "Graph");
        assert_eq!(style_name(2), "Compact");
        assert_eq!(style_name(9), "Graph");
    }

    #[test]
    fn limit_levels() {
        assert_eq!(limit_level(None), 0);
        assert_eq!(limit_level(Some(79.9)), 0);
        assert_eq!(limit_level(Some(80.0)), 1);
        assert_eq!(limit_level(Some(100.0)), 2);
    }

    #[test]
    fn previous_day_crosses_month_and_year() {
        assert_eq!(prev_day(20261115), 20261114);
        assert_eq!(prev_day(20261101), 20261031);
        assert_eq!(prev_day(20260101), 20251231);
        assert_eq!(prev_day(20240301), 20240229); // leap year
        assert_eq!(prev_day(20230301), 20230228);
        assert_eq!(prev_day(20000301), 20000229); // divisible by 400
        assert_eq!(prev_day(19000301), 19000228); // divisible by 100 only
    }

    #[test]
    fn last_days_is_oldest_first() {
        assert_eq!(last_days(20261102, 4), vec![20261030, 20261031, 20261101, 20261102]);
        assert_eq!(last_days(20261102, 0), Vec::<u32>::new());
    }
}
