//! Journal dates and compiled Logseq filename/title formats. E/EE/EEE render
//! abbreviated weekdays and EEEE full names; parsing also accepts old Tine
//! one/two-letter weekday titles. Cost is O(pattern + title), without file I/O.

/// A calendar date as (year, month, day). Stored as an ordinal key `yyyymmdd`
/// for cheap sorting/comparison.
#[deny(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct JournalDate {
    /// Gregorian year.
    pub year: i32,
    /// Month, normally 1 through 12.
    pub month: u32,
    /// Day of month, normally 1 through 31.
    pub day: u32,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl JournalDate {
    /// Encode this date as `yyyymmdd` for `Day` and journal indexes.
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
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
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
    /// Convert days since 1970-01-01 back to a Gregorian date.
    pub fn from_days(z: i64) -> JournalDate {
        let (year, month, day) = civil_from_days(z);
        JournalDate { year, month, day }
    }

    /// Add signed calendar days and return the resulting date.
    pub fn add_days(&self, n: i64) -> JournalDate {
        JournalDate::from_days(self.to_days() + n)
    }
    /// Add `n` months, carrying into years and clamping the day to month length.
    pub fn add_months(&self, n: i64) -> JournalDate {
        let total = (self.month as i64 - 1) + n;
        let year = self.year as i64 + total.div_euclid(12);
        let month = (total.rem_euclid(12) + 1) as u32;
        let day = self.day.min(days_in_month(year as i32, month));
        JournalDate {
            year: year as i32,
            month,
            day,
        }
    }

    /// Today's date in the device's local civil calendar.  Journal membership,
    /// relative queries, and their date-keyed caches must all agree on this
    /// boundary; UTC epoch days split those user-facing meanings near midnight.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn today() -> JournalDate {
        use chrono::{Datelike, Local};
        let now = Local::now();
        JournalDate {
            year: now.year(),
            month: now.month(),
            day: now.day(),
        }
    }

    /// The device's current UTC offset (minutes east of UTC) and the instant it
    /// was sampled, read through the same time-zone source as [`Self::today`].
    /// The frontend corrects its own wall clock by this (GH #607): a WebView's
    /// bundled ICU can carry older zone rules than the OS, and the two sides then
    /// disagree about the calendar day near midnight.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn local_utc_offset_now() -> (i32, i64) {
        let now = chrono::Local::now();
        (now.offset().local_minus_utc() / 60, now.timestamp_millis())
    }

    /// Default display title, e.g. "Jun 14th, 2026".
    pub fn title(&self) -> String {
        let month = MONTHS
            .get((self.month - 1) as usize)
            .copied()
            .unwrap_or("?");
        format!("{} {}, {}", month, ordinal(self.day), self.year)
    }

    /// Default journal file stem, e.g. "2026_06_14" (round-trips `from_file_stem`).
    pub fn file_stem(&self) -> String {
        format!("{:04}_{:02}_{:02}", self.year, self.month, self.day)
    }
}

/// UTC backup directory stamp in `YYYY-MM-DD_HH-MM-SS` form.
pub fn utc_backup_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let date = JournalDate::from_days(days);
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        date.year,
        date.month,
        date.day,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
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
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS_FULL: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
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
#[deny(missing_docs)]
#[derive(Clone, Debug)]
pub struct Format {
    toks: Vec<Tok>,
}

/// The most width renderings one journal pattern may contribute to
/// [`JournalFormat::accepted_stems`] (GH #863 D3): an upper bound on the product
/// of each numeric field's possible widths, so a pattern that repeats fields
/// cannot make an absent-day lookup allocate or stat exponentially many names.
pub const WIDTH_RENDERINGS_MAX: usize = 16;
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
        let mut out = String::new();
        for t in &self.toks {
            render_token(t, d, &mut out);
        }
        out
    }

    pub fn parse(&self, s: &str) -> Option<JournalDate> {
        let mut cs = Vec::with_capacity(s.len());
        cs.extend(s.chars());
        self.parse_chars(&cs)
    }

    /// [`Self::format`] of `d` with each numeric field at every width
    /// [`Self::parse`] reads (a field of up to `max` digits, zero-padded
    /// from none to `max`): `2026-11-1` beside `2026-11-01`. Names, ordinal
    /// suffixes and literals as `format` writes them. The product of each
    /// numeric field's widths: 2 for a month or day below 10, 1 for a year
    /// past 999, so at most 4 for a pattern with one year, month and day
    /// field. Not checked to parse back.
    fn renderings_by_width(&self, d: JournalDate) -> Vec<String> {
        // A numeric field's value and the most digits the parser reads for it.
        let field = |t: &Tok| match t {
            Tok::Year(2) => Some((d.year.rem_euclid(100), 2)),
            Tok::Year(_) => Some((d.year, 4)),
            Tok::MonthNum(_) => Some((d.month as i32, 2)),
            Tok::DayNum(_) | Tok::DayOrd => Some((d.day as i32, 2)),
            _ => None,
        };
        // A format may repeat numeric fields (config accepts "yyyy-MM-dd M M M …"),
        // and each occurrence multiplies the product: past the bound the pattern
        // contributes nothing here, and only its ordinary spelling is tried.
        let within = self.toks.iter().try_fold(1usize, |n, t| {
            let widths = field(t).map_or(1, |(value, max)| {
                let digits = value.to_string().len();
                max.max(digits) - digits + 1
            });
            Some(n * widths).filter(|&n| n <= WIDTH_RENDERINGS_MAX)
        });
        if within.is_none() {
            return Vec::new();
        }
        let mut out = vec![String::new()];
        for t in &self.toks {
            let (value, max) = field(t).unwrap_or((-1, 0));
            let single = Format {
                toks: vec![t.clone()],
            };
            let pieces: Vec<String> = if value < 0 {
                vec![single.format(d)]
            } else {
                let suffix = if matches!(t, Tok::DayOrd) {
                    &ordinal(d.day)[value.to_string().len()..]
                } else {
                    ""
                };
                let digits = value.to_string().len();
                (digits..=max.max(digits))
                    .map(|width| format!("{value:0width$}{suffix}"))
                    .collect()
            };
            out = (out.iter())
                .flat_map(|head| pieces.iter().map(move |piece| format!("{head}{piece}")))
                .collect();
        }
        out
    }

    /// [`Self::parse`] over already-decoded characters, so a caller trying
    /// several formats decodes the name once.
    fn parse_chars(&self, cs: &[char]) -> Option<JournalDate> {
        self.read(cs, &mut ())
    }

    /// THE date parser ([`Self::parse`]), reporting each step to `seen`:
    /// nothing at parse time (`()`), the title-format check's class analysis
    /// otherwise ([`title_refusal`]). One grammar, never a second.
    fn read(&self, cs: &[char], seen: &mut impl Observe) -> Option<JournalDate> {
        let mut i = 0usize;
        let (mut year, mut month, mut day) = (None, None, None);
        for t in &self.toks {
            let start = i;
            match t {
                Tok::Lit(lit) => {
                    for character in lit.chars() {
                        seen.literal(i, character);
                        if cs.get(i) != Some(&character) {
                            return None;
                        }
                        i += 1;
                    }
                }
                Tok::Year(2) => year = Some(2000 + take_digits(cs, &mut i, 2)? as i32),
                Tok::Year(_) => year = Some(take_digits(cs, &mut i, 4)? as i32),
                Tok::MonthNum(_) => month = Some(take_digits(cs, &mut i, 2)? as u32),
                Tok::MonthName(_) => {
                    let (idx, len) = match_name(cs, i, &[&MONTHS_FULL[..], &MONTHS[..]])?;
                    month = Some(idx as u32 + 1);
                    i += len;
                }
                Tok::DayNum(_) => day = Some(take_digits(cs, &mut i, 2)? as u32),
                Tok::DayOrd => {
                    day = Some(take_digits(cs, &mut i, 2)? as u32);
                    seen.field(DateField::Day, t, start..i);
                    take_ordinal_suffix(cs, &mut i)?;
                }
                Tok::Weekday(_) => {
                    let (_, len) = match_name(
                        cs,
                        i,
                        &[
                            &WEEKDAYS_FULL[..],
                            &WEEKDAYS_ABBR[..],
                            &WEEKDAYS_2[..],
                            &WEEKDAYS_1[..],
                        ],
                    )?;
                    i += len;
                }
            }
            match t {
                Tok::Year(_) => seen.field(DateField::Year, t, start..i),
                Tok::MonthNum(_) | Tok::MonthName(_) => seen.field(DateField::Month, t, start..i),
                Tok::DayNum(_) => seen.field(DateField::Day, t, start..i),
                _ => {}
            }
        }
        if i != cs.len() {
            return None;
        }
        let (y, m, d) = (year?, month?, day?);
        seen.read_whole();
        if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
            return None;
        }
        Some(JournalDate {
            year: y,
            month: m,
            day: d,
        })
    }
}

