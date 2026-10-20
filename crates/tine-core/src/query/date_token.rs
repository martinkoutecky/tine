//! The date-token grammar every query dialect shares: `today`, `yesterday`,
//! `tomorrow`, a `yyyy-mm-dd` stem, a journal title, a relative offset
//! (`-7d`, `+2w`), and the timestamp bounds (`now`, `-2h`, `+90n`), each
//! resolved against one execution day. Split from `advanced_patterns.rs`
//! along the grammar/lowering seam.

use crate::date::JournalDate;

/// The largest relative offset a date token may carry, in years (and the
/// same span in days, weeks and months). Within it every resolved date keeps a
/// monotone `yyyymmdd` ordinal; beyond it the token is [`DateToken::OutOfRange`].
pub const MAX_DATE_OFFSET_YEARS: i64 = 10_000;

/// The unit of a relative date token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateUnit {
    /// `d`
    Day,
    /// `w`
    Week,
    /// `m`
    Month,
    /// `y`
    Year,
}

/// A date-literal token (a `between` bound, a TQL date, an advanced input),
/// read WITHOUT `today`. The ONE date-token grammar: the resolver
/// ([`resolve_date_token`]) and the OG printer's bare-or-`[[ ]]` choice both
/// read it (Rule 3), and the parsers diagnose [`DateToken::OutOfRange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateToken {
    /// `today`/`now` (0), `yesterday` (−1), `tomorrow` (+1), any case: a day
    /// offset.
    Keyword(i64),
    /// `[+|-]N[dwmy]`: ASCII digits, a lowercase unit (`-7D` is not one).
    Relative {
        /// The signed count of units.
        n: i64,
        /// The unit.
        unit: DateUnit,
    },
    /// A relative token past ±[`MAX_DATE_OFFSET_YEARS`] years (or past `i64`):
    /// never resolves; parsers report it instead of answering a garbage day.
    OutOfRange,
    /// A `yyyy_MM_dd` or `yyyy-MM-dd` stem (zero padding optional), calendar
    /// valid.
    Stem(JournalDate),
    /// A journal title in the default `MMM do, yyyy` format. The OG printer
    /// writes it inside `[[ ]]`; every other shape prints bare.
    Title(JournalDate),
}

impl DateToken {
    /// Read `tok` (trimmed). `None`: not a date token.
    pub fn parse(tok: &str) -> Option<DateToken> {
        let t = tok.trim();
        for (keyword, offset) in [("today", 0), ("now", 0), ("yesterday", -1), ("tomorrow", 1)] {
            if t.eq_ignore_ascii_case(keyword) {
                return Some(DateToken::Keyword(offset));
            }
        }
        if let Some(token) = Self::relative(t) {
            return Some(token);
        }
        if let Some(date) = JournalDate::from_file_stem(t) {
            return Some(DateToken::Stem(date));
        }
        // master `journal_ordinal`: a default-format journal title literal.
        JournalDate::from_title(t).map(DateToken::Title)
    }

    fn relative(t: &str) -> Option<DateToken> {
        let (sign, rest) = match t.as_bytes().first()? {
            b'+' => (1i64, &t[1..]),
            b'-' => (-1i64, &t[1..]),
            _ => (1i64, t),
        };
        let unit = match rest.as_bytes().last()? {
            b'd' => DateUnit::Day,
            b'w' => DateUnit::Week,
            b'm' => DateUnit::Month,
            b'y' => DateUnit::Year,
            _ => return None,
        };
        let digits = &rest[..rest.len() - 1];
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let per_year = match unit {
            DateUnit::Day => 366,
            DateUnit::Week => 53,
            DateUnit::Month => 12,
            DateUnit::Year => 1,
        };
        match digits.parse::<i64>() {
            Ok(n) if n <= MAX_DATE_OFFSET_YEARS * per_year => {
                Some(DateToken::Relative { n: sign * n, unit })
            }
            _ => Some(DateToken::OutOfRange),
        }
    }

    /// The `yyyymmdd` ordinal this token names on `today`. Offsets are bounded
    /// at parse, so the arithmetic cannot overflow.
    pub fn resolve(self, today: JournalDate) -> Option<i64> {
        let date = match self {
            DateToken::Keyword(offset) => today.add_days(offset),
            DateToken::Relative { n, unit } => match unit {
                DateUnit::Day => today.add_days(n),
                DateUnit::Week => today.add_days(n * 7),
                DateUnit::Month => today.add_months(n),
                DateUnit::Year => today.add_months(n * 12),
            },
            DateToken::OutOfRange => return None,
            DateToken::Stem(date) | DateToken::Title(date) => date,
        };
        Some(date.ordinal_key())
    }

    /// Whether the day this token names moves with `today` (a keyword or a
    /// relative offset), so an answer that resolved it is pinned to its
    /// execution day (card hB3Lv8). A stem or title names a fixed day.
    pub fn reads_today(self) -> bool {
        matches!(self, DateToken::Keyword(_) | DateToken::Relative { .. })
    }

    /// Whether the OG printer may write this token bare (a title needs `[[ ]]`).
    pub fn prints_bare(self) -> bool {
        !matches!(self, DateToken::Title(_))
    }
}

/// Resolve a date-literal token to a `yyyymmdd` ordinal against `today`: the
/// [`DateToken`] grammar. `None` for a non-date or an out-of-range offset.
/// The ONE date-literal resolver: the advanced lowering and the executor's
/// date comparisons both call it (I-12).
pub fn resolve_date_token(tok: &str, today: JournalDate) -> Option<i64> {
    DateToken::parse(tok)?.resolve(today)
}

