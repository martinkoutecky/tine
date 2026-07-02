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
        Format::compile(DEFAULT_TITLE_FORMAT).parse(title.trim())
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

// ===== Configurable journal date formats (cljs-time / Joda subset) =====
//
// Logseq stores two `config.edn` formats: `:journal/file-name-format` (the .md
// filename, default `yyyy_MM_dd`) and `:journal/page-title-format` (the display
// title, default `MMM do, yyyy`). To "just work" on a foreign graph we parse a
// journal name against the user's formats AND a few safe defaults (mirrors OG's
// `safe-journal-title-formatters`), and render new titles/filenames in the
// user's format.

pub const DEFAULT_FILE_FORMAT: &str = "yyyy_MM_dd";
pub const DEFAULT_TITLE_FORMAT: &str = "MMM do, yyyy";

const MONTHS_FULL: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];
const WEEKDAYS_FULL: [&str; 7] =
    ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const WEEKDAYS_ABBR: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAYS_2: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
const WEEKDAYS_1: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];

#[derive(Clone, Debug)]
enum Tok {
    Lit(String),
    Year(u8),        // 4 = yyyy, 2 = yy, 1 = y (variable)
    MonthNum(bool),  // true = MM (padded), false = M
    MonthName(bool), // true = MMMM (full), false = MMM (abbr)
    DayNum(bool),    // true = dd, false = d
    DayOrd,          // do  -> "1st"
    Weekday(u8),     // 1..=4  (E .. EEEE); value is ignored on parse
}

/// A compiled cljs-time / Joda-style date pattern (the subset Logseq uses).
#[derive(Clone, Debug)]
pub struct Format {
    toks: Vec<Tok>,
}

impl Format {
    pub fn compile(pat: &str) -> Format {
        let b: Vec<char> = pat.chars().collect();
        let starts = |i: usize, s: &str| -> bool {
            let sc: Vec<char> = s.chars().collect();
            i + sc.len() <= b.len() && b[i..i + sc.len()] == sc[..]
        };
        let mut toks = Vec::new();
        let mut lit = String::new();
        let mut i = 0;
        macro_rules! flush {
            () => {
                if !lit.is_empty() {
                    toks.push(Tok::Lit(std::mem::take(&mut lit)));
                }
            };
        }
        while i < b.len() {
            // Longest field token wins; anything else is literal text.
            let (tok, len) = if starts(i, "yyyy") {
                (Some(Tok::Year(4)), 4)
            } else if starts(i, "yy") {
                (Some(Tok::Year(2)), 2)
            } else if starts(i, "y") {
                (Some(Tok::Year(1)), 1)
            } else if starts(i, "MMMM") {
                (Some(Tok::MonthName(true)), 4)
            } else if starts(i, "MMM") {
                (Some(Tok::MonthName(false)), 3)
            } else if starts(i, "MM") {
                (Some(Tok::MonthNum(true)), 2)
            } else if starts(i, "M") {
                (Some(Tok::MonthNum(false)), 1)
            } else if starts(i, "dd") {
                (Some(Tok::DayNum(true)), 2)
            } else if starts(i, "do") {
                (Some(Tok::DayOrd), 2)
            } else if starts(i, "d") {
                (Some(Tok::DayNum(false)), 1)
            } else if starts(i, "EEEE") {
                (Some(Tok::Weekday(4)), 4)
            } else if starts(i, "EEE") {
                (Some(Tok::Weekday(3)), 3)
            } else if starts(i, "EE") {
                (Some(Tok::Weekday(2)), 2)
            } else if starts(i, "E") {
                (Some(Tok::Weekday(1)), 1)
            } else {
                (None, 0)
            };
            match tok {
                Some(t) => {
                    flush!();
                    toks.push(t);
                    i += len;
                }
                None => {
                    lit.push(b[i]);
                    i += 1;
                }
            }
        }
        flush!();
        Format { toks }
    }