/// Where [`render_token`] writes: a `String`, or the characters the parser
/// reads (the title check renders straight into them).
trait Sink {
    fn put_str(&mut self, s: &str);
    fn put(&mut self, c: char);
    /// `value` in decimal, zero-padded to at least `width` digits.
    fn put_number(&mut self, value: u32, width: usize) {
        let mut digits = [0u8; 10];
        let (mut n, mut len) = (value, 0);
        loop {
            digits[len] = b'0' + (n % 10) as u8;
            len += 1;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        for _ in len..width {
            self.put('0');
        }
        for &digit in digits[..len].iter().rev() {
            self.put(digit as char);
        }
    }
}

impl Sink for String {
    fn put_str(&mut self, s: &str) {
        self.push_str(s)
    }
    fn put(&mut self, c: char) {
        self.push(c)
    }
}

impl Sink for Vec<char> {
    fn put_str(&mut self, s: &str) {
        self.extend(s.chars())
    }
    fn put(&mut self, c: char) {
        self.push(c)
    }
}

/// Append token `t`'s rendering of `d` (`Format::format`, one token). Digits
/// are written directly: the title check renders every token once per class.
fn render_token(t: &Tok, d: JournalDate, out: &mut impl Sink) {
    let dow = || (d.to_days().rem_euclid(7) + 4).rem_euclid(7) as usize; // 0 = Sunday
    let mi = (d.month.max(1) - 1) as usize % 12;
    let (value, width) = match t {
        Tok::Lit(s) => return out.put_str(s),
        // Years in JOURNAL_YEARS are never negative; others keep `{}`.
        Tok::Year(4) if d.year >= 0 => (d.year as u32, 4),
        Tok::Year(4) => return out.put_str(&format!("{:04}", d.year)),
        Tok::Year(2) => (d.year.rem_euclid(100) as u32, 2),
        Tok::Year(_) if d.year >= 0 => (d.year as u32, 1),
        Tok::Year(_) => return out.put_str(&d.year.to_string()),
        Tok::MonthNum(padded) => (d.month, if *padded { 2 } else { 1 }),
        Tok::MonthName(true) => return out.put_str(MONTHS_FULL[mi]),
        Tok::MonthName(false) => return out.put_str(MONTHS[mi]),
        Tok::DayNum(padded) => (d.day, if *padded { 2 } else { 1 }),
        Tok::DayOrd => return out.put_str(&ordinal(d.day)),
        Tok::Weekday(4) => return out.put_str(WEEKDAYS_FULL[dow()]),
        Tok::Weekday(_) => return out.put_str(WEEKDAYS_ABBR[dow()]),
    };
    out.put_number(value, width);
}

// ===== Which title formats name one day (D-20261119-YL) =====
//
// A configured `:journal/page-title-format` renders journal titles only if
// every title it writes names its day alone: for every day `d` with a year in
// [`JOURNAL_YEARS`], the pattern's own parser reads its title back as `d`
// (`format.parse(key(format.render(d))) = d`, where the key is the trimming
// [`JournalFormat::parse_text`] applies). The effective title is the first
// text route ([`text_routes`]), so its own complete parse decides: no other
// pattern can repair or spoil the round trip, and the check depends on the
// title pattern alone. A pattern that fails is not used, contributes no text
// route; titles use [`DEFAULT_TITLE_FORMAT`] and the refusal says why. Decided
// by an exact analysis of the parser's steps, never by sampling the graph's
// days ([`title_refusal`]).

/// The years a journal day may have: what the parser reads (at most four year
/// digits) and the renderer writes back exactly.
pub const JOURNAL_YEARS: std::ops::RangeInclusive<i32> = 0..=9999;

/// A date field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateField {
    /// The year.
    Year,
    /// The month.
    Month,
    /// The day of the month.
    Day,
}

/// Why a configured title format is not used ([`JournalFormat::title_refusal`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleFormatRefusal {
    /// The pattern writes no such field, so its titles name no single day.
    Missing(DateField),
    /// The title the pattern writes for `day`, `written`, opens as another day
    /// (`read_as`) or as no day at all (`None`).
    Misread {
        /// The day.
        day: JournalDate,
        /// Its title under the configured pattern.
        written: String,
        /// The day the pattern reads that title as.
        read_as: Option<JournalDate>,
    },
}

/// What [`Format::read`] reports while reading: nothing for an ordinary parse.
trait Observe {
    /// A pattern literal character expected at input position `at`.
    fn literal(&mut self, _at: usize, _expected: char) {}
    /// A date field read from input positions `span`.
    fn field(&mut self, _field: DateField, _tok: &Tok, _span: std::ops::Range<usize>) {}
    /// The whole input was read and every field set: only the calendar check
    /// remains.
    fn read_whole(&mut self) {}
}

impl Observe for () {}

/// The pattern's reading of one class representative's title: which positions
/// hold digits whose values vary across the class, and what the reader did.
struct Trace<'a> {
    /// Input positions of the varying digits each field token wrote, in order.
    slots: &'a [(std::ops::Range<usize>, Tok)],
    /// The first pattern digit literal compared to a varying digit, as
    /// `(position, expected)`: from there the reading depends on its value.
    dependent: Option<(usize, char)>,
    /// The last span read per field, with the reading token.
    spans: [Option<(std::ops::Range<usize>, Tok)>; 3],
    whole: bool,
}

impl Observe for Trace<'_> {
    fn literal(&mut self, at: usize, expected: char) {
        if self.dependent.is_none()
            && expected.is_ascii_digit()
            && slot_meeting(self.slots, at..at + 1).is_some()
        {
            self.dependent = Some((at, expected));
        }
    }
    fn field(&mut self, field: DateField, tok: &Tok, span: std::ops::Range<usize>) {
        self.spans[field as usize] = Some((span, tok.clone()));
    }
    fn read_whole(&mut self) {
        self.whole = true;
    }
}

/// The slot (of `slots`, in input order and disjoint) that `span` meets.
/// O(log slots).
fn slot_meeting(
    slots: &[(std::ops::Range<usize>, Tok)],
    span: std::ops::Range<usize>,
) -> Option<&(std::ops::Range<usize>, Tok)> {
    let first = slots.partition_point(|(slot, _)| slot.end <= span.start);
    slots.get(first).filter(|(slot, _)| slot.start < span.end)
}

/// The field a token writes or reads, if any.
fn field_of(tok: &Tok) -> Option<DateField> {
    match tok {
        Tok::Year(_) => Some(DateField::Year),
        Tok::MonthNum(_) | Tok::MonthName(_) => Some(DateField::Month),
        Tok::DayNum(_) | Tok::DayOrd => Some(DateField::Day),
        Tok::Lit(_) | Tok::Weekday(_) => None,
    }
}

/// [`title_refusal`] for a title pattern, memoized per process by that pattern
/// alone (the verdict depends on nothing else): `JournalFormat` is built
/// wherever config is read, and the check reads one title per class.
fn checked_title(title_pat: &str, title: &Format) -> Option<TitleFormatRefusal> {
    type Memo = Vec<(String, Option<TitleFormatRefusal>)>;
    static MEMO: std::sync::Mutex<Memo> = std::sync::Mutex::new(Vec::new());
    const MEMO_MAX: usize = 8;
    if let Some((_, verdict)) =
        (MEMO.lock().ok()).and_then(|memo| memo.iter().find(|(k, _)| k == title_pat).cloned())
    {
        return verdict;
    }
    let verdict = title_refusal(title);
    if let Ok(mut memo) = MEMO.lock() {
        if memo.len() >= MEMO_MAX {
            memo.remove(0);
        }
        memo.push((title_pat.to_string(), verdict.clone()));
    }
    verdict
}

/// Whether `title` names every day in [`JOURNAL_YEARS`] alone (`None`), else
/// the first failure found.
///
/// Exact, by classes. A class fixes everything about a title but the values
/// of its numeric fields' digits: the year's digit count when the title writes
/// an unpadded year, the weekday when it writes one, and a shape of month and
/// day (the month when it writes a month name, the digit count of an unpadded
/// month or day, the ordinal suffix). The reader's steps depend on a digit's
/// value only where a pattern digit literal meets it or in the final calendar
/// check, so one representative's traced reading decides the whole class: it
/// fails for all (a step failed with no value dependence), or reads every
/// field exactly from the digits that field wrote, or from characters fixed
/// for the class (so every class day reads as itself). A class it leaves
/// undecided is split by month and day, where only the year's digits vary: at
/// the first literal that meets a year digit, every member whose digit there
/// differs fails to read back, so one such member is the refusal; otherwise
/// the year is read exactly or the class is checked day by day. One render and
/// one read per class, each one pass over the pattern's tokens:
/// O(classes · pattern), independent of every other pattern.
fn title_refusal(title: &Format) -> Option<TitleFormatRefusal> {
    title_refusal_by(title, true)
}

