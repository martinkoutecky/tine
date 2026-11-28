use super::{
    end_of_line, line_ranges, raw_range, trimmed_range, whole_line, BlockRegions, Drawer,
    LiteralBlock, Planning, Property, Range,
};
use crate::lsdoc_block_parse::Coords;
use lsdoc::ast::{Block, Inline, ListItem, Span};
use serde::Serialize;
use std::collections::HashMap;

/// A parser-recognized token used by the date chips and body renderer.
/// `span_start` matches the render AST. `line_start` is a raw UTF-8 offset.
#[derive(Debug, Serialize)]
pub struct RenderPlanning {
    pub kind: &'static str,
    pub span_start: usize,
    pub line_start: usize,
    pub value: Option<String>,
    pub has_trailing_body: bool,
    pub top_level: bool,
    /// The native date facets use uppercase source keywords.
    #[serde(skip)]
    pub canonical_keyword: bool,
}

/// Collect top-level display planning from a block's existing AST.
/// Visits top-level nodes and their direct inline flows, with no structural regions.
pub fn top_level_render_planning(raw: &str, blocks: &[Block]) -> Vec<RenderPlanning> {
    let coords = Coords::of(raw, false);
    let mut planning = PlanningScan::for_render();
    for inline in blocks.iter().filter_map(flow_inline) {
        visit_inline(raw, inline, coords, None, &mut planning, Some(true));
    }
    planning.into_render()
}

fn flow_inline(block: &Block) -> Option<&[Inline]> {
    match block {
        Block::Paragraph { inline, .. }
        | Block::Bullet { inline, .. }
        | Block::Heading { inline, .. } => Some(inline),
        _ => None,
    }
}