    pub fn format(&self, d: JournalDate) -> String {
        let dow = (d.to_days().rem_euclid(7) + 4).rem_euclid(7) as usize; // 0 = Sunday
        let mi = (d.month.max(1) - 1) as usize % 12;
        let mut out = String::new();
        for t in &self.toks {
            match t {
                Tok::Lit(s) => out.push_str(s),
                Tok::Year(4) => out.push_str(&format!("{:04}", d.year)),
                Tok::Year(2) => out.push_str(&format!("{:02}", d.year.rem_euclid(100))),
                Tok::Year(_) => out.push_str(&d.year.to_string()),
                Tok::MonthNum(true) => out.push_str(&format!("{:02}", d.month)),
                Tok::MonthNum(false) => out.push_str(&d.month.to_string()),
                Tok::MonthName(true) => out.push_str(MONTHS_FULL[mi]),
                Tok::MonthName(false) => out.push_str(MONTHS[mi]),
                Tok::DayNum(true) => out.push_str(&format!("{:02}", d.day)),
                Tok::DayNum(false) => out.push_str(&d.day.to_string()),
                Tok::DayOrd => out.push_str(&ordinal(d.day)),
                Tok::Weekday(4) => out.push_str(WEEKDAYS_FULL[dow]),
                Tok::Weekday(3) => out.push_str(WEEKDAYS_ABBR[dow]),
                Tok::Weekday(2) => out.push_str(WEEKDAYS_2[dow]),
                Tok::Weekday(_) => out.push_str(WEEKDAYS_1[dow]),
            }
        }
        out
    }

    pub fn parse(&self, s: &str) -> Option<JournalDate> {
        let cs: Vec<char> = s.chars().collect();
        let mut i = 0usize;
        let (mut year, mut month, mut day) = (None, None, None);
        for t in &self.toks {
            match t {
                Tok::Lit(lit) => {
                    let lc: Vec<char> = lit.chars().collect();
                    if i + lc.len() > cs.len() || cs[i..i + lc.len()] != lc[..] {
                        return None;
                    }
                    i += lc.len();
                }
                Tok::Year(4) => year = Some(take_digits(&cs, &mut i, 4)? as i32),
                Tok::Year(2) => year = Some(2000 + take_digits(&cs, &mut i, 2)? as i32),
                Tok::Year(_) => year = Some(take_digits(&cs, &mut i, 4)? as i32),
                Tok::MonthNum(_) => month = Some(take_digits(&cs, &mut i, 2)? as u32),
                Tok::MonthName(_) => {
                    let (idx, len) = match_name(&cs, i, &[&MONTHS_FULL[..], &MONTHS[..]])?;
                    month = Some(idx as u32 + 1);
                    i += len;
                }
                Tok::DayNum(_) => day = Some(take_digits(&cs, &mut i, 2)? as u32),
                Tok::DayOrd => {
                    day = Some(take_digits(&cs, &mut i, 2)? as u32);
                    take_ordinal_suffix(&cs, &mut i)?;
                }
                Tok::Weekday(_) => {
                    let (_, len) = match_name(
                        &cs,
                        i,
                        &[&WEEKDAYS_FULL[..], &WEEKDAYS_ABBR[..], &WEEKDAYS_2[..], &WEEKDAYS_1[..]],
                    )?;
                    i += len;
                }
            }
        }
        if i != cs.len() {
            return None;
        }
        let (y, m, d) = (year?, month?, day?);
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        Some(JournalDate { year: y, month: m, day: d })
    }
}

/// Take up to `max` ASCII digits from `cs` starting at `*i`, advancing `*i`.
fn take_digits(cs: &[char], i: &mut usize, max: usize) -> Option<i64> {
    let start = *i;
    while *i < cs.len() && *i - start < max && cs[*i].is_ascii_digit() {
        *i += 1;
    }
    if *i == start {
        return None;
    }
    cs[start..*i].iter().collect::<String>().parse().ok()
}

/// `do` requires an ordinal suffix, but OG cljs-time's parse-ordinal-suffix
/// accepts any of st/nd/rd/th without checking whether it matches the number.
fn take_ordinal_suffix(cs: &[char], i: &mut usize) -> Option<()> {
    for suffix in ["st", "nd", "rd", "th"] {
        if *i + 2 <= cs.len()
            && cs[*i..*i + 2].iter().collect::<String>().eq_ignore_ascii_case(suffix)
        {
            *i += 2;
            return Some(());
        }
    }
    None
}

