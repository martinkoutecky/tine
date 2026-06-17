//! Minimal journal date handling for the default Logseq formats
//! (file `yyyy_MM_dd`, title `MMM do, yyyy`). Configurable formats are a
//! later milestone; these defaults cover the standard graph layout.

/// A calendar date as (year, month, day). Stored as an ordinal key `yyyymmdd`
/// for cheap sorting/comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct JournalDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl JournalDate {
    pub fn ordinal_key(&self) -> i64 {
        self.year as i64 * 10000 + self.month as i64 * 100 + self.day as i64
    }

    /// Parse a display title in the default `MMM do, yyyy` format, e.g.
    /// "Jan 1st, 2022" or "Jun 14th, 2026". Returns `None` if it doesn't match.
    pub fn from_title(title: &str) -> Option<JournalDate> {
        let (md, year) = title.trim().rsplit_once(", ")?;
        let year: i32 = year.trim().parse().ok()?;
        let (mon, day) = md.trim().split_once(' ')?;
        let month = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(mon))? as u32 + 1;
        let digits: String = day.chars().take_while(|c| c.is_ascii_digit()).collect();
        let day: u32 = digits.parse().ok()?;
        if !(1..=31).contains(&day) {
            return None;
        }
        Some(JournalDate { year, month, day })
    }

    /// Parse a journal file stem. Accepts `yyyy_MM_dd` (default) and
    /// `yyyy-MM-dd` (legacy). Returns `None` if it isn't a date.
    pub fn from_file_stem(stem: &str) -> Option<JournalDate> {
        let sep = if stem.contains('_') { '_' } else { '-' };
        let parts: Vec<&str> = stem.split(sep).collect();
        if parts.len() != 3 {
            return None;
        }
        let year: i32 = parts[0].parse().ok()?;
        let month: u32 = parts[1].parse().ok()?;
        let day: u32 = parts[2].parse().ok()?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some(JournalDate { year, month, day })
    }

    /// Build from an ordinal key (`yyyymmdd`).
    pub fn from_ordinal(key: i64) -> JournalDate {
        JournalDate {
            year: (key / 10000) as i32,
            month: ((key / 100) % 100) as u32,
            day: (key % 100) as u32,
        }
    }

    /// Days since 1970-01-01 (proleptic Gregorian; Hinnant's algorithm).
    pub fn to_days(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }
    pub fn from_days(z: i64) -> JournalDate {
        let (year, month, day) = civil_from_days(z);
        JournalDate { year, month, day }
    }

    pub fn add_days(&self, n: i64) -> JournalDate {
        JournalDate::from_days(self.to_days() + n)
    }
    /// Add `n` months, carrying into years and clamping the day to month length.
    pub fn add_months(&self, n: i64) -> JournalDate {
        let total = (self.month as i64 - 1) + n;
        let year = self.year as i64 + total.div_euclid(12);
        let month = (total.rem_euclid(12) + 1) as u32;
        let day = self.day.min(days_in_month(year as i32, month));
        JournalDate { year: year as i32, month, day }
    }

    /// Today's date (UTC) from the system clock.
    pub fn today() -> JournalDate {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        JournalDate::from_days(secs.div_euclid(86_400))
    }

    /// Default display title, e.g. "Jun 14th, 2026".
    pub fn title(&self) -> String {
        let month = MONTHS.get((self.month - 1) as usize).copied().unwrap_or("?");
        format!("{} {}, {}", month, ordinal(self.day), self.year)
    }

    /// Default journal file stem, e.g. "2026_06_14" (round-trips `from_file_stem`).
    pub fn file_stem(&self) -> String {
        format!("{:04}_{:02}_{:02}", self.year, self.month, self.day)
    }
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(y) { 29 } else { 28 },
        _ => 30,
    }
}

// Howard Hinnant's civil <-> days-since-epoch algorithms (1970-01-01 = day 0).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = ((m as i64) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}
