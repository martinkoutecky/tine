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

    /// Default display title, e.g. "Jun 14th, 2026".
    pub fn title(&self) -> String {
        let month = MONTHS.get((self.month - 1) as usize).copied().unwrap_or("?");
        format!("{} {}, {}", month, ordinal(self.day), self.year)
    }
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