/// Match the longest name (case-insensitive) from any table at position `i`;
/// returns `(index-within-its-table, chars-consumed)`.
fn match_name(cs: &[char], i: usize, tables: &[&[&str]]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for table in tables {
        for (idx, name) in table.iter().enumerate() {
            let nc: Vec<char> = name.chars().collect();
            if i + nc.len() <= cs.len() {
                let seg: String = cs[i..i + nc.len()].iter().collect();
                if seg.eq_ignore_ascii_case(name) && best.map_or(true, |(_, l)| nc.len() > l) {
                    best = Some((idx, nc.len()));
                }
            }
        }
    }
    best
}

/// The journal date formats for a graph: the user's filename + title patterns,
/// plus a fallback parse list so foreign/default journal files still resolve.
#[derive(Clone)]
pub struct JournalFormat {
    file: Format,
    title: Format,
    file_pat: String,
    title_pat: String,
    parse_list: Vec<Format>,
}

impl JournalFormat {
    /// Build from the config strings (`None`/empty → Logseq defaults).
    pub fn new(file_fmt: Option<&str>, title_fmt: Option<&str>) -> JournalFormat {
        let file_pat = file_fmt.filter(|s| !s.is_empty()).unwrap_or(DEFAULT_FILE_FORMAT).to_string();
        let title_pat =
            title_fmt.filter(|s| !s.is_empty()).unwrap_or(DEFAULT_TITLE_FORMAT).to_string();
        // OG-style "just works": try the user's formats first, then safe defaults.
        let mut pats: Vec<String> = Vec::new();
        for p in [
            file_pat.as_str(),
            title_pat.as_str(),
            DEFAULT_TITLE_FORMAT,
            "yyyy-MM-dd",
            DEFAULT_FILE_FORMAT,
        ] {
            if !pats.iter().any(|x| x == p) {
                pats.push(p.to_string());
            }
        }
        JournalFormat {
            file: Format::compile(&file_pat),
            title: Format::compile(&title_pat),
            parse_list: pats.iter().map(|p| Format::compile(p)).collect(),
            file_pat,
            title_pat,
        }
    }

    /// Default Logseq formats (`yyyy_MM_dd` / `MMM do, yyyy`).
    pub fn default() -> JournalFormat {
        Self::new(None, None)
    }

    /// Parse a journal filename stem OR display title to a date (tries every
    /// fallback format). `None` ⇒ not a journal date.
    pub fn parse(&self, s: &str) -> Option<JournalDate> {
        let s = s.trim();
        self.parse_list.iter().find_map(|f| f.parse(s))
    }

    /// Display title for a date, in the user's `:journal/page-title-format`.
    pub fn title(&self, d: JournalDate) -> String {
        self.title.format(d)
    }

    /// Filename stem for a date, in the user's `:journal/file-name-format`.
    pub fn file_stem(&self, d: JournalDate) -> String {
        self.file.format(d)
    }

    pub fn title_format(&self) -> &str {
        &self.title_pat
    }
    pub fn file_format(&self) -> &str {
        &self.file_pat
    }
}