/// [`title_refusal`]; `by_shape` false skips the shape classes and decides
/// every month and day on its own (the reference the shape classes must agree
/// with, in tests).
fn title_refusal_by(title: &Format, by_shape: bool) -> Option<TitleFormatRefusal> {
    let has = |f: fn(&Tok) -> bool| title.toks.iter().any(f);
    for (field, present) in [
        (DateField::Year, has(|t| matches!(t, Tok::Year(_)))),
        (
            DateField::Month,
            has(|t| matches!(t, Tok::MonthNum(_) | Tok::MonthName(_))),
        ),
        (
            DateField::Day,
            has(|t| matches!(t, Tok::DayNum(_) | Tok::DayOrd)),
        ),
    ] {
        if !present {
            return Some(TitleFormatRefusal::Missing(field));
        }
    }
    let by_length = has(|t| matches!(t, Tok::Year(1)));
    let by_weekday = has(|t| matches!(t, Tok::Weekday(_)));
    // Search order: the years people write first, so a refusal names one.
    let years = || {
        (1900..=2100)
            .chain(0..1900)
            .chain(2101..=*JOURNAL_YEARS.end())
    };
    let misread = |d: JournalDate| {
        #[cfg(test)]
        count_validator_read();
        let written = title.format(d);
        let read_as = title.parse(written.trim());
        (read_as != Some(d)).then_some(TitleFormatRefusal::Misread {
            day: d,
            written,
            read_as,
        })
    };
    let lengths: &[u32] = if by_length { &[1, 2, 3, 4] } else { &[0] };
    let weekdays: &[Option<i64>] = if by_weekday {
        &[
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(6),
        ]
    } else {
        &[None]
    };
    let member = |length: u32, weekday: Option<i64>, d: JournalDate| {
        (d.month != 2 || d.day != 29 || is_leap(d.year))
            && (length == 0 || d.year.to_string().len() as u32 == length)
            && weekday.map_or(true, |w| d.to_days().rem_euclid(7) == w)
    };
    // The month and day shape: what of them a title writes besides digit values.
    let (month_name, short_month) = (
        has(|t| matches!(t, Tok::MonthName(_))),
        has(|t| matches!(t, Tok::MonthNum(false))),
    );
    let (short_day, ordinal_day) = (
        has(|t| matches!(t, Tok::DayNum(false) | Tok::DayOrd)),
        has(|t| matches!(t, Tok::DayOrd)),
    );
    let shape = |month: u32, day: u32| {
        (
            if month_name {
                month
            } else if short_month {
                u32::from(month >= 10)
            } else {
                0
            },
            u32::from(short_day && day >= 10),
            if ordinal_day {
                ordinal(day)[day.to_string().len()..].to_owned()
            } else {
                String::new()
            },
        )
    };
    let mut shapes: Vec<((u32, u32, String), Vec<(u32, u32)>)> = Vec::new();
    for month in 1..=12u32 {
        for day in 1..=days_in_month(2000, month) {
            let key = if by_shape {
                shape(month, day)
            } else {
                (month, day, String::new())
            };
            match shapes.iter_mut().find(|(k, _)| *k == key) {
                Some((_, pairs)) => pairs.push((month, day)),
                None => shapes.push((key, vec![(month, day)])),
            }
        }
    }
    let mut buffer = Vec::new();
    // One month and day's class: only the year's digits vary.
    let mut pair_class = |month: u32, day: u32, length: u32, weekday: Option<i64>| {
        let date = |year| JournalDate { year, month, day };
        let in_class = |year: &i32| member(length, weekday, date(*year));
        let rep = years().find(in_class)?;
        let day_by_day = || years().filter(in_class).find_map(|y| misread(date(y)));
        match class_verdict(title, date(rep), &mut buffer) {
            Verdict::Names => None,
            Verdict::Refused => misread(date(rep)),
            Verdict::Depends { tok, offset, digit } => {
                let differs = |y: &i32| {
                    let mut year = String::new();
                    render_token(&tok, date(*y), &mut year);
                    year.chars().nth(offset) != Some(digit)
                };
                misread(date(rep))
                    .or_else(|| {
                        (years().filter(in_class))
                            .find(differs)
                            .and_then(|y| misread(date(y)))
                    })
                    .or_else(day_by_day)
            }
            Verdict::DayByDay => day_by_day(),
        }
    };
    let mut shape_buffer = Vec::new();
    for (_, pairs) in &shapes {
        for &length in lengths {
            for &weekday in weekdays {
                let rep = years().find_map(|year| {
                    (pairs.iter())
                        .map(|&(month, day)| JournalDate { year, month, day })
                        .find(|d| member(length, weekday, *d))
                });
                let Some(rep) = rep else {
                    continue;
                };
                let verdict = (pairs.len() > 1)
                    .then(|| shape_verdict(title, rep, pairs, &mut shape_buffer))
                    .unwrap_or(ShapeVerdict::Undecided);
                match verdict {
                    ShapeVerdict::Names => {}
                    ShapeVerdict::Refused => return misread(rep),
                    ShapeVerdict::Undecided => {
                        for &(month, day) in pairs {
                            if let Some(refusal) = pair_class(month, day, length, weekday) {
                                return Some(refusal);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
thread_local! {
    /// Title renders and reads [`title_refusal`] made on this thread.
    static VALIDATOR_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_validator_read() {
    VALIDATOR_READS.with(|reads| reads.set(reads.get() + 1));
}

/// `rep`'s title as the parser reads it (trimmed, as
/// `JournalFormat::parse_text` trims), rendered into `buffer`, with the input
/// positions of the digits each token `slot` selects wrote. Digits only, so
/// trimming never cuts into one. One pass over the tokens.
fn render_for_read(
    title: &Format,
    rep: JournalDate,
    buffer: &mut Vec<char>,
    slot: impl Fn(&Tok) -> bool,
) -> (std::ops::Range<usize>, Vec<(std::ops::Range<usize>, Tok)>) {
    buffer.clear();
    let mut slots: Vec<(std::ops::Range<usize>, Tok)> = Vec::new();
    for t in &title.toks {
        let start = buffer.len();
        render_token(t, rep, buffer);
        if slot(t) {
            let digits = buffer[start..].iter().take_while(|c| c.is_ascii_digit());
            slots.push((start..start + digits.count(), t.clone()));
        }
    }
    let lead = buffer.iter().take_while(|c| c.is_whitespace()).count();
    let trail = (buffer[lead..].iter().rev())
        .take_while(|c| c.is_whitespace())
        .count();
    for (slot, _) in &mut slots {
        *slot = slot.start - lead..slot.end - lead;
    }
    (lead..buffer.len() - trail, slots)
}

enum ShapeVerdict {
    /// Every class day's title reads back as that day.
    Names,
    /// The representative's title does not, nor does any class day's.
    Refused,
    /// Split the class by month and day.
    Undecided,
}

/// [`title_refusal`] for the shape class of `rep`, whose month and day are
/// among `pairs`: one render and one traced read, where every numeric field's
/// digits vary.
fn shape_verdict(
    title: &Format,
    rep: JournalDate,
    pairs: &[(u32, u32)],
    buffer: &mut Vec<char>,
) -> ShapeVerdict {
    #[cfg(test)]
    count_validator_read();
    let numeric = |t: &Tok| !matches!(t, Tok::Lit(_) | Tok::Weekday(_) | Tok::MonthName(_));
    let (input, slots) = render_for_read(title, rep, buffer, numeric);
    let mut trace = Trace {
        slots: &slots,
        dependent: None,
        spans: [None, None, None],
        whole: false,
    };
    let got = title.read(&buffer[input], &mut trace);
    if trace.dependent.is_some() {
        return ShapeVerdict::Undecided;
    }
    if !trace.whole {
        return ShapeVerdict::Refused; // Failed before any value mattered.
    }
    let Some(got) = got.filter(|got| *got == rep) else {
        return ShapeVerdict::Undecided;
    };
    for (field, span) in [DateField::Year, DateField::Month, DateField::Day]
        .into_iter()
        .zip(&trace.spans)
    {
        let (span, reader) = span.as_ref().expect("read_whole sets every field");
        let exact = match slot_meeting(&slots, span.clone()) {
            // Read from the digits that field wrote, whole; `yy` reads 20xx.
            Some((slot, writer)) => {
                slot == span
                    && field_of(writer) == Some(field)
                    && !matches!(writer, Tok::Year(2))
                    && !matches!(reader, Tok::Year(2))
            }
            // Read from characters fixed for the class: right for every class
            // day only if they all share the value read.
            None => match field {
                DateField::Year => false,
                DateField::Month => pairs.iter().all(|&(month, _)| month == got.month),
                DateField::Day => pairs.iter().all(|&(_, day)| day == got.day),
            },
        };
        if !exact {
            return ShapeVerdict::Undecided;
        }
    }
    ShapeVerdict::Names
}

enum Verdict {
    /// Every class day's title reads back as that day.
    Names,
    /// The representative's title does not.
    Refused,
    /// The reading compares the pattern digit `digit` to character `offset`
    /// of the year `tok` writes: a member whose year differs there fails.
    Depends {
        tok: Tok,
        offset: usize,
        digit: char,
    },
    /// Undecided by the representative.
    DayByDay,
}

/// [`title_refusal`] for the class of `rep`'s month and day, where only the
/// year's digits vary: one render and one traced read, rendered into `buffer`.
fn class_verdict(title: &Format, rep: JournalDate, buffer: &mut Vec<char>) -> Verdict {
    #[cfg(test)]
    count_validator_read();
    let (input, year_slots) = render_for_read(title, rep, buffer, |t| matches!(t, Tok::Year(_)));
    let mut trace = Trace {
        slots: &year_slots,
        dependent: None,
        spans: [None, None, None],
        whole: false,
    };
    let got = title.read(&buffer[input], &mut trace);
    if let Some((at, digit)) = trace.dependent {
        let (slot, tok) =
            slot_meeting(&year_slots, at..at + 1).expect("a dependent position is a year digit");
        return Verdict::Depends {
            tok: tok.clone(),
            offset: at - slot.start,
            digit,
        };
    }
    if !trace.whole {
        return Verdict::Refused; // Failed before any value mattered: fails for the class.
    }
    let read = |field: &Option<(std::ops::Range<usize>, Tok)>| {
        field.clone().expect("read_whole sets every field")
    };
    let [year, month, day] = &trace.spans;
    let (year, month, day) = (read(year), read(month), read(day));
    let in_year = |span: &std::ops::Range<usize>| slot_meeting(&year_slots, span.clone()).is_some();
    // `yy` reads 20xx: exact only for this century, so day by day.
    let exact_year = !matches!(year.1, Tok::Year(2))
        && (year_slots.iter()).any(|(slot, tok)| *slot == year.0 && !matches!(tok, Tok::Year(2)));
    if !exact_year || in_year(&month.0) || in_year(&day.0) {
        return Verdict::DayByDay;
    }
    // The month and day are read from fixed characters, the year exactly:
    // every class day reads as (its year, these month and day).
    match got {
        Some(d) if (d.month, d.day) == (rep.month, rep.day) => Verdict::Names,
        Some(_) => Verdict::Refused,
        // The calendar check failed for this year; another class year may
        // pass it (29 February).
        None => Verdict::DayByDay,
    }
}

/// What [`Format::compile`] does with a pattern part it does not support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedHandling {
    /// Letters Tine writes, and expects when reading, verbatim.
    Literal,
    /// A run of `y`, `M`, `d` or `E` Tine reads as several shorter fields.
    ShorterFields,
    /// Quoted text: Tine keeps the quote marks and reads letters inside as fields.
    Quoted,
}

/// A pattern part, as Logseq's cljs-time tokenises it, that Tine does not
/// render or read the way Logseq does.
#[deny(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedToken {
    /// The part exactly as written, quote marks included.
    pub text: String,
    /// What Tine does with it instead.
    pub handling: UnsupportedHandling,
}

/// The parts of `pattern` that [`Format::compile`] does not interpret as
/// Logseq does, in pattern order. Detection only: `compile` is unchanged.
/// Tokenises like cljs-time `internal/parse.cljs` `read-pattern` (logseq fork
/// 5704fbf, pinned by OG `deps.edn`): `'` starts quoted text up to the next `'`
/// (`''` is one quote), an ASCII letter starts a run of that same letter, and
/// anything else starts a literal run up to the next ASCII letter or `'`.
/// Supported runs are the ones `compile` reads as one field (`y yy yyyy M MM
/// MMM MMMM d dd E EE EEE EEEE`, and `o` straight after a single `d`); literal
/// runs are supported. O(pattern).
pub fn unsupported_tokens(pattern: &str) -> Vec<UnsupportedToken> {
    let cs: Vec<char> = pattern.chars().collect();
    let mut out = Vec::new();
    let mut previous = String::new();
    let mut i = 0;
    while i < cs.len() {
        let start = i;
        let first = cs[i];
        i += 1;
        let handling = if first == '\'' {
            if cs.get(i) == Some(&'\'') {
                i += 1;
            } else {
                while i < cs.len() && cs[i] != '\'' {
                    i += 1;
                }
                i = (i + 1).min(cs.len());
            }
            Some(UnsupportedHandling::Quoted)
        } else if first.is_ascii_alphabetic() {
            while i < cs.len() && cs[i] == first {
                i += 1;
            }
            let run: String = cs[start..i].iter().collect();
            let supported = matches!(
                run.as_str(),
                "y" | "yy"
                    | "yyyy"
                    | "M"
                    | "MM"
                    | "MMM"
                    | "MMMM"
                    | "d"
                    | "dd"
                    | "E"
                    | "EE"
                    | "EEE"
                    | "EEEE"
            ) || (run == "o" && previous == "d");
            (!supported).then_some(if matches!(first, 'y' | 'M' | 'd' | 'E') {
                UnsupportedHandling::ShorterFields
            } else {
                UnsupportedHandling::Literal
            })
        } else {
            while i < cs.len() && !cs[i].is_ascii_alphabetic() && cs[i] != '\'' {
                i += 1;
            }
            None
        };
        previous = cs[start..i].iter().collect();
        if let Some(handling) = handling {
            out.push(UnsupportedToken {
                text: previous.clone(),
                handling,
            });
        }
    }
    out
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
    cs[start..*i].iter().try_fold(0i64, |value, digit| {
        value
            .checked_mul(10)?
            .checked_add((*digit as u8 - b'0') as i64)
    })
}

/// `do` requires an ordinal suffix, but OG cljs-time's parse-ordinal-suffix
/// accepts any of st/nd/rd/th without checking whether it matches the number.
fn take_ordinal_suffix(cs: &[char], i: &mut usize) -> Option<()> {
    for suffix in ["st", "nd", "rd", "th"] {
        if *i + 2 <= cs.len()
            && cs[*i..*i + 2]
                .iter()
                .zip(suffix.chars())
                .all(|(character, expected)| character.eq_ignore_ascii_case(&expected))
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
            let length = name.chars().count();
            if i + length <= cs.len() {
                let matches = cs[i..i + length]
                    .iter()
                    .zip(name.chars())
                    .all(|(character, expected)| character.eq_ignore_ascii_case(&expected));
                if matches && best.map_or(true, |(_, l)| length > l) {
                    best = Some((idx, length));
                }
            }
        }
    }
    best
}

/// THE order in which text names a day (D-20261119-YL, model round 3): every
/// consumer of a name, a link, a query argument, search, capture, publication,
/// the CLI and wasm reads text through [`JournalFormat::parse_text`], which
/// tries these. The effective title format first (a refused title format is no
/// route), then the configured file-name format (D-20261102-JDL: Martin chose
/// the native list; deliberate OG deviation, OG `page-name->map` tries title
/// formats only, so `[[22-10-2026]]` under `dd-MM-yyyy` is that day's journal
/// everywhere), then `MMM do, yyyy`, `yyyy-MM-dd` and `yyyy_MM_dd`, each once.
fn text_routes(file_pat: &str, effective_title_pat: &str) -> Vec<Format> {
    routes([
        effective_title_pat,
        file_pat,
        DEFAULT_TITLE_FORMAT,
        "yyyy-MM-dd",
        DEFAULT_FILE_FORMAT,
    ])
}

/// THE order in which a journal file's stem names its day (D-20261119-YL,
/// model round 4: unchanged by the title ruling, so every existing file keeps
/// its day): the configured file-name format, the configured title format even
/// when refused for text, then `MMM do, yyyy`, `yyyy-MM-dd` and `yyyy_MM_dd`,
/// each once. Read through [`JournalFormat::parse_file_stem`] by journal path
/// extraction, discovery ([`JournalFormat::accepted_stems`]), find, reuse and
/// create, and claimant selection; never by text.
fn file_stem_routes(file_pat: &str, configured_title_pat: &str) -> Vec<Format> {
    routes([
        file_pat,
        configured_title_pat,
        DEFAULT_TITLE_FORMAT,
        "yyyy-MM-dd",
        DEFAULT_FILE_FORMAT,
    ])
}

/// `patterns` compiled in order, each distinct pattern once: trying a pattern
/// twice cannot change the first success.
fn routes(patterns: [&str; 5]) -> Vec<Format> {
    let mut seen: Vec<&str> = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        if !seen.contains(&pattern) {
            seen.push(pattern);
        }
    }
    seen.into_iter().map(Format::compile).collect()
}

/// The first route that reads `s` (trimmed) as a day.
fn first_route(routes: &[Format], s: &str) -> Option<JournalDate> {
    let s = s.trim();
    // One pre-sized buffer for every format tried (I-25,
    // tests/og_p10_journal_date_cost.rs).
    let mut cs = Vec::with_capacity(s.len());
    cs.extend(s.chars());
    routes.iter().find_map(|f| f.parse_chars(&cs))
}

/// The text routes ([`text_routes`]) of a file-name format and an
/// *effective* title format, built without the title check: for a reader
/// handed the native core's decision. The frontend reads text through wasm
/// with `GraphMeta.journal_title.effective` ([`JournalFormat::title_format`],
/// decided once per config load in native code), so no title pattern is
/// checked on the graph window's load or config path (D-20261119-YL). A
/// [`JournalFormat`] holds the same routes. O(patterns) to build.
#[derive(Clone)]
pub struct TextRoutes(Vec<Format>);

impl TextRoutes {
    /// The routes for config strings (`None`/empty → Logseq defaults);
    /// `effective_title_fmt` must be an effective title format, never a
    /// refused configured one.
    pub fn new(file_fmt: Option<&str>, effective_title_fmt: Option<&str>) -> TextRoutes {
        TextRoutes(text_routes(
            file_fmt
                .filter(|s| !s.is_empty())
                .unwrap_or(DEFAULT_FILE_FORMAT),
            effective_title_fmt
                .filter(|s| !s.is_empty())
                .unwrap_or(DEFAULT_TITLE_FORMAT),
        ))
    }

    /// [`JournalFormat::parse_text`].
    pub fn parse(&self, s: &str) -> Option<JournalDate> {
        first_route(&self.0, s)
    }

    /// [`JournalFormat::absent_link_is_journal`]: a link target no file,
    /// alias or reference claims is a journal iff it is a date as text.
    pub fn absent_link_is_journal(&self, name: &str) -> bool {
        self.parse(name).is_some()
    }
}

/// The journal date formats for a graph: the user's filename + title patterns,
/// and the two route lists that read a day back: one for text
/// ([`text_routes`]), one for a journal file's stem ([`file_stem_routes`]).
#[deny(missing_docs)]
#[derive(Clone)]
pub struct JournalFormat {
    file: Format,
    /// The effective title renderer: the configured pattern when its titles
    /// name one day each, else the default (D-20261119-YL).
    title: Format,
    file_pat: String,
    title_pat: String,
    /// The configured title pattern (the default when none is set).
    configured_title_pat: String,
    /// Why the configured title pattern is not used, if it is not.
    title_refusal: Option<TitleFormatRefusal>,
    text_routes: TextRoutes,
    file_stem_routes: Vec<Format>,
}

impl JournalFormat {
    /// Build from config strings (`None`/empty → Logseq defaults). Supported
    /// field tokens are compiled; unrecognized characters become literals
    /// ([`unsupported_tokens`] reports them).
    ///
    /// The one place a configured title pattern is accepted or refused
    /// (D-20261119-YL, I-12): it renders journal titles only if, for every
    /// day in [`JOURNAL_YEARS`], its own parser reads its title back as that
    /// day ([`title_refusal`]); otherwise titles use [`DEFAULT_TITLE_FORMAT`],
    /// the pattern is no text route, and [`Self::title_refusal`] says why.
    /// O(patterns) after the first check of a title pattern in this process.
    pub fn new(file_fmt: Option<&str>, title_fmt: Option<&str>) -> JournalFormat {
        let file_pat = file_fmt
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_FILE_FORMAT)
            .to_string();
        let title_pat = title_fmt
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_TITLE_FORMAT)
            .to_string();
        let configured = Format::compile(&title_pat);
        let title_refusal = (title_pat != DEFAULT_TITLE_FORMAT)
            .then(|| checked_title(&title_pat, &configured))
            .flatten();
        let (title, effective) = match title_refusal {
            Some(_) => (Format::compile(DEFAULT_TITLE_FORMAT), DEFAULT_TITLE_FORMAT),
            None => (configured, title_pat.as_str()),
        };
        JournalFormat {
            file: Format::compile(&file_pat),
            title,
            title_pat: effective.to_string(),
            text_routes: TextRoutes(text_routes(&file_pat, effective)),
            file_stem_routes: file_stem_routes(&file_pat, &title_pat),
            file_pat,
            configured_title_pat: title_pat,
            title_refusal,
        }
    }

    /// Default Logseq formats (`yyyy_MM_dd` / `MMM do, yyyy`).
    pub fn default() -> JournalFormat {
        Self::new(None, None)
    }

    /// The day a piece of text names (a page name, a link target, a query
    /// argument, a typed title), trimmed, by [`text_routes`]. `None` means no
    /// route matched: the text is an ordinary name.
    pub fn parse_text(&self, s: &str) -> Option<JournalDate> {
        self.text_routes.parse(s)
    }

    /// The day a journal file's stem names, trimmed, by [`file_stem_routes`]:
    /// a file's calendar identity, never the meaning of typed text. `None`
    /// means the stem names no day.
    pub fn parse_file_stem(&self, stem: &str) -> Option<JournalDate> {
        first_route(&self.file_stem_routes, stem)
    }

    /// Every rendering of `d` by a text route that [`Self::parse_text`] reads
    /// back as `d`, deduplicated: the spellings of the day as text. A refused
    /// title pattern is no route, so its rendering (`11-02` under `MM-dd`) is
    /// no spelling. Real names reached by a looser spelling (`2026-10-2`,
    /// `October 22nd, 2026`) are found by parsing them, not here.
    /// O(routes²).
    pub fn spellings(&self, d: JournalDate) -> Vec<String> {
        Self::renderings(&self.text_routes.0, d)
    }

    /// The renderings of `d` by `routes` that the first of them to read each
    /// reads back as `d`, deduplicated.
    fn renderings(routes: &[Format], d: JournalDate) -> Vec<String> {
        let mut out: Vec<String> = Vec::with_capacity(routes.len());
        for f in routes {
            let s = f.format(d);
            if !out.contains(&s) && first_route(routes, &s) == Some(d) {
                out.push(s);
            }
        }
        out
    }

    /// The journal file stems [`Self::parse_file_stem`] reads as `d`: each
    /// file-stem route's rendering, then those differing only in numeric field
    /// width (`2026-11-1`, `2026_11_01`), deduplicated, each checked to parse
    /// back (a two-digit year outside 2000-2099 does not): where a file of the
    /// day may be found by name without listing a directory (GH #863 D3).
    /// Bounded by `renderings_by_width`: at most 4 per route with one year,
    /// month and day field, for a year past 999, and never more than
    /// [`WIDTH_RENDERINGS_MAX`] per route (a route whose width product would
    /// exceed it contributes only its ordinary spelling).
    /// Other accepted forms (a month name's other length, any ordinal
    /// suffix, another weekday, case, surrounding space) are not listed.
    pub fn accepted_stems(&self, d: JournalDate) -> Vec<String> {
        let routes = &self.file_stem_routes;
        let widths = routes.iter().flat_map(|f| f.renderings_by_width(d));
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<String> = Vec::new();
        for s in Self::renderings(routes, d).into_iter().chain(widths) {
            if !seen.contains(&s) && self.parse_file_stem(&s) == Some(d) {
                seen.insert(s.clone());
                out.push(s);
            }
        }
        out
    }

    /// A cheap necessary condition for `self.parse_text(name) == Some(day)`
    /// and `self.parse_file_stem(name) == Some(day)`: every route reads the
    /// day of month and the year as contiguous ASCII digit runs
    /// (`Format::parse`: `d`/`dd`/`do` and `yy`/`yyyy`), so the name contains
    /// `day.day` and `day.year % 100` in decimal (a padded run contains the
    /// unpadded digits). Lets a scan for one day's spellings skip most names
    /// unparsed; never a replacement for parsing.
    pub fn may_spell_day(name: &str, day: JournalDate) -> bool {
        Self::day_spelling_prefilter(day)(name)
    }

    /// [`Self::may_spell_day`] for one `day` over many names: the digit
    /// needles are rendered once.
    pub fn day_spelling_prefilter(day: JournalDate) -> impl Fn(&str) -> bool {
        let (dom, year) = (day.day.to_string(), day.year.rem_euclid(100).to_string());
        move |name| name.contains(dom.as_str()) && name.contains(year.as_str())
    }

    /// The kind of a link target no file, alias or reference claims: a
    /// journal iff the name is a date as text ([`Self::parse_text`]). The one
    /// rule; the store's `link_target` and the frontend (through wasm) both
    /// call it.
    pub fn absent_link_is_journal(&self, name: &str) -> bool {
        self.text_routes.absent_link_is_journal(name)
    }

    /// Display title for a date, in the effective title format
    /// ([`Self::title_format`]): it names `d` alone.
    pub fn title(&self, d: JournalDate) -> String {
        self.title.format(d)
    }

    /// Filename stem for a date, in the user's `:journal/file-name-format`.
    pub fn file_stem(&self, d: JournalDate) -> String {
        self.file.format(d)
    }

    /// Whether `stem` names the canonical file of its day: a `yyyy_MM_dd` /
    /// `yyyy-MM-dd` date stem, or exactly the stem the configured
    /// `:journal/file-name-format` renders for the date it parses to. The one
    /// answer to "is this journal file the day's own file or a stray" — a
    /// configured-format file must never count as a stray of itself (C3 L05).
    pub fn is_canonical_stem(&self, stem: &str) -> bool {
        JournalDate::from_file_stem(stem).is_some()
            || self
                .file
                .parse(stem)
                .is_some_and(|d| self.writes_stem(d, stem))
    }

    /// Whether `stem` is exactly the file name this format writes for `day`:
    /// the one answer to "is this the day's preferred file" (claimant ranking).
    /// [`Self::is_canonical_stem`] builds on it and also admits the ISO and
    /// underscore stems, which are never strays but not the written name.
    pub fn writes_stem(&self, day: JournalDate, stem: &str) -> bool {
        self.file.format(day) == stem
    }

    /// The effective page-title format: the configured one, including
    /// unsupported literals, unless refused ([`Self::title_refusal`]), then
    /// [`DEFAULT_TITLE_FORMAT`].
    pub fn title_format(&self) -> &str {
        &self.title_pat
    }
    /// The configured page-title format ([`DEFAULT_TITLE_FORMAT`] when none).
    pub fn configured_title_format(&self) -> &str {
        &self.configured_title_pat
    }
    /// Why the configured page-title format is not used, if it is not.
    pub fn title_refusal(&self) -> Option<&TitleFormatRefusal> {
        self.title_refusal.as_ref()
    }
    /// Configured filename format string, including unsupported literals.
    pub fn file_format(&self) -> &str {
        &self.file_pat
    }
}

#[cfg(test)]
mod fmt_tests {
    use super::*;

    #[test]
    fn configured_file_stem_is_its_days_canonical_file() {
        for (pat, own, stray) in [
            ("dd-MM-yyyy", "24-06-2026", "Wednesday, 24-06-2026"),
            ("yyyyMMdd", "20260624", "Jun 24th, 2026"),
            ("yyyy.MM.dd", "2026.06.24", "Jun 24th, 2026"),
        ] {
            let f = JournalFormat::new(Some(pat), Some("EEEE, dd-MM-yyyy"));
            assert!(f.is_canonical_stem(own), "{pat}");
            assert!(f.is_canonical_stem("2026_06_24"), "{pat}");
            assert!(!f.is_canonical_stem(stray), "{pat}");
        }
    }

    fn d(y: i32, m: u32, day: u32) -> JournalDate {
        JournalDate {
            year: y,
            month: m,
            day,
        }
    }

    #[test]
    fn rejects_impossible_calendar_days() {
        for stem in ["2026_02_31", "2026-02-29", "2026_04_31"] {
            assert_eq!(JournalDate::from_file_stem(stem), None, "{stem}");
            assert_eq!(JournalFormat::default().parse_text(stem), None, "{stem}");
            assert_eq!(
                JournalFormat::default().parse_file_stem(stem),
                None,
                "{stem}"
            );
        }
        assert_eq!(
            Format::compile("MMM do, yyyy").parse("Feb 31st, 2026"),
            None
        );
        assert_eq!(
            JournalDate::from_file_stem("2024_02_29"),
            Some(d(2024, 2, 29))
        );
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
        for (s, day) in [
            ("Jan 1st, 2024", 1),
            ("Jan 2nd, 2024", 2),
            ("Jan 3rd, 2024", 3),
            ("Jan 21st, 2024", 21),
            ("Jan 22nd, 2024", 22),
        ] {
            assert_eq!(f.parse(s), Some(d(2024, 1, day)));
        }
        assert_eq!(f.parse("Jan 1th, 2024"), Some(d(2024, 1, 1)));
        assert_eq!(f.parse("Jan 1, 2024"), None);
        assert_eq!(
            JournalDate::from_title("Jan 1th, 2024"),
            Some(d(2024, 1, 1))
        );
        assert_eq!(JournalDate::from_title("Jan 1, 2024"), None);
    }

    #[test]
    fn non_dates_reject() {
        let f = JournalFormat::default();
        for s in ["My Notes", "Project/Alpha", "2024-13-01", "hello world", ""] {
            assert_eq!(f.parse_text(s), None, "should reject {s:?}");
        }
    }

    #[test]
    fn fallback_recognizes_default_and_custom() {
        // A graph configured for dd-MM-yyyy filenames still resolves an old
        // default-format file, and vice-versa.
        let custom = JournalFormat::new(Some("dd-MM-yyyy"), Some("MMM do, yyyy"));
        for parse in [JournalFormat::parse_text, JournalFormat::parse_file_stem] {
            assert_eq!(parse(&custom, "05-01-2024"), Some(d(2024, 1, 5))); // user's format
            assert_eq!(parse(&custom, "2024_01_05"), Some(d(2024, 1, 5))); // legacy default file
            assert_eq!(parse(&custom, "Jan 5th, 2024"), Some(d(2024, 1, 5))); // title ref
        }
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

    /// Card g4czVA: a pattern part Tine does not support is reported in
    /// pattern order, tokenised as cljs-time `read-pattern` does.
    #[test]
    fn unsupported_pattern_parts_are_reported_in_order() {
        use UnsupportedHandling::{Literal, Quoted, ShorterFields};
        for (pattern, want) in [
            // Seen in Logseq issues #9542 and #9886.
            ("yyyy-MM-dd 'w'ww-E", vec![("'w'", Quoted), ("ww", Literal)]),
            ("yyyy-MM-dd-ww", vec![("ww", Literal)]),
            // FFF is illegal in cljs-time (#8570); dddd splits (#11429).
            ("EEE,FFF do MMM yyyy", vec![("FFF", Literal)]),
            ("yyyy/MM/dddd", vec![("dddd", ShorterFields)]),
            // cljs-time reads YYYY as the year and Do as the day ordinal.
            ("YYYY-MM-DD", vec![("YYYY", Literal), ("DD", Literal)]),
            ("MMMM Do, yyyy", vec![("D", Literal), ("o", Literal)]),
            // `o` is an ordinal only straight after a single `d` in Tine.
            ("ddo MMM", vec![("o", Literal)]),
            (
                "yyy MMMMM EEEEE",
                vec![
                    ("yyy", ShorterFields),
                    ("MMMMM", ShorterFields),
                    ("EEEEE", ShorterFields),
                ],
            ),
            (
                "h:mm a",
                vec![("h", Literal), ("mm", Literal), ("a", Literal)],
            ),
            ("''yyyy", vec![("''", Quoted)]),
            ("yyyy 'unterminated", vec![("'unterminated", Quoted)]),
        ] {
            let got: Vec<(String, UnsupportedHandling)> = unsupported_tokens(pattern)
                .into_iter()
                .map(|token| (token.text, token.handling))
                .collect();
            let want: Vec<(String, UnsupportedHandling)> = want
                .into_iter()
                .map(|(text, handling)| (text.to_string(), handling))
                .collect();
            assert_eq!(got, want, "{pattern}");
        }
    }

    /// Every default and OG preset pattern, and every pattern Tine's own tests
    /// round-trip, reports nothing.
    #[test]
    fn supported_patterns_report_nothing() {
        let og: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/og-journal-formats.json"
        ))
        .unwrap();
        let presets = og["formats"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["pattern"].as_str().unwrap());
        for pattern in presets.chain([
            DEFAULT_FILE_FORMAT,
            DEFAULT_TITLE_FORMAT,
            "yyyy-MM-dd",
            "dd-MM-yyyy",
            "yyyy.MM.dd",
            "EE E yyyy-MM-dd",
            "M/d/y",
            "yy-M-d",
            "EEEE, MMMM do, yyyy",
            "EEE do, MMM yyyy",
            "yyyy年MM月dd日",
            "",
        ]) {
            assert_eq!(unsupported_tokens(pattern), vec![], "{pattern:?}");
        }
    }

    /// Journal page identity (title, file stem, `is_canonical_stem`, parse) for
    /// the default and every OG preset is byte-identical to the capture made at
    /// bb0c6cccb, before detection existed.
    #[test]
    fn journal_identity_matches_pre_detection_capture() {
        let golden: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/journal-identity-golden.json"
        ))
        .unwrap();
        let rows = golden["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 689);
        for row in rows {
            let pattern = row["pattern"].as_str();
            let fmt = match row["role"].as_str().unwrap() {
                "title" => JournalFormat::new(None, pattern),
                _ => JournalFormat::new(pattern, None),
            };
            let date = JournalDate::from_ordinal(row["date"].as_i64().unwrap());
            let title = fmt.title(date);
            let stem = fmt.file_stem(date);
            let got = serde_json::json!({
                "pattern": pattern,
                "role": row["role"],
                "date": date.ordinal_key(),
                "title": title,
                "stem": stem,
                "canonical": fmt.is_canonical_stem(&stem),
                "parse_title": fmt.parse_text(&title).map(|d| d.ordinal_key()),
                "parse_stem": fmt.parse_file_stem(&stem).map(|d| d.ordinal_key()),
            });
            assert_eq!(&got, row);
        }
    }

    #[test]
    fn i12_journal_title_fixture_matches_rust() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/i12-journal-title-golden.json"
        ))
        .unwrap();
        let fmt = JournalFormat::default();
        for case in fixture["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            assert_eq!(
                fmt.parse_text(name).is_some(),
                case["expected"].as_bool().unwrap(),
                "{name}"
            );
        }
    }

    /// D-20261102-JDL: the text routes include the configured file-name
    /// format; every spelling they render for a day parses back to that day,
    /// and the unknown-name link rule is that same parse.
    #[test]
    fn spellings_parse_back_and_the_file_name_format_counts() {
        let fmt = JournalFormat::new(Some("dd-MM-yyyy"), Some("EEEE, dd.MM.yyyy"));
        let day = d(2026, 10, 22);
        let spellings = fmt.spellings(day);
        for expected in ["22-10-2026", "Oct 22nd, 2026", "2026-10-22", "2026_10_22"] {
            assert!(
                spellings.iter().any(|s| s == expected),
                "{expected} in {spellings:?}"
            );
        }
        let mut unique = spellings.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), spellings.len(), "deduplicated: {spellings:?}");
        for spelling in &spellings {
            assert_eq!(fmt.parse_text(spelling), Some(day), "{spelling}");
            assert!(fmt.absent_link_is_journal(spelling), "{spelling}");
        }
        assert!(!fmt.absent_link_is_journal("Launch"));
        // The prefilter never rejects a spelling the parser accepts, padded,
        // lenient or two-digit-year alike.
        let lenient = JournalFormat::new(Some("d.M.yy"), None);
        for (format, spelling, day) in [
            (&fmt, "October 22nd, 2026", d(2026, 10, 22)),
            (&fmt, "2026-10-22", d(2026, 10, 22)),
            (&lenient, "5.1.5", d(2005, 1, 5)),
            (&lenient, "05.01.05", d(2005, 1, 5)),
            (&fmt, "2010-10-10", d(2010, 10, 10)),
            (&lenient, "1.1.0", d(2000, 1, 1)),
        ] {
            assert_eq!(format.parse_text(spelling), Some(day), "{spelling}");
            assert_eq!(format.parse_file_stem(spelling), Some(day), "{spelling}");
            assert!(JournalFormat::may_spell_day(spelling, day), "{spelling}");
        }
        assert!(!JournalFormat::may_spell_day("Oct 23rd, 2026", day));
        assert!(!JournalFormat::may_spell_day("Project 22", day));
        assert!(!fmt.absent_link_is_journal("22-10"));
        // The writer's name is the configured one only; the ISO stem is
        // canonical (never a stray) but not the written name.
        assert!(fmt.writes_stem(day, "22-10-2026"));
        assert!(!fmt.writes_stem(day, "2026_10_22"));
        assert!(fmt.is_canonical_stem("2026_10_22"));
        assert!(fmt.is_canonical_stem("22-10-2026"));
        assert!(!fmt.is_canonical_stem("22-10-2026 copy"));
    }

    /// GH #863 D3: the stems a day's file may carry include every numeric
    /// width the parser accepts (default settings read `2026-11-1` as
    /// November 1), each parses back to that day, and the list is bounded.
    #[test]
    fn accepted_stems_cover_the_parsers_numeric_widths() {
        let fmt = JournalFormat::default();
        let day = d(2026, 11, 1);
        let stems = fmt.accepted_stems(day);
        for expected in [
            "2026_11_01",
            "2026_11_1",
            "2026-11-01",
            "2026-11-1",
            "Nov 1st, 2026",
            "Nov 01st, 2026",
        ] {
            assert!(
                stems.iter().any(|s| s == expected),
                "{expected} in {stems:?}"
            );
        }
        // The file-stem routes' own renderings first.
        assert_eq!(
            stems[..3],
            ["2026_11_01", "Nov 1st, 2026", "2026-11-01"],
            "renderings first"
        );
        let mut unique = stems.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), stems.len(), "deduplicated: {stems:?}");
        for stem in &stems {
            assert_eq!(fmt.parse_file_stem(stem), Some(day), "{stem}");
        }
        assert_eq!(stems.len(), 6, "{stems:?}");
        // Bounded: one-digit month and day double each numeric pattern.
        assert_eq!(fmt.accepted_stems(d(2026, 1, 1)).len(), 10);
        assert_eq!(fmt.accepted_stems(d(2026, 11, 22)).len(), 3);
        // A two-digit year reads 2000 + digits: no other century is listed.
        let short = JournalFormat::new(Some("d.M.yy"), Some("d.M.yy"));
        let stems = short.accepted_stems(d(2005, 1, 5));
        assert!(stems.iter().any(|s| s == "5.1.5"), "{stems:?}");
        assert!(stems.iter().any(|s| s == "05.01.05"), "{stems:?}");
        assert!(short
            .accepted_stems(d(1999, 1, 5))
            .iter()
            .all(|s| !s.contains(".99")));
    }

    #[test]
    fn accepted_stems_stay_bounded_when_a_format_repeats_numeric_fields() {
        // Merge review r2 (GH #863): "yyyy-MM-dd" + " M" x 14 made 65,546 stems
        // for January 1 and ~196k stats under the writer, growing exponentially.
        let repeated = format!("yyyy-MM-dd{}", " M".repeat(14));
        let fmt = JournalFormat::new(Some(&repeated), Some("MMM do, yyyy"));
        let day = d(2026, 1, 1);
        let stems = fmt.accepted_stems(day);
        let bound = fmt.spellings(day).len() + 2 * 16; // two patterns, WIDTH_RENDERINGS_MAX each
        assert!(stems.len() <= bound, "{} stems", stems.len());
        // The pattern's ordinary spelling is still tried.
        assert!(
            stems.iter().any(|s| s.starts_with("2026-01-01 1 1")),
            "{stems:?}"
        );
    }
}