pub(super) fn visit_blocks(
    raw: &str,
    org: bool,
    blocks: &[Block],
    coords: Coords,
    out: &mut BlockRegions,
    planning: &mut PlanningScan,
    top_level: bool,
) {
    for block in blocks {
        if let Some(inline) = flow_inline(block) {
            visit_inline(raw, inline, coords, Some(out), planning, Some(top_level));
            continue;
        }
        match block {
            Block::Src { span, .. }
            | Block::Example { span, .. }
            | Block::Export { span, .. }
            | Block::CommentBlock { span, .. }
            | Block::DisplayedMath { span, .. }
            | Block::LatexEnv { span, .. }
            | Block::RawHtml { span, .. } => {
                if let Some(r) = raw_range(raw, span, coords) {
                    out.literals.push(r);
                    let (kind, lang) = match block {
                        Block::Src { lang, .. } => ("src", lang.as_str()),
                        Block::Example { .. } => ("example", ""),
                        _ => ("other", ""),
                    };
                    out.literal_blocks
                        .push(LiteralBlock::new(raw, kind, lang, r));
                }
            }
            Block::Custom { span, children, .. } => {
                if let Some(r) = raw_range(raw, span, coords) {
                    out.literals.push(r);
                    out.literal_blocks
                        .push(LiteralBlock::new(raw, "other", "", r));
                }
                visit_blocks(raw, org, children, coords, out, planning, false);
            }
            Block::Quote { children, .. } => {
                visit_blocks(raw, org, children, coords, out, planning, false)
            }
            Block::FootnoteDef { inline, .. } => {
                visit_inline(raw, inline, coords, Some(out), planning, None)
            }
            Block::List { items, .. } => visit_items(raw, org, items, coords, out, planning),
            Block::Table { header, rows, .. } => {
                for row in header.iter().chain(rows) {
                    for cell in row {
                        visit_inline(raw, cell, coords, Some(out), planning, None);
                    }
                }
            }
            Block::Properties {
                props,
                span,
                drawers,
                ..
            } => {
                if let Some(r) = raw_range(raw, span, coords) {
                    let r = whole_line(raw, r);
                    let index = out.property_regions.len();
                    out.property_regions.push(r);
                    // lsdoc answers which lines are drawer delimiters, folded
                    // ones (`:END: tail`) included: none of them is a row.
                    let mut delimiters = Vec::new();
                    for d in drawers {
                        for span in [d.open, d.close] {
                            if let Some(line) = raw_range(raw, &Some(span), coords) {
                                delimiters.push(whole_line(raw, line).0);
                            }
                        }
                    }
                    let lines = line_ranges(raw, r);
                    if org && !drawers.is_empty() && !is_one_frame(raw, &lines, &delimiters) {
                        out.opaque_regions.push(index);
                    }
                    // Index accepted parser keys once, rather than searching
                    // the entire folded property node for every source entry.
                    let mut names = HashMap::new();
                    for property in props {
                        let name = &property.0;
                        names.entry(name.to_ascii_lowercase()).or_insert(name);
                    }
                    for line in lines {
                        if delimiters.contains(&line.0) {
                            continue;
                        }
                        let t = trimmed_range(raw, line);
                        let s = t.slice(raw);
                        // A Properties node can fold Markdown lines, a drawer
                        // and directives together. Split only this accepted node;
                        // preserve each entry's source spelling and syntax.
                        let markdown = s.split_once("::").map(|(k, v)| (k, v, 0, 2, !org));
                        let directive = s
                            .strip_prefix("#+")
                            .and_then(|s| s.split_once(':'))
                            .map(|(k, v)| (k, v, 2, 1, false));
                        let drawer = s
                            .strip_prefix(':')
                            .and_then(|s| s.split_once(':'))
                            .map(|(k, v)| (k, v, 1, 1, org));
                        let candidates = if org {
                            [directive, drawer, markdown]
                        } else {
                            [markdown, directive, drawer]
                        };
                        // Other-format entries own spans (template id removal), not
                        // metadata; a `#+` line's references count (OG hoists it).
                        let Some((key, value, prefix, delim, appropriate)) = candidates
                            .into_iter()
                            .flatten()
                            .find(|(key, ..)| names.contains_key(&key.to_ascii_lowercase()))
                        else {
                            continue;
                        };
                        let name = names[&key.to_ascii_lowercase()];
                        let ks = t.0 + prefix;
                        let vs = ks + key.len() + delim;
                        let value_range = trimmed_range(raw, Range(vs, t.1));
                        out.properties.push(Property {
                            key: name.clone(),
                            value: value.trim().to_string(),
                            line,
                            key_range: Range(ks, ks + key.len()),
                            value_range,
                            region: index,
                            primary: appropriate,
                            directive: org && prefix == 2,
                            applicable: appropriate || prefix == 2,
                        });
                    }
                }
            }
            Block::Directive {
                name, value, span, ..
            } if org => {
                // A directive that does not start its line (after a bare CR or
                // a folded `:END:`) stays text of the line it shares.
                let starts_line = |r: &Range| {
                    raw[whole_line(raw, *r).0..r.0]
                        .bytes()
                        .all(|b| b == b' ' || b == b'\t')
                };
                if let Some(r) = raw_range(raw, span, coords).filter(starts_line) {
                    let line = whole_line(raw, r);
                    let region = out.property_regions.len();
                    out.property_regions.push(line);
                    // Locate sub-tokens only inside lsdoc's accepted directive.
                    let t = trimmed_range(raw, line);
                    if let Some(colon) = t.slice(raw).find(':') {
                        let key_range = Range(t.0 + 2, t.0 + colon);
                        let value_range = trimmed_range(raw, Range(t.0 + colon + 1, t.1));
                        out.properties.push(Property {
                            key: name.clone(),
                            value: value.clone(),
                            line,
                            key_range,
                            value_range,
                            region,
                            primary: false,
                            directive: true,
                            applicable: true,
                        });
                    }
                }
            }
            Block::Drawer { name, span, .. } => {
                if let Some(r) = raw_range(raw, span, coords) {
                    let r = whole_line(raw, r);
                    let lines = line_ranges(raw, r);
                    if let Some(last) = lines.last() {
                        let clocks = if name.eq_ignore_ascii_case("LOGBOOK") {
                            lines
                                .iter()
                                .skip(1)
                                .take(lines.len().saturating_sub(2))
                                .filter(|r| r.slice(raw).trim_start().starts_with("CLOCK:"))
                                .copied()
                                .collect()
                        } else {
                            Vec::new()
                        };
                        out.drawers.push(Drawer {
                            name: name.clone(),
                            range: r,
                            close: last.0,
                            clocks,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}
/// Whether an Org property region is exactly one drawer whose opener is its
/// first line and whose closer is its last, so the editor may drop that frame.
/// `lines` are the region's LF lines; `delimiters` the line starts of its
/// drawers' open and close spans.
fn is_one_frame(raw: &str, lines: &[Range], delimiters: &[usize]) -> bool {
    let (Some(first), Some(last), [open, close]) = (lines.first(), lines.last(), delimiters) else {
        return false;
    };
    let marker = |line: &Range, opening| {
        let text = line.slice(raw);
        let text = text.strip_suffix('\n').unwrap_or(text);
        super::is_property_drawer_marker(text.strip_suffix('\r').unwrap_or(text), opening)
    };
    *open == first.0 && *close == last.0 && marker(first, true) && marker(last, false)
}

fn visit_items(
    raw: &str,
    org: bool,
    items: &[ListItem],
    coords: Coords,
    out: &mut BlockRegions,
    planning: &mut PlanningScan,
) {
    for item in items {
        visit_blocks(raw, org, &item.content, coords, out, planning, false);
        visit_inline(raw, &item.name, coords, Some(out), planning, None);
        visit_items(raw, org, &item.items, coords, out, planning);
    }
}
#[derive(Default)]
pub(super) struct PlanningScan {
    last_line: Option<(Range, usize)>,
    render: Option<Vec<RenderPlanning>>,
}

impl PlanningScan {
    pub(super) fn for_render() -> Self {
        Self {
            render: Some(Vec::new()),
            ..Self::default()
        }
    }

    pub(super) fn into_render(self) -> Vec<RenderPlanning> {
        self.render.unwrap()
    }

    fn line(&mut self, raw: &str, timestamp: Range) -> (Range, usize) {
        let (mut line, first_nonblank) = match self.last_line {
            Some(cached) if cached.0.contains(timestamp.0) => cached,
            _ => {
                let line = whole_line(raw, Range(timestamp.0, timestamp.0));
                #[cfg(test)]
                PLANNING_BOUNDARY_BYTES.set(PLANNING_BOUNDARY_BYTES.get() + line.1 - line.0);
                let cached = (line, line.1 - line.slice(raw).trim_start().len());
                self.last_line = Some(cached);
                cached
            }
        };
        // lsdoc can accept a line break inside a timestamp. Cache the start's
        // physical line while the structural range includes the ending line.
        if timestamp.1 > line.1 {
            line.1 = end_of_line(raw, timestamp.1, line.0);
            #[cfg(test)]
            PLANNING_BOUNDARY_BYTES.set(PLANNING_BOUNDARY_BYTES.get() + line.1 - timestamp.1);
        }
        (line, first_nonblank)
    }
}

struct PlanningRun {
    line: Range,
    begin: usize,
    end: usize,
    accepts_more: bool,
}

impl PlanningRun {
    fn finish(self, raw: &str, render: &mut [RenderPlanning]) {
        if self.begin < render.len() {
            let has_trailing_body = !crate::editor_text::js_blank(&raw[self.end..self.line.1]);
            for token in &mut render[self.begin..] {
                token.has_trailing_body = has_trailing_body;
            }
        }
    }
}

fn visit_inline(
    raw: &str,
    inline: &[Inline],
    coords: Coords,
    mut out: Option<&mut BlockRegions>,
    planning: &mut PlanningScan,
    render_flow: Option<bool>,
) {
    let mut run: Option<PlanningRun> = None;
    for i in inline {
        match i {
            Inline::Code { span, .. } | Inline::Verbatim { span, .. } => {
                let Some(out) = out.as_deref_mut() else {
                    continue;
                };
                if let Some(r) = raw_range(raw, span, coords) {
                    out.literals.push(r);
                    #[cfg(test)]
                    INLINE_LITERAL_RECORDS.set(INLINE_LITERAL_RECORDS.get() + 1);
                }
            }
            Inline::Timestamp { ts, date, span }
                if matches!(ts.as_str(), "Scheduled" | "Deadline")
                    || (ts == "Closed" && out.is_some()) =>
            {
                if let (Some(r), Some(Span(start, _))) = (raw_range(raw, span, coords), span) {
                    let (line, first_nonblank) = planning.line(raw, r);
                    // The Timestamp owns the planning prefix. A glued suffix
                    // remains body text; only whitespace may precede the token.
                    if let Some(out) = out.as_deref_mut().filter(|_| r.0 <= first_nonblank) {
                        out.planning.push(Planning {
                            kind: ts.clone(),
                            line,
                            timestamp: r,
                            date: date.clone(),
                        });
                    }
                    // The body and date chips also own subsequent Scheduled/Deadline
                    // tokens separated only by whitespace in this inline flow.
                    if ts == "Closed" {
                        continue;
                    }
                    let (Some(top_level), Some(render)) = (render_flow, &mut planning.render)
                    else {
                        continue;
                    };
                    if run.as_ref().is_some_and(|run| !run.line.contains(r.0)) {
                        run.take().unwrap().finish(raw, render);
                    }
                    let run = run.get_or_insert_with(|| PlanningRun {
                        line,
                        begin: render.len(),
                        end: line.0,
                        accepts_more: true,
                    });
                    if !run.accepts_more {
                        continue;
                    }
                    run.accepts_more = raw
                        .get(run.end..r.0)
                        .is_some_and(crate::editor_text::js_blank);
                    if run.accepts_more {
                        let (kind, keyword) = if ts == "Scheduled" {
                            ("Scheduled", "SCHEDULED:")
                        } else {
                            ("Deadline", "DEADLINE:")
                        };
                        let text = r.slice(raw);
                        let value = text
                            .split_once('<')
                            .and_then(|(_, rest)| rest.split_once('>'))
                            .map(|(value, _)| value.to_owned());
                        render.push(RenderPlanning {
                            kind,
                            span_start: *start,
                            line_start: run.line.0,
                            value,
                            has_trailing_body: false,
                            top_level,
                            canonical_keyword: text.starts_with(keyword),
                        });
                        run.end = r.1;
                        run.line.1 = run.line.1.max(line.1);
                    }
                }
            }
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. }
            | Inline::Tag { children, .. }
            | Inline::Link {
                label: children, ..
            } => {
                if let Some(out) = out.as_deref_mut() {
                    visit_inline(raw, children, coords, Some(out), planning, None);
                }
            }
            _ => {}
        }
    }
    if let (Some(run), Some(render)) = (run, &mut planning.render) {
        run.finish(raw, render);
    }
}

#[cfg(test)]
thread_local! {
    static PLANNING_BOUNDARY_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static INLINE_LITERAL_RECORDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_regions::{from_blocks_with_planning, top_level_render_planning};

    #[test]
    fn export_planning_builds_no_literal_records() {
        for (org, delimiter) in [(false, "`"), (true, "~"), (true, "=")] {
            let raw = format!(
                "Review {} {delimiter}SCHEDULED: <2026-11-19 Thu>{delimiter}\nDEADLINE: <2026-11-20 Fri>",
                format!("{delimiter}code{delimiter} ").repeat(999)
            );
            let blocks = crate::lsdoc_block_parse::parse_block(&raw, org);
            INLINE_LITERAL_RECORDS.set(0);
            from_blocks_with_planning(&raw, org, &blocks);
            assert_eq!(INLINE_LITERAL_RECORDS.get(), 1000, "{delimiter}");
            INLINE_LITERAL_RECORDS.set(0);
            let planning = top_level_render_planning(&raw, &blocks);
            assert_eq!(planning.len(), 1, "{delimiter}");
            assert_eq!(planning[0].kind, "Deadline", "{delimiter}");
            assert_eq!(INLINE_LITERAL_RECORDS.get(), 0, "{delimiter}");
        }
    }

    #[test]
    fn planning_region_scan_is_linear() {
        let keywords: &[&[&str]] = &[
            &["SCHEDULED"],
            &["DEADLINE"],
            &["CLOSED"],
            &["SCHEDULED", "DEADLINE", "CLOSED"],
        ];
        let mut failures = Vec::new();
        for org in [false, true] {
            for keys in keywords {
                for separator in [" ", "\t"] {
                    for newline in ["\n", "\r\n"] {
                        for tail in ["", " trailing prose"] {
                            let cost = |n| {
                                let run = keys
                                    .iter()
                                    .cycle()
                                    .take(n)
                                    .map(|key| {
                                        let (open, close) = if *key == "CLOSED" {
                                            ('[', ']')
                                        } else {
                                            ('<', '>')
                                        };
                                        format!("{key}: {open}2026-11-19 Thu{close}")
                                    })
                                    .collect::<Vec<_>>()
                                    .join(separator);
                                let raw = format!("prose{newline}{run}{tail}{newline}after");
                                let blocks = crate::lsdoc_block_parse::parse_block(&raw, org);
                                PLANNING_BOUNDARY_BYTES.set(0);
                                let (regions, _) = from_blocks_with_planning(&raw, org, &blocks);
                                assert_eq!(
                                    regions.planning.len(),
                                    1,
                                    "{org} {keys:?} {separator:?} {newline:?} {tail:?}"
                                );
                                PLANNING_BOUNDARY_BYTES.get()
                            };
                            let (small, large) = (cost(128), cost(256));
                            assert!(small > 0);
                            if large > 2 * small + 64 {
                                failures.push(format!("org={org} {keys:?} {separator:?} {newline:?} {tail:?}: {small} -> {large}"));
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn planning_ranges_keep_line_ownership_across_multiline_timestamps() {
        for org in [false, true] {
            let raw = "Review\nSCHEDULED: <2026-11-19\n Thu> DEADLINE: <2026-11-20 Fri> tail\nCLOSED: [2026-11-21 Sat]";
            let blocks = crate::lsdoc_block_parse::parse_block(raw, org);
            let (regions, render) = from_blocks_with_planning(raw, org, &blocks);
            assert_eq!(
                regions
                    .planning
                    .iter()
                    .map(|p| p.kind.as_str())
                    .collect::<Vec<_>>(),
                ["Scheduled", "Closed"]
            );
            assert_eq!(
                render
                    .iter()
                    .map(|token| (token.kind, token.line_start, token.has_trailing_body))
                    .collect::<Vec<_>>(),
                [("Scheduled", 7, true), ("Deadline", 7, true)]
            );
        }
    }
}
