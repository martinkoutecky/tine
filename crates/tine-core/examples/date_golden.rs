//! Regenerate with:
//! `source scripts/env.sh && cargo run --quiet -p tine-core --example date_golden > src/fixtures/date-golden.json`

use serde::Serialize;
use tine_core::date::{Format, JournalDate};

const README: &str = "Generated from Rust date.rs. Regenerate with `source scripts/env.sh && cargo run --quiet -p tine-core --example date_golden > src/fixtures/date-golden.json`. Rust Format is authoritative; both Rust and TS assert against this committed fixture. `do` parsing intentionally accepts mismatched st/nd/rd/th suffixes, matching OG Logseq's cljs-time internal/parse.cljs parse-ordinal-suffix, but a suffix must be present.";

#[derive(Serialize)]
struct Fixture {
    _readme: &'static str,
    format: Vec<FormatVector>,
    parse: Vec<ParseVector>,
}

#[derive(Serialize)]
struct FormatVector {
    fmt: &'static str,
    date: DateParts,
    title: String,
}

#[derive(Serialize)]
struct ParseVector {
    fmt: &'static str,
    input: String,
    date: Option<DateParts>,
}

#[derive(Clone, Copy, Serialize)]
struct DateParts {
    y: i32,
    m: u32,
    d: u32,
}

fn main() {
    let fixture = Fixture { _readme: README, format: format_vectors(), parse: parse_vectors() };
    println!("{}", serde_json::to_string_pretty(&fixture).expect("serialize date golden fixture"));
}

fn jd(y: i32, m: u32, d: u32) -> JournalDate {
    JournalDate { year: y, month: m, day: d }
}

fn parts(d: JournalDate) -> DateParts {
    DateParts { y: d.year, m: d.month, d: d.day }
}

fn format_vector(fmt: &'static str, date: JournalDate) -> FormatVector {
    FormatVector { fmt, date: parts(date), title: Format::compile(fmt).format(date) }
}

fn parse_vector(fmt: &'static str, input: impl Into<String>) -> ParseVector {
    let input = input.into();
    let date = Format::compile(fmt).parse(&input).map(parts);
    ParseVector { fmt, input, date }
}

fn format_vectors() -> Vec<FormatVector> {
    let mut vectors = vec![
        format_vector("MMM do, yyyy", jd(2026, 6, 1)),
        format_vector("MMM do, yyyy", jd(2026, 6, 2)),
        format_vector("MMM do, yyyy", jd(2026, 6, 3)),
        format_vector("MMM do, yyyy", jd(2026, 6, 4)),
        format_vector("MMM do, yyyy", jd(2026, 6, 11)),
        format_vector("MMM do, yyyy", jd(2026, 6, 12)),
        format_vector("MMM do, yyyy", jd(2026, 6, 13)),
        format_vector("MMM do, yyyy", jd(2026, 6, 21)),
        format_vector("MMM do, yyyy", jd(2026, 6, 22)),
        format_vector("MMM do, yyyy", jd(2026, 6, 23)),
        format_vector("MMM do, yyyy", jd(2026, 1, 31)),
        format_vector("yyyy_MM_dd", jd(2026, 6, 1)),
        format_vector("yyyy-MM-dd", jd(2026, 6, 1)),
        format_vector("yyyy/MM/dd", jd(2026, 6, 1)),
        format_vector("MM/dd/yyyy", jd(2026, 6, 1)),
        format_vector("M/d/y", jd(2026, 6, 1)),
        format_vector("yy-M-d", jd(2026, 6, 1)),
        format_vector("MMMM do, yyyy", jd(2026, 6, 1)),
        format_vector("do MMM yyyy", jd(2026, 6, 1)),
        format_vector("yyyy_MM_dd", jd(2025, 12, 31)),
        format_vector("yyyy_MM_dd", jd(2026, 1, 1)),
        format_vector("MMM do, yyyy", jd(2025, 12, 31)),
        format_vector("MMM do, yyyy", jd(2026, 1, 1)),
        format_vector("yyyy-MM-dd", jd(2024, 2, 29)),
        format_vector("EEEE, MMMM do, yyyy", jd(2024, 2, 29)),
        format_vector("EEE, yyyy/MM/dd", jd(2024, 1, 5)),
        format_vector("EE E yyyy-MM-dd", jd(2024, 1, 5)),
    ];

    for date in [
        jd(2024, 1, 7),
        jd(2024, 1, 8),
        jd(2024, 1, 9),
        jd(2024, 1, 10),
        jd(2024, 1, 11),
        jd(2024, 1, 12),
        jd(2024, 1, 13),
    ] {
        vectors.push(format_vector("EEEE EEE EE E yyyy-MM-dd", date));
    }

    vectors
}

fn parse_vectors() -> Vec<ParseVector> {
    let weekday_fmt = Format::compile("EEEE EEE EE E yyyy-MM-dd");
    let mut vectors = vec![
        parse_vector("MMM do, yyyy", "Jun 1st, 2026"),
        parse_vector("MMM do, yyyy", "Jun 1th, 2026"),
        parse_vector("MMM do, yyyy", "Jun 2st, 2026"),
        parse_vector("MMM do, yyyy", "June 3rd, 2026"),
        parse_vector("MMM do, yyyy", "jun 4TH, 2026"),
        parse_vector("MMMM do, yyyy", "June 21nd, 2026"),
        parse_vector("do MMM yyyy", "23rd Jun 2026"),
        parse_vector("yyyy_MM_dd", "2026_06_01"),
        parse_vector("yyyy-MM-dd", "2026-06-01"),
        parse_vector("yyyy/MM/dd", "2026/06/01"),
        parse_vector("MM/dd/yyyy", "06/01/2026"),
        parse_vector("M/d/y", "6/1/2026"),
        parse_vector("yy-M-d", "26-6-1"),
        parse_vector("MMM do, yyyy", "Jax 1st, 2026"),
        parse_vector("MMM do, yyyy", "Jun 1, 2026"),
        parse_vector("MMM do, yyyy", "Jun 1xx, 2026"),
        parse_vector("MMM do, yyyy", "Jun 1st, 2026 extra"),
        parse_vector("yyyy-MM-dd", "2026-13-01"),
        parse_vector("yyyy-MM-dd", "2026-06-40"),
        parse_vector("yyyy-MM-dd", "2026-00-01"),
        parse_vector("EEEE, yyyy-MM-dd", "Funday, 2026-06-01"),
        parse_vector("MM/dd/yyyy", "13/01/2026"),
        parse_vector("yyyy/MM/dd", "2026/06/00"),
    ];

    for date in [
        jd(2024, 1, 7),
        jd(2024, 1, 8),
        jd(2024, 1, 9),
        jd(2024, 1, 10),
        jd(2024, 1, 11),
        jd(2024, 1, 12),
        jd(2024, 1, 13),
    ] {
        vectors.push(parse_vector("EEEE EEE EE E yyyy-MM-dd", weekday_fmt.format(date)));
    }

    vectors
}