#[cfg(test)]
mod title_check_tests {
    //! D-20261119-YL: a configured title format is used only if every title
    //! it writes names its day alone; [`title_refusal`] decides it exactly.
    use super::*;

    fn d(year: i32, month: u32, day: u32) -> JournalDate {
        JournalDate { year, month, day }
    }

    /// The exhaustive law, day by day over [`JOURNAL_YEARS`]: the reference
    /// the class analysis must agree with. The pattern's own round trip
    /// (`format.parse(key(format.render(d))) = d`), and, when it holds, the
    /// first text route reads the title back the same way.
    fn names_every_day(file: &str, title: &str) -> bool {
        let fmt = JournalFormat::new(Some(file), Some(title));
        let configured = Format::compile(title);
        let own = |day: JournalDate| configured.parse(configured.format(day).trim()) == Some(day);
        let (start, end) = (
            d(*JOURNAL_YEARS.start(), 1, 1).to_days(),
            d(*JOURNAL_YEARS.end(), 12, 31).to_days(),
        );
        (start..=end).all(|z| {
            let day = JournalDate::from_days(z);
            own(day) && fmt.parse_text(&configured.format(day)) == Some(day)
        })
    }

    fn verdict(file: &str, title: &str) -> Option<TitleFormatRefusal> {
        JournalFormat::new(Some(file), Some(title))
            .title_refusal()
            .cloned()
    }

