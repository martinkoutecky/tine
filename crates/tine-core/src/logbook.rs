//! OG Logseq-compatible `:LOGBOOK:` clock drawer handling.
//!
//! The block-region door owns drawer/planning boundaries. This module interprets
//! clock values only inside parser-accepted LOGBOOK rows, shared with wasm.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogbookFormat {
    Markdown,
    Org,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampParts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    /// 0=Sun, 1=Mon, ..., 6=Sat.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockRow {
    pub kind: String,
    pub start: String,
    pub end: Option<String>,
    pub span: Option<String>,
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

impl TimestampParts {
    pub fn format(self, with_seconds: bool) -> String {
        let wd = WEEKDAYS
            .get(self.weekday as usize)
            .copied()
            .unwrap_or("Sun");
        if with_seconds {
            format!(
                "{:04}-{:02}-{:02} {} {:02}:{:02}:{:02}",
                self.year, self.month, self.day, wd, self.hour, self.minute, self.second
            )
        } else {
            format!(
                "{:04}-{:02}-{:02} {} {:02}:{:02}",
                self.year, self.month, self.day, wd, self.hour, self.minute
            )
        }
    }
}

pub fn clock_in_at(
    raw: &str,
    format: LogbookFormat,
    with_seconds: bool,
    now: TimestampParts,
) -> String {
    insert_logbook_line(
        raw,
        format,
        &format!("CLOCK: [{}]", now.format(with_seconds)),
    )
}

pub fn clock_out_at(
    raw: &str,
    format: LogbookFormat,
    with_seconds: bool,
    now: TimestampParts,
) -> String {
    let regions = crate::block_regions::parse(raw, format == LogbookFormat::Org);
    clock_out_with_regions(
        raw,
        format == LogbookFormat::Org,
        with_seconds,
        now,
        &regions,
    )
}
fn clock_out_with_regions(
    raw: &str,
    is_org: bool,
    with_seconds: bool,
    now: TimestampParts,
    regions: &crate::block_regions::BlockRegions,
) -> String {
    let Some(drawer) = regions
        .drawers
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
    else {
        return raw.to_string();
    };
    let Some(row) = drawer.clocks.last().filter(|r| r.1 == drawer.close) else {
        return raw.to_string();
    };
    let clock_in_log = row.slice(raw).trim();
    if clock_in_log.contains("--") {
        return raw.to_string();
    }
    let Some(clock_start) = open_clock_start(clock_in_log) else {
        return raw.to_string();
    };
    let Some(start_sec) = parse_timestamp_seconds(clock_start, with_seconds) else {
        return raw.to_string();
    };
    let clock_end = now.format(with_seconds);
    let Some(end_sec) = parse_timestamp_seconds(&clock_end, with_seconds) else {
        return raw.to_string();
    };
    if end_sec < start_sec {
        return raw.to_string();
    }
    let span = format_span(end_sec - start_sec, with_seconds);
    regions
        .apply(
            raw,
            is_org,
            crate::block_regions::Edit::ClockOut {
                value: format!("CLOCK: [{}]--[{}] =>  {}", clock_start, clock_end, span),
            },
        )
        .expect("CLOCK edit on parsed block")
}

