//! CSV export of the daily usage log (opens directly in Excel / LibreOffice).

use crate::UsageLog;

/// `YYYYMMDD` -> `YYYY-MM-DD`.
fn iso_date(date: u32) -> String {
    format!("{:04}-{:02}-{:02}", date / 10_000, date / 100 % 100, date % 100)
}

/// One row per day, oldest first: `date,download_bytes,upload_bytes,total_bytes`.
/// Plain bytes (not "3.2 GB") so spreadsheets can sum and chart the columns.
pub fn usage_csv(log: &UsageLog) -> String {
    let mut out = String::with_capacity(48 + log.days().len() * 40);
    out.push_str("date,download_bytes,upload_bytes,total_bytes\n");
    for d in log.days() {
        out.push_str(&format!(
            "{},{},{},{}\n",
            iso_date(d.date),
            d.rx,
            d.tx,
            d.rx as u128 + d.tx as u128
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_log_is_just_the_header() {
        assert_eq!(usage_csv(&UsageLog::new()), "date,download_bytes,upload_bytes,total_bytes\n");
    }

    #[test]
    fn rows_are_sorted_and_totals_do_not_overflow() {
        let mut log = UsageLog::new();
        log.add(20261115, 5, 6);
        log.add(20261114, u64::MAX, u64::MAX);
        let csv = usage_csv(&log);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[1], "2026-11-14,18446744073709551615,18446744073709551615,36893488147419103230");
        assert_eq!(lines[2], "2026-11-15,5,6,11");
    }
}