/// Whether a `Value::Date` literal resolves against the execution day: a
/// relative date token, or a timestamp bound (`now`, `-2h`) that is anchored at
/// it. An answer that reads one is stale once the local date changes (card
/// hB3Lv8); one that reads none is not. O(token length).
pub fn date_literal_reads_today(tok: &str) -> bool {
    let tok = tok.trim();
    clock_offset_ms(tok).is_some() || DateToken::parse(tok).is_some_and(DateToken::reads_today)
}

/// Milliseconds in one day.
const MS_PER_DAY: i64 = 86_400_000;

/// The largest `h`/`n` offset a timestamp bound may carry. Hours and minutes
/// beyond this span (about 10 000 years) read as no bound, exactly as an
/// out-of-range day offset does, so the arithmetic cannot overflow.
const MAX_CLOCK_OFFSET: i64 = MAX_DATE_OFFSET_YEARS * 366 * 24 * 60;

/// UTC midnight of `date`, in epoch milliseconds (OG `tc/to-long (t/today)`).
fn midnight_ms(date: JournalDate) -> i64 {
    date.to_days() * MS_PER_DAY
}

/// A `[+|-]N` count followed by the `h` (hours) or `n` (minutes) unit letter:
/// the two units only a TIMESTAMP bound has. `None` for any other shape.
fn clock_offset_ms(token: &str) -> Option<Option<i64>> {
    let (sign, rest) = match token.as_bytes().first()? {
        b'+' => (1i64, &token[1..]),
        b'-' => (-1i64, &token[1..]),
        _ => (1i64, token),
    };
    let per_unit = match rest.as_bytes().last()? {
        b'h' => 3_600_000,
        b'n' => 60_000,
        _ => return None,
    };
    let digits = &rest[..rest.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(match digits.parse::<i64>() {
        Ok(n) if n <= MAX_CLOCK_OFFSET => Some(sign * n * per_unit),
        _ => None,
    })
}

/// Whether `tok` is a bound of a `created_at` / `last_modified_at` range:
/// everything [`DateToken`] reads, plus `now` and the `h`/`n` clock offsets.
pub fn is_timestamp_token(tok: &str) -> bool {
    let tok = tok.trim();
    match clock_offset_ms(tok) {
        Some(offset) => offset.is_some(),
        None => matches!(DateToken::parse(tok), Some(token) if token != DateToken::OutOfRange),
    }
}

/// Resolve a `created_at` / `last_modified_at` bound to epoch milliseconds,
/// OG `->timestamp` (query_dsl.cljs:81-113): `now` is the instant `now_ms`;
/// every other keyword, a journal title or stem, and every signed offset is
/// anchored at UTC midnight of `today` (`d`/`w`/`m`/`y` move whole days, `h`
/// and `n` add hours and minutes to that midnight, NOT to the current time).
/// `None` when the token is not a timestamp bound.
pub fn resolve_timestamp_token(tok: &str, today: JournalDate, now_ms: i64) -> Option<i64> {
    let tok = tok.trim();
    if tok.eq_ignore_ascii_case("now") {
        return Some(now_ms);
    }
    if let Some(offset) = clock_offset_ms(tok) {
        return offset.map(|offset| midnight_ms(today) + offset);
    }
    let day = DateToken::parse(tok)?.resolve(today)?;
    Some(midnight_ms(JournalDate::from_ordinal(day)))
}

#[cfg(test)]
mod date_token_tests {
    use super::*;

    /// Reader B (og 14 Q2): an offset past the admitted range neither panics
    /// (debug overflow in `to_days() + n`, `n * 7`, `n * 12`) nor resolves to a
    /// garbage day in release; it does not resolve at all.
    #[test]
    fn an_out_of_range_offset_resolves_to_nothing_and_never_panics() {
        let today = JournalDate::from_ordinal(20260904);
        for token in [
            "+9223372036854775807d",
            "-9223372036854775807w",
            "1537228672809129302m",
            "-768614336404564651y",
            "+99999999999999999999999d",
            "-99999y",
        ] {
            assert_eq!(
                DateToken::parse(token),
                Some(DateToken::OutOfRange),
                "{token}"
            );
            assert_eq!(resolve_date_token(token, today), None, "{token}");
        }
        assert!(
            resolve_date_token("-10000y", today).is_some(),
            "the admitted limit resolves"
        );
        assert_eq!(resolve_date_token("+2w", today), Some(20260918));
        assert_eq!(
            resolve_date_token("-7D", today),
            None,
            "units are lowercase"
        );
        assert_eq!(resolve_date_token("2026_01_05", today), Some(20260105));
    }

    /// The grammar golden shared with the frontend preview (`dateExpr.ts`,
    /// I-12): this resolver is authoritative and `src/editor/dateExpr.test.ts`
    /// reads the same file, so the TypeScript twin cannot drift from it.
    #[test]
    fn the_shared_date_token_golden_resolves_as_recorded() {
        let golden: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/i12-date-token-golden.json"
        ))
        .expect("golden parses");
        let mut mismatches = Vec::new();
        for case in golden["cases"].as_array().expect("cases") {
            let today = JournalDate::from_ordinal(case["today"].as_i64().expect("today"));
            let token = case["token"].as_str().expect("token");
            let expected = case["ordinal"].as_i64();
            let actual = resolve_date_token(token, today);
            if actual != expected {
                mismatches.push(format!(
                    "today {} token {token:?}: golden {expected:?}, resolver {actual:?}",
                    case["today"]
                ));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }
}