pub fn apply_marker_transition_at(
    raw: &str,
    format: LogbookFormat,
    old_marker: Option<&str>,
    new_marker: Option<&str>,
    enabled: bool,
    with_seconds: bool,
    now: TimestampParts,
) -> String {
    if !enabled {
        return raw.to_string();
    }
    let Some(new_marker) = marker_name(new_marker) else {
        return raw.to_string();
    };
    let old_marker = marker_name(old_marker);
    let regions = crate::block_regions::parse(raw, format == LogbookFormat::Org);
    let should_clock_in = match old_marker {
        None => new_marker == "doing" || new_marker == "now",
        Some("todo") => new_marker == "doing",
        Some("later") => new_marker == "now",
        Some("now") => {
            new_marker == "now"
                && !regions
                    .drawers
                    .iter()
                    .any(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
        }
        Some("doing") => {
            new_marker == "doing"
                && !regions
                    .drawers
                    .iter()
                    .any(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
        }
        _ => false,
    };
    if should_clock_in {
        return regions
            .apply(
                raw,
                format == LogbookFormat::Org,
                crate::block_regions::Edit::DrawerRow {
                    name: "LOGBOOK".into(),
                    value: format!("CLOCK: [{}]", now.format(with_seconds)),
                },
            )
            .expect("parsed clock transition");
    }

    let should_clock_out = matches!(
        (old_marker, new_marker),
        (Some("doing"), "todo") | (Some("now"), "later")
    ) || (matches!(old_marker, Some("now") | Some("doing"))
        && new_marker == "done");
    if should_clock_out {
        return clock_out_with_regions(
            raw,
            format == LogbookFormat::Org,
            with_seconds,
            now,
            &regions,
        );
    }
    raw.to_string()
}

pub fn has_logbook_drawer(raw: &str, is_org: bool) -> bool {
    crate::block_regions::parse(raw, is_org)
        .drawers
        .iter()
        .any(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
}

pub fn clock_summary_seconds(raw: &str, is_org: bool) -> u64 {
    clock_lines(raw, is_org)
        .into_iter()
        .filter_map(|line| {
            let span = line.split_once("=>")?.1.trim();
            parse_span_seconds(span)
        })
        .sum()
}

pub fn clock_summary_compact(raw: &str, is_org: bool) -> String {
    format_compact_duration(clock_summary_seconds(raw, is_org))
}

pub fn clock_rows(raw: &str, is_org: bool) -> Vec<ClockRow> {
    clock_lines(raw, is_org)
        .into_iter()
        .filter_map(parse_clock_row)
        .collect()
}

/// Clock rows and total duration from one parser-owned LOGBOOK projection.
/// O(bytes in one block); one parse, no graph work.
pub fn clock_info(raw: &str, is_org: bool) -> (Vec<ClockRow>, u64) {
    let rows = clock_rows(raw, is_org);
    let seconds = rows
        .iter()
        .filter_map(|r| r.span.as_deref().and_then(parse_span_seconds))
        .sum();
    (rows, seconds)
}

pub fn format_compact_duration(seconds: u64) -> String {
    if seconds == 0 {
        return "0s".to_string();
    }
    let days = seconds / 86_400;
    let mut rem = seconds % 86_400;
    let hours = rem / 3_600;
    rem %= 3_600;
    let minutes = rem / 60;
    let secs = rem % 60;
    let mut out = String::new();
    if days > 0 {
        out.push_str(&format!("{days}d"));
    }
    if hours > 0 {
        out.push_str(&format!("{hours}h"));
    }
    if minutes > 0 {
        out.push_str(&format!("{minutes}m"));
    }
    if secs > 0 {
        out.push_str(&format!("{secs}s"));
    }
    out
}

fn insert_logbook_line(raw: &str, format: LogbookFormat, value: &str) -> String {
    crate::block_regions::edit(
        raw,
        format == LogbookFormat::Org,
        crate::block_regions::Edit::DrawerRow {
            name: "LOGBOOK".into(),
            value: value.into(),
        },
    )
    .expect("LOGBOOK edit on parsed block")
}

#[cfg(test)]
fn is_md_property_line(line: &str) -> bool {
    crate::property_line::parse_property_line(line).is_some()
}

fn clock_lines(raw: &str, is_org: bool) -> Vec<&str> {
    let regions = crate::block_regions::parse(raw, is_org);
    regions
        .drawers
        .iter()
        .filter(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
        .flat_map(|d| {
            d.clocks
                .iter()
                .map(|r| r.slice(raw).trim_end_matches(['\r', '\n']))
        })
        .collect()
}

fn parse_clock_row(line: &str) -> Option<ClockRow> {
    let line = line.trim();
    let start = line.strip_prefix("CLOCK: [")?;
    let (start, rest) = start.split_once(']')?;
    let mut row = ClockRow {
        kind: "CLOCK".to_string(),
        start: start.to_string(),
        end: None,
        span: None,
    };
    if let Some(rest) = rest.strip_prefix("--[") {
        let (end, rest) = rest.split_once(']')?;
        row.end = Some(end.to_string());
        if let Some((_, span)) = rest.split_once("=>") {
            row.span = Some(span.trim().to_string());
        }
    }
    Some(row)
}

fn open_clock_start(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("CLOCK: [")?;
    let (start, tail) = rest.split_once(']')?;
    tail.trim().is_empty().then_some(start)
}

fn marker_name(marker: Option<&str>) -> Option<&str> {
    marker
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(|m| match m {
            "TODO" | "todo" => "todo",
            "DOING" | "doing" => "doing",
            "LATER" | "later" => "later",
            "NOW" | "now" => "now",
            "DONE" | "done" => "done",
            _ => m,
        })
}

fn parse_timestamp_seconds(ts: &str, with_seconds: bool) -> Option<i64> {
    let mut parts = ts.split_whitespace();
    let date = parts.next()?;
    let weekday = parts.next()?;
    let time = parts.next()?;
    if parts.next().is_some() || weekday.len() != 3 {
        return None;
    }
    let mut d = date.split('-');
    let year: i32 = d.next()?.parse().ok()?;
    let month: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    if d.next().is_some() {
        return None;
    }
    let mut t = time.split(':');
    let hour: u32 = t.next()?.parse().ok()?;
    let minute: u32 = t.next()?.parse().ok()?;
    let second = if with_seconds {
        t.next()?.parse().ok()?
    } else {
        0
    };
    if t.next().is_some() {
        return None;
    }
    if !with_seconds && time.matches(':').count() != 1 {
        return None;
    }
    if with_seconds && time.matches(':').count() != 2 {
        return None;
    }
    Some(
        days_from_civil(year, month, day)? * 86_400
            + (hour as i64) * 3_600
            + (minute as i64) * 60
            + second as i64,
    )
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = month as i64 + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn format_span(seconds: i64, with_seconds: bool) -> String {
    if with_seconds {
        let hours = seconds / 3_600;
        let minutes = (seconds % 3_600) / 60;
        let seconds = seconds % 60;
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        let minutes_total = seconds / 60;
        let hours = minutes_total / 60;
        let minutes = minutes_total % 60;
        format!("{hours:02}:{minutes:02}")
    }
}

fn parse_span_seconds(span: &str) -> Option<u64> {
    let parts: Vec<&str> = span.trim().split(':').collect();
    match parts.as_slice() {
        [h, m] => Some(h.parse::<u64>().ok()? * 3_600 + m.parse::<u64>().ok()? * 60),
        [h, m, s] => Some(
            h.parse::<u64>().ok()? * 3_600 + m.parse::<u64>().ok()? * 60 + s.parse::<u64>().ok()?,
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// og 14 Q5 follow-up: the TypeScript properties panel writes only keys in
    /// this shared fixture's accepted list (`src/editor/editablePropertyKeys.test.ts`
    /// pins the TS side). Both Rust property readers must find every one again,
    /// and must accept the whole class the TS validator allows (Unicode letters,
    /// marks, numbers, `_`, `.`, `/`, `-`). Widening the panel means widening these readers first.
    #[test]
    fn editable_property_keys_fixture() {
        let fixture = include_str!("../tests/fixtures/editable-property-keys.txt");
        let accepted: Vec<&str> = fixture
            .lines()
            .filter_map(|line| line.strip_prefix('+'))
            .collect();
        assert!(accepted.len() > 5);
        let class = ('a'..='z')
            .chain('A'..='Z')
            .chain('0'..='9')
            .chain(['-', '_', '.', '/', 'é', '本'])
            .map(|c| format!("a{c}b"));
        for key in accepted.iter().map(|k| k.to_string()).chain(class) {
            let line = format!("{key}:: v");
            assert_eq!(
                crate::doc::parse_property_line(&line),
                Some((key.as_str(), "v")),
                "{key}"
            );
            assert!(is_md_property_line(&line), "{key}");
        }
    }

    #[test]
    fn markdown_logbook_properties_use_the_core_recognizer() {
        for line in ["klíč:: v", "a.b:: v", "\tkey:: v", "empty::"] {
            assert_eq!(
                is_md_property_line(line),
                crate::property_line::parse_property_line(line).is_some(),
                "{line:?}"
            );
        }
        for line in ["key::value", "a b:: value"] {
            assert!(!is_md_property_line(line), "{line:?}");
        }
    }

    fn ts(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        weekday: u32,
    ) -> TimestampParts {
        TimestampParts {
            year,
            month,
            day,
            weekday,
            hour,
            minute,
            second,
        }
    }

    #[test]
    fn clock_in_then_clock_out_writes_exact_og_seconds_format() {
        let raw = "TODO Ship logbook\nSCHEDULED: <2026-06-25 Thu>\nDEADLINE: <2026-06-26 Fri>\nowner:: martin\nBody stays below";
        let start = ts(2026, 6, 25, 9, 0, 0, 4);
        let end = ts(2026, 6, 25, 10, 2, 3, 4);
        let running = clock_in_at(raw, LogbookFormat::Markdown, true, start);
        assert_eq!(
            running,
            "TODO Ship logbook\nSCHEDULED: <2026-06-25 Thu>\nDEADLINE: <2026-06-26 Fri>\nowner:: martin\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 09:00:00]\n:END:\nBody stays below"
        );
        let closed = clock_out_at(&running, LogbookFormat::Markdown, true, end);
        assert_eq!(
            closed,
            "TODO Ship logbook\nSCHEDULED: <2026-06-25 Thu>\nDEADLINE: <2026-06-26 Fri>\nowner:: martin\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 09:00:00]--[2026-06-25 Thu 10:02:03] =>  01:02:03\n:END:\nBody stays below"
        );
    }

    #[test]
    fn minute_mode_omits_seconds_and_uses_hour_minute_span() {
        let raw = "TODO Ship";
        let start = ts(2026, 6, 25, 9, 0, 30, 4);
        let end = ts(2026, 6, 25, 9, 31, 45, 4);
        let running = clock_in_at(raw, LogbookFormat::Markdown, false, start);
        assert_eq!(
            running,
            "TODO Ship\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 09:00]\n:END:"
        );
        assert_eq!(
            clock_out_at(&running, LogbookFormat::Markdown, false, end),
            "TODO Ship\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 09:00]--[2026-06-25 Thu 09:31] =>  00:31\n:END:"
        );
    }

    #[test]
    fn appends_to_existing_logbook_without_reformatting_existing_rows() {
        let raw = "TODO Ship\n:LOGBOOK:\nCLOCK: [2026-06-24 Wed 08:00:00]--[2026-06-24 Wed 08:10:00] =>  00:10:00\n:END:\nBody";
        let out = clock_in_at(
            raw,
            LogbookFormat::Markdown,
            true,
            ts(2026, 6, 25, 9, 0, 0, 4),
        );
        assert_eq!(
            out,
            "TODO Ship\n:LOGBOOK:\nCLOCK: [2026-06-24 Wed 08:00:00]--[2026-06-24 Wed 08:10:00] =>  00:10:00\nCLOCK: [2026-06-25 Thu 09:00:00]\n:END:\nBody"
        );
    }

    #[test]
    fn org_insert_places_logbook_after_properties_drawer() {
        let raw =
            "TODO Ship\nSCHEDULED: <2026-06-25 Thu>\n:PROPERTIES:\n:owner: martin\n:END:\nBody";
        let out = clock_in_at(raw, LogbookFormat::Org, true, ts(2026, 6, 25, 9, 0, 0, 4));
        assert_eq!(
            out,
            "TODO Ship\nSCHEDULED: <2026-06-25 Thu>\n:PROPERTIES:\n:owner: martin\n:END:\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 09:00:00]\n:END:\nBody"
        );
    }

    #[test]
    fn unrelated_marker_save_preserves_og_logbook_verbatim() {
        let raw = "TODO Ship\n:LOGBOOK:\nCLOCK: [2026-06-24 Wed 08:00:00]--[2026-06-24 Wed 08:10:00] =>  00:10:00\n:END:\nBody";
        let out = apply_marker_transition_at(
            raw,
            LogbookFormat::Markdown,
            Some("TODO"),
            Some("TODO"),
            true,
            true,
            ts(2026, 6, 25, 9, 0, 0, 4),
        );
        assert_eq!(out, raw);
    }

    #[test]
    fn marker_transition_table_matches_og_and_no_double_clock_in() {
        let now = ts(2026, 6, 25, 9, 0, 0, 4);
        for (old, new) in [
            (None, Some("DOING")),
            (None, Some("NOW")),
            (Some("TODO"), Some("DOING")),
            (Some("LATER"), Some("NOW")),
            (Some("DOING"), Some("DOING")),
            (Some("NOW"), Some("NOW")),
        ] {
            let out = apply_marker_transition_at(
                "DOING Ship",
                LogbookFormat::Markdown,
                old,
                new,
                true,
                true,
                now,
            );
            assert!(
                out.contains("CLOCK: [2026-06-25 Thu 09:00:00]"),
                "{old:?}->{new:?}: {out}"
            );
        }

        let running = "DOING Ship\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 08:00:00]\n:END:";
        for (old, new) in [
            (Some("DOING"), Some("TODO")),
            (Some("NOW"), Some("LATER")),
            (Some("DOING"), Some("DONE")),
            (Some("NOW"), Some("DONE")),
        ] {
            let out = apply_marker_transition_at(
                running,
                LogbookFormat::Markdown,
                old,
                new,
                true,
                true,
                now,
            );
            assert!(
                out.contains("--[2026-06-25 Thu 09:00:00] =>  01:00:00"),
                "{old:?}->{new:?}: {out}"
            );
        }

        let with_logbook = "DOING Ship\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 08:00:00]--[2026-06-25 Thu 08:30:00] =>  00:30:00\n:END:";
        let guarded = apply_marker_transition_at(
            with_logbook,
            LogbookFormat::Markdown,
            Some("DOING"),
            Some("DOING"),
            true,
            true,
            now,
        );
        assert_eq!(guarded, with_logbook);
        let disabled = apply_marker_transition_at(
            "DOING Ship",
            LogbookFormat::Markdown,
            Some("TODO"),
            Some("DOING"),
            false,
            true,
            now,
        );
        assert_eq!(disabled, "DOING Ship");
        let ignored = apply_marker_transition_at(
            "DONE Ship",
            LogbookFormat::Markdown,
            Some("TODO"),
            Some("DONE"),
            true,
            true,
            now,
        );
        assert_eq!(ignored, "DONE Ship");
    }

    #[test]
    fn clock_summary_trusts_stored_spans_and_formats_compactly() {
        let raw = "DONE Ship\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 09:00:00]--[2026-06-25 Thu 10:00:00] =>  00:05:00\nCLOCK: [2026-06-25 Thu 10:00:00]--[2026-06-25 Thu 11:00:00] =>  1:20\nCLOCK: [2026-06-25 Thu 12:00:00]--[2026-06-25 Thu 12:00:45] =>  00:00:45\n:END:";
        assert_eq!(clock_summary_seconds(raw, false), 5 * 60 + 80 * 60 + 45);
        assert_eq!(clock_summary_compact(raw, false), "1h25m45s");
        let rows = clock_rows(raw, false);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, "CLOCK");
        assert_eq!(rows[0].span.as_deref(), Some("00:05:00"));
    }
}