    /// Text lookup parses a spelling itself, as the title is written, so a
    /// literal's case is the title's (`X 2026_22_10` names its day).
    #[test]
    fn a_title_is_read_as_written() {
        assert_eq!(verdict("dd.MM.yyyy", "X yyyy_dd_MM"), None);
        let fmt = JournalFormat::new(Some("dd.MM.yyyy"), Some("X yyyy_dd_MM"));
        assert_eq!(fmt.title(d(2026, 10, 22)), "X 2026_22_10");
        assert!(names_every_day("dd.MM.yyyy", "X yyyy_dd_MM"));
        // Literal text and slashes are read as written too.
        assert_eq!(verdict("yyyy_MM_dd", "'Day' dd.MM.yyyy"), None);
        assert_eq!(verdict("yyyy_MM_dd", "/yyyy/MM/dd/"), None);
    }

    #[test]
    fn a_title_without_a_year_is_refused_and_titles_use_the_default() {
        let fmt = JournalFormat::new(None, Some("MM-dd"));
        assert_eq!(
            fmt.title_refusal(),
            Some(&TitleFormatRefusal::Missing(DateField::Year))
        );
        assert_eq!(fmt.title_format(), DEFAULT_TITLE_FORMAT);
        assert_eq!(fmt.configured_title_format(), "MM-dd");
        assert_eq!(fmt.title(d(2026, 11, 2)), "Nov 2nd, 2026");
        assert_eq!(fmt.title(d(2027, 11, 2)), "Nov 2nd, 2027");
        // `11-02` names no day: no day's spelling, and it parses as none.
        assert!(!fmt.spellings(d(2026, 11, 2)).iter().any(|s| s == "11-02"));
        assert_eq!(fmt.parse_text("11-02"), None);
        assert_eq!(
            verdict("yyyy_MM_dd", "MMMM yyyy"),
            Some(TitleFormatRefusal::Missing(DateField::Day))
        );
        assert_eq!(
            verdict("yyyy_MM_dd", "do yyyy"),
            Some(TitleFormatRefusal::Missing(DateField::Month))
        );
    }

