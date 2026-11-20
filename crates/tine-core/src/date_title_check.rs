//! Exact admission of folded journal text routes (D-20261119-YL and A-20261120-DK).

use super::*;

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
pub(super) fn checked_title(title_pat: &str, title: &Format) -> Option<TitleFormatRefusal> {
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
pub(super) fn title_refusal(title: &Format) -> Option<TitleFormatRefusal> {
    title_refusal_by(title, true)
}

/// [`title_refusal`]; `by_shape` false skips the shape classes and decides
/// every month and day on its own (the reference the shape classes must agree
/// with, in tests).
pub(super) fn title_refusal_by(title: &Format, by_shape: bool) -> Option<TitleFormatRefusal> {
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
    let reader = title.folded();
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
        let read_as = reader.parse(&page_key(&written));
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
        match class_verdict(title, &reader, date(rep), &mut buffer) {
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
                    .then(|| shape_verdict(title, &reader, rep, pairs, &mut shape_buffer))
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
    pub(super) static VALIDATOR_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_validator_read() {
    VALIDATOR_READS.with(|reads| reads.set(reads.get() + 1));
}

/// The representative's folded title and the positions of selected digits.
/// Key normalization preserves every ASCII digit in order. Month names,
/// weekdays and ordinal suffixes are fixed in each class, so composition
/// and contextual case changes across a literal boundary are fixed too.
fn render_for_read(
    title: &Format,
    rep: JournalDate,
    buffer: &mut Vec<char>,
    slot: impl Fn(&Tok) -> bool,
) -> Vec<(std::ops::Range<usize>, Tok)> {
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
    let key = page_key(&buffer.iter().collect::<String>());
    let mut digits = key.chars().enumerate().filter(|(_, c)| c.is_ascii_digit());
    let mut start = 0;
    for (range, _) in &mut slots {
        let skip = buffer[start..range.start]
            .iter()
            .filter(|c| c.is_ascii_digit())
            .count();
        let first = digits.nth(skip).expect("key preserves digits").0;
        let len = range.len();
        start = range.end;
        if len > 1 {
            digits.nth(len - 2).expect("key preserves digits");
        }
        *range = first..first + len;
    }
    buffer.clear();
    buffer.extend(key.chars());
    slots
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
    reader: &Format,
    rep: JournalDate,
    pairs: &[(u32, u32)],
    buffer: &mut Vec<char>,
) -> ShapeVerdict {
    #[cfg(test)]
    count_validator_read();
    let numeric = |t: &Tok| !matches!(t, Tok::Lit(_) | Tok::Weekday(_) | Tok::MonthName(_));
    let slots = render_for_read(title, rep, buffer, numeric);
    let mut trace = Trace {
        slots: &slots,
        dependent: None,
        spans: [None, None, None],
        whole: false,
    };
    let got = reader.read(buffer, &mut trace);
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
fn class_verdict(
    title: &Format,
    reader: &Format,
    rep: JournalDate,
    buffer: &mut Vec<char>,
) -> Verdict {
    #[cfg(test)]
    count_validator_read();
    let year_slots = render_for_read(title, rep, buffer, |t| matches!(t, Tok::Year(_)));
    let mut trace = Trace {
        slots: &year_slots,
        dependent: None,
        spans: [None, None, None],
        whole: false,
    };
    let got = reader.read(buffer, &mut trace);
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