#[cfg(test)]
mod fmt_tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> JournalDate {
        JournalDate { year: y, month: m, day }
    }

    #[test]
    fn format_roundtrips_common_patterns() {
        let cases = [
            ("yyyy_MM_dd", "2024_01_05"),
            ("yyyy-MM-dd", "2024-01-05"),
            ("dd-MM-yyyy", "05-01-2024"),
            ("MM/dd/yyyy", "01/05/2024"),
            ("dd.MM.yyyy", "05.01.2024"),
            ("yyyyMMdd", "20240105"),
            ("MMM do, yyyy", "Jan 5th, 2024"),
            ("MMMM do, yyyy", "January 5th, 2024"),
            ("do MMM yyyy", "5th Jan 2024"),
            ("yyyy年MM月dd日", "2024年01月05日"),
        ];
        for (pat, want) in cases {
            let f = Format::compile(pat);
            assert_eq!(f.format(d(2024, 1, 5)), want, "format {pat}");
            assert_eq!(f.parse(want), Some(d(2024, 1, 5)), "parse {pat} <- {want}");
        }
    }

    #[test]
    fn weekday_tokens_format_and_parse() {
        // 2024-01-05 is a Friday.
        let f = Format::compile("EEE, dd-MM-yyyy");
        assert_eq!(f.format(d(2024, 1, 5)), "Fri, 05-01-2024");
        assert_eq!(f.parse("Fri, 05-01-2024"), Some(d(2024, 1, 5)));
        let g = Format::compile("EEEE, yyyy/MM/dd");
        assert_eq!(g.format(d(2024, 1, 5)), "Friday, 2024/01/05");
        assert_eq!(g.parse("Friday, 2024/01/05"), Some(d(2024, 1, 5)));
    }

    #[test]
    fn ordinal_suffixes_parse() {
        let f = Format::compile("MMM do, yyyy");
        for (s, day) in [("Jan 1st, 2024", 1), ("Jan 2nd, 2024", 2), ("Jan 3rd, 2024", 3), ("Jan 21st, 2024", 21), ("Jan 22nd, 2024", 22)] {
            assert_eq!(f.parse(s), Some(d(2024, 1, day)));
        }
        assert_eq!(f.parse("Jan 1th, 2024"), Some(d(2024, 1, 1)));
        assert_eq!(f.parse("Jan 1, 2024"), None);
        assert_eq!(JournalDate::from_title("Jan 1th, 2024"), Some(d(2024, 1, 1)));
        assert_eq!(JournalDate::from_title("Jan 1, 2024"), None);
    }

    #[test]
    fn non_dates_reject() {
        let f = JournalFormat::default();
        for s in ["My Notes", "Project/Alpha", "2024-13-01", "hello world", ""] {
            assert_eq!(f.parse(s), None, "should reject {s:?}");
        }
    }

    #[test]
    fn fallback_recognizes_default_and_custom() {
        // A graph configured for dd-MM-yyyy filenames still resolves an old
        // default-format file, and vice-versa.
        let custom = JournalFormat::new(Some("dd-MM-yyyy"), Some("MMM do, yyyy"));
        assert_eq!(custom.parse("05-01-2024"), Some(d(2024, 1, 5))); // user's format
        assert_eq!(custom.parse("2024_01_05"), Some(d(2024, 1, 5))); // legacy default file
        assert_eq!(custom.parse("Jan 5th, 2024"), Some(d(2024, 1, 5))); // title ref
        assert_eq!(custom.file_stem(d(2024, 1, 5)), "05-01-2024");
    }

    #[derive(Debug, serde::Deserialize)]
    struct GoldenFixture {
        format: Vec<FormatVector>,
        parse: Vec<ParseVector>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct FormatVector {
        fmt: String,
        date: GoldenDate,
        title: String,
    }

    #[derive(Debug, serde::Deserialize)]
    struct ParseVector {
        fmt: String,
        input: String,
        date: Option<GoldenDate>,
    }

    #[derive(Clone, Copy, Debug, serde::Deserialize)]
    struct GoldenDate {
        y: i32,
        m: u32,
        d: u32,
    }

    impl GoldenDate {
        fn into_journal_date(self) -> JournalDate {
            d(self.y, self.m, self.d)
        }
    }

    fn load_date_golden_fixture() -> GoldenFixture {
        serde_json::from_str(include_str!("../../../src/fixtures/date-golden.json"))
            .expect("parse src/fixtures/date-golden.json")
    }

    #[test]
    fn date_golden_fixture_matches_rust_format() {
        let fixture = load_date_golden_fixture();
        for vector in fixture.format {
            let got = Format::compile(&vector.fmt).format(vector.date.into_journal_date());
            assert_eq!(got, vector.title, "format {:?}", vector);
        }
    }

    #[test]
    fn date_golden_fixture_matches_rust_parse() {
        let fixture = load_date_golden_fixture();
        for vector in fixture.parse {
            let want = vector.date.map(GoldenDate::into_journal_date);
            let got = Format::compile(&vector.fmt).parse(&vector.input);
            assert_eq!(got, want, "parse {:?}", vector);
        }
    }
}