    #[test]
    fn full_day_patterns_are_used() {
        for title in [
            "yyyy-MM-dd",
            "dd.MM.yyyy",
            "MM/dd/yyyy",
            "EEEE, MMMM d, yyyy",
            "EEE, dd MMM yyyy",
            "yyyyMMdd",
            "do MMMM y",
            "MMMM do, yyyy",
        ] {
            let fmt = JournalFormat::new(None, Some(title));
            assert_eq!(fmt.title_refusal(), None, "{title}");
            assert_eq!(fmt.title_format(), title);
        }
        assert_eq!(
            JournalFormat::new(None, Some("dd.MM.yyyy")).title(d(2026, 11, 2)),
            "02.11.2026"
        );
    }

    /// OG's title choices (`journal-title-formatters`), as the fixture
    /// transcribes them.
    fn og_title_patterns() -> Vec<String> {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/og-journal-formats.json"
        ))
        .unwrap();
        (fixture["formats"].as_array().unwrap().iter())
            .map(|row| row["pattern"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn every_og_title_choice_is_used_and_the_default_names_one_day_beside_each() {
        for title in og_title_patterns() {
            assert_eq!(verdict(DEFAULT_FILE_FORMAT, &title), None, "{title}");
        }
        // The fallback names one day on its own (the default is never checked
        // at run time), and as the first text route no file-name format can
        // read its titles otherwise: every OG choice as the file-name format.
        assert_eq!(title_refusal(&Format::compile(DEFAULT_TITLE_FORMAT)), None);
        for file in og_title_patterns() {
            let fmt = JournalFormat::new(Some(&file), None);
            for day in [d(2026, 11, 2), d(1999, 2, 28), d(2024, 2, 29), d(5, 12, 31)] {
                assert_eq!(fmt.parse_text(&fmt.title(day)), Some(day), "file {file}");
            }
        }
    }

    #[test]
    fn a_two_digit_year_is_refused_with_a_day_it_misnames() {
        let refusal = verdict("yyyy_MM_dd", "d.M.yy").expect("yy names only one century");
        let TitleFormatRefusal::Misread {
            day,
            written,
            read_as,
        } = refusal
        else {
            panic!("{refusal:?}");
        };
        assert_eq!(day.year, 1900);
        assert_eq!(written, "1.1.00");
        assert_eq!(read_as, Some(d(2000, 1, 1)));
    }

    #[test]
    fn a_title_its_own_parser_misreads_is_refused() {
        // Distinct renderings, yet one the parser does not read back:
        // `190011` is 1900-01-01, read as month 11 with no day.
        assert_eq!(
            verdict("yyyy_MM_dd", "yyyyMd"),
            Some(TitleFormatRefusal::Misread {
                day: d(1900, 1, 1),
                written: "190011".to_owned(),
                read_as: None
            })
        );
    }

    #[test]
    fn leap_days_and_the_year_range_edges_are_checked() {
        // `yyyy` writes `0000`..`9999`; 29 February of year 0 and 9996 parse back.
        let fmt = JournalFormat::new(None, Some("yyyy-MM-dd"));
        for day in [d(0, 2, 29), d(9996, 2, 29), d(0, 1, 1), d(9999, 12, 31)] {
            assert_eq!(fmt.parse_text(&fmt.title(day)), Some(day));
        }
        assert_eq!(verdict("yyyy_MM_dd", "y-M-d"), None);
    }

    #[test]
    fn the_class_analysis_agrees_with_the_exhaustive_law() {
        // Including a file format whose digit literal meets the title's year
        // digits, so the reading depends on their value and the analysis
        // checks that class day by day. A used pattern costs the reference a
        // full sweep, so two of them.
        for (file, title) in [
            ("yyyy_MM_dd", "MM-dd"),
            ("yyyy_MM_dd", "d.M.yy"),
            ("MM-dd-yyyy", "dd-MM-yyyy"),
            ("yyyy_MM_dd", "dMyyyy"),
            ("yyyy_MM_dd", "EEE d MMM y"),
            ("1MM-dd-yyyy", "yyyy-MM-dd"),
            ("yyyy_MM_dd", "MMd2yyyy"),
        ] {
            assert_eq!(
                verdict(file, title).is_none(),
                names_every_day(file, title),
                "{file} / {title}"
            );
        }
    }

    /// D-20261119-YL model round 3 (review finding 1): text tries the effective
    /// title first, then the file-name format, then the fixed forms; a refused
    /// title is no text route. A file's stem keeps the old order (round 4).
    #[test]
    fn text_reads_the_effective_title_first_and_a_refused_title_not_at_all() {
        let nov2 = d(2026, 11, 2);
        // The refused `MMM do, ddyy` reads `Nov 2nd, 2026` as November 20.
        let refused = JournalFormat::new(None, Some("MMM do, ddyy"));
        assert!(refused.title_refusal().is_some());
        assert_eq!(refused.title(nov2), "Nov 2nd, 2026");
        assert_eq!(refused.parse_text("Nov 2nd, 2026"), Some(nov2));
        assert_eq!(
            refused.parse_file_stem("Nov 2nd, 2026"),
            Some(d(2026, 11, 20))
        );
        // The same pattern as the file-name format, with the title absent,
        // explicitly default, or refused: the default title reads first.
        for title in [
            None,
            Some(DEFAULT_TITLE_FORMAT),
            Some("MM-dd"),
            Some("MMM do, ddyy"),
        ] {
            let fmt = JournalFormat::new(Some("MMM do, ddyy"), title);
            assert_eq!(fmt.title_format(), DEFAULT_TITLE_FORMAT, "{title:?}");
            assert_eq!(fmt.parse_text("Nov 2nd, 2026"), Some(nov2), "{title:?}");
            assert_eq!(
                fmt.spellings(nov2).first().map(String::as_str),
                Some("Nov 2nd, 2026")
            );
        }
        // A refused title that was a date's only route: typed text is an
        // ordinary name, the legacy file keeps its day and is found.
        let short = JournalFormat::new(None, Some("d.M.yy"));
        assert!(short.title_refusal().is_some());
        assert_eq!(short.parse_text("2.11.26"), None);
        assert!(!short.absent_link_is_journal("2.11.26"));
        assert_eq!(short.parse_file_stem("2.11.26"), Some(nov2));
        assert!(short.accepted_stems(nov2).iter().any(|s| s == "2.11.26"));
        assert!(!short.spellings(nov2).iter().any(|s| s == "2.11.26"));
        // The same pattern as the file-name format stays a text route.
        let as_file = JournalFormat::new(Some("d.M.yy"), None);
        assert_eq!(as_file.parse_text("2.11.26"), Some(nov2));
        // An accepted numeric title swapped against the file-name format:
        // typed text follows the title, the file `2026_11_02` stays November 2.
        let swapped = JournalFormat::new(Some("yyyy_MM_dd"), Some("yyyy_dd_MM"));
        assert_eq!(swapped.title_refusal(), None);
        assert_eq!(swapped.parse_text("2026_11_02"), Some(d(2026, 2, 11)));
        assert_eq!(swapped.parse_file_stem("2026_11_02"), Some(nov2));
        assert_eq!(swapped.title(nov2), "2026_02_11");
        assert_eq!(swapped.parse_text(&swapped.title(nov2)), Some(nov2));
        assert!(swapped
            .accepted_stems(nov2)
            .iter()
            .any(|s| s == "2026_11_02"));
        // Refused before, when the file-name format read its titles first.
        assert_eq!(verdict("MM-dd-yyyy", "dd-MM-yyyy"), None);
    }

    /// Review finding 5: the check reads the title pattern alone, one render
    /// and one read per class, each one pass over the tokens. Was 0.95 s for
    /// the `2!` pairs (a file-name digit literal against the title's year) and
    /// 0.74 s for 1,000 `-dd`.
    #[test]
    fn the_title_check_costs_one_read_per_class_whatever_the_pattern() {
        let repeated = format!("yyyy-MM{}", "-dd".repeat(1000));
        for (title, refused) in [
            ("2!yyyy-MM-dd", false),
            ("yyyy-MM-dd", false),
            ("2!yyyy_MM_dd", false),
            ("yyyy_MM_dd", false),
            (repeated.as_str(), false),
            // A digit literal the reading meets inside the year.
            ("MMd2yyyy", true),
        ] {
            let format = Format::compile(title);
            VALIDATOR_READS.with(|reads| reads.set(0));
            let started = std::time::Instant::now();
            let verdict = title_refusal(&format);
            let elapsed = started.elapsed();
            let reads = VALIDATOR_READS.with(|reads| reads.get());
            let name = &title[..title.len().min(12)];
            assert_eq!(verdict.is_some(), refused, "{name}: {verdict:?}");
            // One shape class for these padded patterns, whatever their length.
            assert!(reads <= 4, "{name}: {reads} reads");
            // Native release about 0.1 ms; a debug build is slower.
            let budget = if cfg!(debug_assertions) { 100 } else { 5 };
            assert!(elapsed.as_millis() < budget, "{name}: {elapsed:?}");
        }
        // The file-name format no longer enters: these pairs share one verdict.
        for file in ["2!yyyy-MM-dd", "1MM-dd-yyyy", "yyyy-MM-dd", "MMM do, ddyy"] {
            assert_eq!(verdict(file, "yyyy-MM-dd"), None, "{file}");
        }
    }

    /// D-20261119-YL, review round 3: the frontend's text routes are built
    /// from the effective format the native core published, so building and
    /// reading them checks no pattern, whatever its length (a 1,000-weekday
    /// title took 1.8 s per cold check in wasm when the window checked it).
    #[test]
    fn text_routes_from_a_published_effective_format_check_no_pattern() {
        let long = format!("{}EEEE MMMM do y", "EEEE ".repeat(1000));
        for (file, effective) in [
            (None, None),
            (Some("d.M.yy"), Some("MMM do, yyyy")),
            (None, Some(long.as_str())),
        ] {
            VALIDATOR_READS.with(|reads| reads.set(0));
            let routes = TextRoutes::new(file, effective);
            let full = JournalFormat::new(file, effective);
            for text in ["Nov 2nd, 2026", "2026-11-02", "2.11.26", "Notes"] {
                assert_eq!(routes.parse(text), full.parse_text(text), "{text}");
                assert_eq!(
                    routes.absent_link_is_journal(text),
                    full.absent_link_is_journal(text)
                );
            }
            VALIDATOR_READS.with(|reads| reads.set(0));
            let routes = TextRoutes::new(file, effective);
            let _ = routes.parse("Monday Monday November 2nd 2026");
            assert_eq!(VALIDATOR_READS.with(|reads| reads.get()), 0);
        }
    }

    /// Settings offers only its presets (`src/journalTitleFormats.ts`) and
    /// shows them without asking which are used: every one is, and the
    /// verdict depends on the title pattern alone (`checked_title`), so under
    /// every file-name format too.
    #[test]
    fn every_settings_title_preset_is_used() {
        let source = include_str!("../../../src/journalTitleFormats.ts");
        let list = source
            .split_once("JOURNAL_TITLE_FORMATS = [")
            .and_then(|(_, rest)| rest.split_once(']'))
            .map(|(list, _)| list)
            .expect("the preset list");
        let presets: Vec<&str> = list.split('"').skip(1).step_by(2).collect();
        assert!(presets.len() >= 20, "{presets:?}");
        for preset in presets {
            let format = JournalFormat::new(None, Some(preset));
            assert_eq!(format.title_refusal(), None, "{preset}");
            assert_eq!(format.title_format(), preset);
        }
    }

    /// The shape classes decide exactly what deciding every month and day on
    /// its own decides (that reference agrees with the exhaustive law above).
    #[test]
    fn shape_classes_agree_with_deciding_each_month_and_day() {
        let mut titles = og_title_patterns();
        titles.extend(
            [
                "d.M.yyyy",
                "M/d/yyyy",
                "do MMM yyyy",
                "MMMM d, yyyy",
                "EEE, d MMM y",
                "yyyyMMdd",
                "yyyyMd",
                "dMyyyy",
                "Mdyyyy",
                "MMd2yyyy",
                "d1M-yyyy",
                "MM1dd-yyyy",
                "yyyy-M-d",
                "y-M-d",
                "dd.MM.yy",
                "d MMMM yyyy EEEE",
                "1MM-dd-yyyy",
                "dd-MM-yyyy dd",
                "MM dd yyyy MM",
                "do do MMM yyyy",
                "d-yyyy-M",
                "yMd",
                "y M d",
                "MMMMd yyyy",
                "MMMdo yyyy",
                "EEEEd.M.yyyy",
                "dd MM 12 yyyy",
                " yyyy-MM-dd ",
                "yyyy-MM-dd yy",
                "yy yyyy-MM-dd",
                "MM-dd-yyyy-d",
                "d-MM-yyyy-dd",
                "2!yyyy-MM-dd",
                "MMM do, ddyy",
            ]
            .map(String::from),
        );
        for title in titles {
            let format = Format::compile(&title);
            let (by_shape, each) = (
                title_refusal_by(&format, true),
                title_refusal_by(&format, false),
            );
            assert_eq!(
                by_shape.is_none(),
                each.is_none(),
                "{title}: {by_shape:?} / {each:?}"
            );
            if let Some(TitleFormatRefusal::Misread {
                day,
                written,
                read_as,
            }) = by_shape
            {
                assert_eq!(format.format(day), written, "{title}");
                assert_eq!(format.parse(written.trim()), read_as, "{title}");
                assert_ne!(read_as, Some(day), "{title}");
            }
        }
    }
}
