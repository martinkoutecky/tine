//! Parser-owned regions and source edits for ONE raw outline block (I-4/I-12).
//!
//! `parse` answers header, literal, property, planning, drawer and identity ownership
//! in raw UTF-8 byte coordinates. `from_blocks` answers the same questions from an
//! existing single-block AST, without parsing. Both cost O(bytes in this block +
//! AST nodes), never O(page/graph). `BlockRegions` can be retained while raw is
//! unchanged; callers must not know parser preparation, wrapper syntax or placement.
//!
//! `edit` / `apply` set/remove properties and planning, strip copy metadata, project
//! the visible body, normalize planning, and insert drawer rows. Only parser-owned
//! ranges can be replaced; new metadata is placed outside literals. No file I/O.
//! An invalid request returns an error; a quarantined parse refuses edits. Callers
//! must surface that refusal. Debug builds reparse and verify literal preservation.
//! Sub-token scans below are confined to regions lsdoc has ALREADY accepted.

use lsdoc::ast::{Block, Inline, ListItem, Span};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Range(pub usize, pub usize);
impl Range {
    pub fn slice<'a>(&self, raw: &'a str) -> &'a str {
        &raw[self.0..self.1]
    }
    pub fn contains(&self, at: usize) -> bool {
        self.0 <= at && at < self.1
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Header {
    pub marker: Option<String>,
    pub priority: Option<String>,
    pub heading: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Property {
    pub key: String,
    pub value: String,
    pub line: Range,
    pub key_range: Range,
    pub value_range: Range,
    pub region: usize,
    pub primary: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Planning {
    pub kind: String,
    pub line: Range,
    pub timestamp: Range,
    pub date: serde_json::Value,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Drawer {
    pub name: String,
    pub range: Range,
    pub close: usize,
    pub clocks: Vec<Range>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BlockRegions {
    pub header: Header,
    pub literals: Vec<Range>,
    pub property_regions: Vec<Range>,
    pub properties: Vec<Property>,
    pub planning: Vec<Planning>,
    pub drawers: Vec<Drawer>,
    pub id: Option<Property>,
    pub quarantined: bool,
}

// The only conversion from prepared single-block coordinates to raw coordinates.
fn raw_range(raw: &str, span: &Option<Span>) -> Option<Range> {
    let Span(start, end) = span.as_ref()?;
    let lead = raw.len() - raw.trim_start().len();
    let r = Range(
        (start.saturating_sub(2) + lead).min(raw.len()),
        (end.saturating_sub(2) + lead).min(raw.len()),
    );
    (r.0 < r.1 && raw.is_char_boundary(r.0) && raw.is_char_boundary(r.1)).then_some(r)
}
fn whole_line(raw: &str, r: Range) -> Range {
    let start = raw[..r.0].rfind('\n').map_or(0, |p| p + 1);
    let end = if r.1 > start && raw.as_bytes()[r.1 - 1] == b'\n' {
        r.1
    } else {
        raw[r.1..].find('\n').map_or(raw.len(), |p| r.1 + p + 1)
    };
    Range(start, end)
}
fn line_ranges(raw: &str, range: Range) -> Vec<Range> {
    let mut at = range.0;
    range
        .slice(raw)
        .split_inclusive('\n')
        .map(|s| {
            let r = Range(at, at + s.len());
            at += s.len();
            r
        })
        .collect()
}
fn trimmed_range(raw: &str, range: Range) -> Range {
    let s = range.slice(raw);
    Range(
        range.0 + s.len() - s.trim_start().len(),
        range.1 - (s.len() - s.trim_end().len()),
    )
}

/// Parse one raw block with the same boundary used by render. No page work.
pub fn parse(raw: &str, is_org: bool) -> BlockRegions {
    #[cfg(not(target_arch = "wasm32"))]
    let blocks = match std::panic::catch_unwind(|| crate::render::parse_block(raw, is_org)) {
        Ok(blocks) => blocks,
        Err(_) => {
            return BlockRegions {
                literals: vec![Range(0, raw.len())],
                quarantined: true,
                ..BlockRegions::default()
            }
        }
    };
    #[cfg(target_arch = "wasm32")]
    let blocks = crate::render::parse_block(raw, is_org);
    from_blocks(raw, is_org, &blocks)
}

/// Derive regions from render's existing AST; O(block bytes), zero parses.
pub fn from_blocks(raw: &str, is_org: bool, blocks: &[Block]) -> BlockRegions {
    let mut result = BlockRegions::default();
    if let Some(
        Block::Bullet {
            marker,
            priority,
            size,
            ..
        }
        | Block::Heading {
            marker,
            priority,
            size,
            ..
        },
    ) = blocks.first()
    {
        result.header = Header {
            marker: marker.clone(),
            priority: priority.clone(),
            heading: *size,
        };
    }
    visit_blocks(raw, is_org, blocks, &mut result);
    result.literals.sort_by_key(|r| r.0);
    let mut merged: Vec<Range> = Vec::new();
    for r in result.literals.drain(..) {
        if let Some(last) = merged.last_mut().filter(|last| r.0 <= last.1) {
            last.1 = last.1.max(r.1);
        } else {
            merged.push(r);
        }
    }
    result.literals = merged;
    // Nested content under a literal Custom belongs to that container, even if
    // lsdoc emitted child Properties or timestamps there.
    result
        .properties
        .retain(|p| !result.literals.iter().any(|r| r.contains(p.line.0)));
    result
        .planning
        .retain(|p| !result.literals.iter().any(|r| r.contains(p.line.0)));
    result
        .drawers
        .retain(|p| !result.literals.iter().any(|r| r.contains(p.range.0)));
    let own = result.own_org_region(raw);
    for p in &mut result.properties {
        p.primary = p.primary && (!is_org || Some(p.region) == own);
    }
    result.id = result
        .properties
        .iter()
        .find(|p| p.primary && p.key.eq_ignore_ascii_case("id"))
        .cloned();
    result
}
fn visit_blocks(raw: &str, org: bool, blocks: &[Block], out: &mut BlockRegions) {
    for block in blocks {
        match block {
            Block::Src { span, .. }
            | Block::Example { span, .. }
            | Block::Export { span, .. }
            | Block::CommentBlock { span, .. }
            | Block::DisplayedMath { span, .. }
            | Block::LatexEnv { span, .. }
            | Block::RawHtml { span, .. } => {
                if let Some(r) = raw_range(raw, span) {
                    out.literals.push(r);
                }
            }
            Block::Custom { span, children, .. } => {
                if let Some(r) = raw_range(raw, span) {
                    out.literals.push(r);
                }
                visit_blocks(raw, org, children, out);
            }
            Block::Quote { children, .. } => visit_blocks(raw, org, children, out),
            Block::Paragraph { inline, .. }
            | Block::Bullet { inline, .. }
            | Block::Heading { inline, .. }
            | Block::FootnoteDef { inline, .. } => visit_inline(raw, inline, out),
            Block::List { items, .. } => visit_items(raw, org, items, out),
            Block::Table { header, rows, .. } => {
                for row in header.iter().chain(rows) {
                    for cell in row {
                        visit_inline(raw, cell, out);
                    }
                }
            }
            Block::Properties { props, span } => {
                if let Some(r) = raw_range(raw, span) {
                    let r = whole_line(raw, r);
                    let index = out.property_regions.len();
                    out.property_regions.push(r);
                    for line in line_ranges(raw, r) {
                        let t = trimmed_range(raw, line);
                        let s = t.slice(raw);
                        // A Properties node can fold Markdown lines, a drawer
                        // and directives together. Split only this accepted node;
                        // preserve each entry's source spelling and syntax.
                        let parts = s
                            .split_once("::")
                            .map(|(k, v)| (k, v, 0, 2, !org))
                            .or_else(|| {
                                s.strip_prefix("#+")
                                    .and_then(|s| s.split_once(':'))
                                    .map(|(k, v)| (k, v, 2, 1, false))
                            })
                            .or_else(|| {
                                s.strip_prefix(':')
                                    .and_then(|s| s.split_once(':'))
                                    .map(|(k, v)| (k, v, 1, 1, org))
                            });
                        let Some((key, value, prefix, delim, appropriate)) = parts else {
                            continue;
                        };
                        let Some(prop) = props.iter().find(|p| p.0.eq_ignore_ascii_case(key))
                        else {
                            continue;
                        };
                        let ks = t.0 + prefix;
                        let vs = ks + key.len() + delim;
                        let value_range = trimmed_range(raw, Range(vs, t.1));
                        out.properties.push(Property {
                            key: prop.0.clone(),
                            value: value.trim().to_string(),
                            line,
                            key_range: Range(ks, ks + key.len()),
                            value_range,
                            region: index,
                            primary: appropriate,
                        });
                    }
                }
            }
            Block::Drawer { name, span } => {
                if let Some(r) = raw_range(raw, span) {
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
fn visit_items(raw: &str, org: bool, items: &[ListItem], out: &mut BlockRegions) {
    for item in items {
        visit_blocks(raw, org, &item.content, out);
        visit_inline(raw, &item.name, out);
        visit_items(raw, org, &item.items, out);
    }
}
fn visit_inline(raw: &str, inline: &[Inline], out: &mut BlockRegions) {
    for i in inline {
        match i {
            Inline::Code { span, .. } | Inline::Verbatim { span, .. } => {
                if let Some(r) = raw_range(raw, span) {
                    out.literals.push(r);
                }
            }
            Inline::Timestamp { ts, date, span }
                if matches!(ts.as_str(), "Scheduled" | "Deadline" | "Closed") =>
            {
                if let Some(r) = raw_range(raw, span) {
                    let line = whole_line(raw, r);
                    // The Timestamp owns the planning prefix. A glued suffix
                    // remains body text; only whitespace may precede the token.
                    if raw[line.0..r.0].trim().is_empty() {
                        out.planning.push(Planning {
                            kind: ts.clone(),
                            line,
                            timestamp: r,
                            date: date.clone(),
                        });
                    }
                }
            }
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. }
            | Inline::Tag { children, .. } => visit_inline(raw, children, out),
            Inline::Link { label, .. } => visit_inline(raw, label, out),
            _ => {}
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Edit {
    Property {
        key: String,
        value: Option<String>,
    },
    Properties {
        values: Vec<(String, String)>,
    },
    Planning {
        which: String,
        value: Option<String>,
    },
    StripCopy {
        template: bool,
    },
    Visible,
    NormalizePlanning,
    DrawerRow {
        name: String,
        value: String,
    },
    ClockOut {
        value: String,
    },
}

fn splice(raw: &str, mut edits: Vec<(Range, String)>) -> String {
    edits.sort_by_key(|(r, _)| r.0);
    let mut merged: Vec<(Range, String)> = Vec::new();
    for (r, text) in edits {
        if let Some((previous, replacement)) = merged.last_mut() {
            if replacement.is_empty() && text.is_empty() && previous.1 == r.0 {
                previous.1 = r.1;
                continue;
            }
        }
        merged.push((r, text));
    }
    let mut edits = merged;
    let preceding_end = edits.iter().rev().nth(1).map_or(0, |(r, _)| r.1);
    if let Some((r, _)) = edits
        .last_mut()
        .filter(|(r, text)| text.is_empty() && r.1 == raw.len() && r.0 > 0 && !raw.ends_with('\n'))
    {
        if raw[..r.0].ends_with("\r\n") && r.0 >= preceding_end + 2 {
            r.0 -= 2;
        } else if raw[..r.0].ends_with('\n') && r.0 > preceding_end {
            r.0 -= 1;
        }
    }
    let mut out = String::with_capacity(raw.len());
    let mut at = 0;
    for (r, text) in edits {
        assert!(r.0 >= at);
        out.push_str(&raw[at..r.0]);
        out.push_str(&text);
        at = r.1;
    }
    out.push_str(&raw[at..]);
    out
}
fn newline(raw: &str) -> &'static str {
    if raw.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}
impl BlockRegions {
    fn planning_removal(raw: &str, p: &Planning) -> Range {
        if raw[p.timestamp.1..p.line.1].trim().is_empty() {
            p.line
        } else {
            Range(p.line.0, p.timestamp.1)
        }
    }
    pub fn property(&self, key: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|p| p.key.eq_ignore_ascii_case(key))
    }
    pub fn literal_at(&self, at: usize) -> bool {
        self.literals.iter().any(|r| r.contains(at))
    }
    fn hoist_planning(&self, raw: &str, after: &str) -> String {
        let mut entries: Vec<&Planning> = self
            .planning
            .iter()
            .filter(|p| p.kind != "Closed")
            .collect();
        entries.sort_by_key(|p| if p.kind == "Scheduled" { 0 } else { 1 });
        let at = self.title_end(raw);
        if entries.is_empty() || entries.iter().any(|p| p.line.0 < at) {
            return raw.to_string();
        }
        let mut text = entries
            .iter()
            .map(|p| p.timestamp.slice(raw))
            .collect::<Vec<_>>()
            .join(newline(raw));
        if !after.is_empty() {
            text.push_str(newline(raw));
            text.push_str(after);
        }
        text.push_str(newline(raw));
        let mut edits: Vec<_> = entries
            .iter()
            .map(|p| (Self::planning_removal(raw, p), String::new()))
            .collect();
        if let Some((_, replacement)) = edits.iter_mut().find(|(r, _)| r.0 == at) {
            *replacement = text;
        } else {
            edits.push((Range(at, at), text));
        }
        splice(raw, edits)
    }
    fn title_end(&self, raw: &str) -> usize {
        // If the first thing is a literal container, insert after it, never into it.
        if let Some(r) = self
            .literals
            .iter()
            .find(|r| r.0 == raw.len() - raw.trim_start().len())
        {
            whole_line(raw, *r).1
        } else {
            raw.find('\n').map_or(raw.len(), |p| p + 1)
        }
    }
    fn head_end(&self, raw: &str) -> usize {
        let mut at = self.title_end(raw);
        loop {
            if let Some(p) = self.planning.iter().find(|p| p.line.0 == at) {
                at = p.line.1;
            } else {
                break;
            }
        }
        at
    }
    fn own_org_region(&self, raw: &str) -> Option<usize> {
        let at = self.head_end(raw);
        self.property_regions.iter().position(|r| {
            (r.0 == at || r.0 == 0)
                && r.slice(raw)
                    .lines()
                    .next()
                    .is_some_and(|line| line.trim().eq_ignore_ascii_case(":PROPERTIES:"))
        })
    }
    fn inserted(&self, raw: &str, at: usize, text: &str) -> String {
        let nl = newline(raw);
        let prefix = if at > 0 && !raw[..at].ends_with('\n') {
            nl
        } else {
            ""
        };
        let suffix = if at < raw.len() { nl } else { "" };
        splice(
            raw,
            vec![(Range(at, at), format!("{prefix}{text}{suffix}"))],
        )
    }
    fn remove_properties(
        &self,
        raw: &str,
        org: bool,
        remove: impl Fn(&Property) -> bool,
    ) -> String {
        let selected: Vec<&Property> = self.properties.iter().filter(|p| remove(p)).collect();
        let mut edits = Vec::new();
        for (index, r) in self.property_regions.iter().enumerate() {
            let all: Vec<&Property> = self
                .properties
                .iter()
                .filter(|p| p.region == index)
                .collect();
            if org && !all.is_empty() && all.iter().all(|p| remove(p)) {
                edits.push((*r, String::new()));
            } else {
                for p in selected.iter().filter(|p| p.region == index) {
                    edits.push((p.line, String::new()));
                }
            }
        }
        splice(raw, edits)
    }
    fn set_values(
        &self,
        raw: &str,
        org: bool,
        values: Vec<(String, String)>,
    ) -> Result<String, String> {
        let own = self.own_org_region(raw);
        let mut edits = Vec::new();
        let mut add = Vec::new();
        for (key, value) in values {
            if key.is_empty() || key.contains(['\n', '\r', ':']) || value.contains(['\n', '\r']) {
                return Err("Invalid property edit".into());
            }
            let matching: Vec<_> = self
                .properties
                .iter()
                .filter(|p| p.key.eq_ignore_ascii_case(&key) && (!org || Some(p.region) == own))
                .collect();
            if let Some(first) = matching.first() {
                let trailer = matches!(key.as_str(), "id" | "collapsed" | "logseq.order-list-type");
                let region = self.property_regions[first.region];
                if !org && !trailer && region.0 != 0 && region.0 != self.head_end(raw) {
                    // Move legacy trailing user properties to the existing head,
                    // using accepted property spans rather than a second scanner.
                    edits.extend(matching.iter().map(|p| (p.line, String::new())));
                    add.push((key, value));
                } else {
                    edits.push((first.value_range, value));
                    edits.extend(matching.iter().skip(1).map(|p| (p.line, String::new())));
                }
            } else {
                add.push((key, value));
            }
        }
        if !add.is_empty() {
            let lines = add
                .iter()
                .map(|(key, value)| {
                    if org {
                        format!(":{key}: {value}")
                    } else {
                        format!("{key}:: {value}")
                    }
                })
                .collect::<Vec<_>>()
                .join(newline(raw));
            let (at, text) = if org {
                if let Some(index) = own {
                    let body_lines = line_ranges(raw, self.property_regions[index]);
                    let close = body_lines
                        .iter()
                        .find(|r| r.slice(raw).trim().eq_ignore_ascii_case(":END:"))
                        .ok_or("Properties drawer has no parser-owned closer")?;
                    (close.0, lines)
                } else {
                    let at = self.head_end(raw);
                    let text = format!(":PROPERTIES:{}{lines}{}:END:", newline(raw), newline(raw));
                    // OG places accepted planning above a new drawer even when
                    // authored below body content. Literal lookalikes are absent.
                    if self
                        .planning
                        .iter()
                        .any(|p| p.kind != "Closed" && p.line.0 >= at)
                    {
                        return Ok(self.hoist_planning(raw, &text));
                    }
                    (at, text)
                }
            } else {
                let at = if add.iter().any(|(key, _)| {
                    matches!(key.as_str(), "id" | "collapsed" | "logseq.order-list-type")
                }) || self.literal_at(0)
                {
                    raw.len()
                } else if let Some(r) = self
                    .property_regions
                    .iter()
                    .find(|r| r.0 == self.head_end(raw) || r.0 == 0)
                {
                    r.1
                } else {
                    self.head_end(raw)
                };
                (at, lines)
            };
            let prefix = if at > 0 && !raw[..at].ends_with('\n') {
                newline(raw)
            } else {
                ""
            };
            let suffix = if at < raw.len() { newline(raw) } else { "" };
            if !text.is_empty() {
                edits.push((Range(at, at), format!("{prefix}{text}{suffix}")));
            }
        }
        Ok(splice(raw, edits))
    }
    /// Apply an operation using these regions from EXACTLY this raw source.
    /// O(block bytes); zero parses in release, one preservation reparse in debug.
    pub fn apply(&self, raw: &str, org: bool, edit: Edit) -> Result<String, String> {
        if self.quarantined {
            return Err("Structural edit refused: block parsing is quarantined".into());
        }
        #[cfg(debug_assertions)]
        let (property_keys, all_properties, planning_kind, drawer_name) = match &edit {
            Edit::Property { key, .. } => (vec![key.to_ascii_lowercase()], false, None, None),
            Edit::Properties { values } => (
                values.iter().map(|(k, _)| k.to_ascii_lowercase()).collect(),
                false,
                None,
                None,
            ),
            Edit::StripCopy { template } => (
                if *template {
                    vec![
                        "id".into(),
                        "template".into(),
                        "template-including-parent".into(),
                    ]
                } else {
                    vec!["id".into()]
                },
                false,
                None,
                None,
            ),
            Edit::Visible => (vec![], true, None, None),
            Edit::Planning { which, .. } => (vec![], false, Some(which.clone()), None),
            Edit::DrawerRow { name, .. } => (vec![], false, None, Some(name.clone())),
            Edit::ClockOut { .. } => (vec![], false, None, Some("LOGBOOK".into())),
            Edit::NormalizePlanning => (vec![], false, None, None),
        };
        let out = match edit {
            Edit::Property { key, value } => {
                if key.is_empty()
                    || key.contains(['\n', '\r', ':'])
                    || value.as_ref().is_some_and(|v| v.contains(['\n', '\r']))
                {
                    return Err("Invalid property edit".into());
                }
                let own = org.then(|| self.own_org_region(raw)).flatten();
                let matching: Vec<&Property> = self
                    .properties
                    .iter()
                    .filter(|p| p.key.eq_ignore_ascii_case(&key) && (!org || Some(p.region) == own))
                    .collect();
                if value.is_none() {
                    self.remove_properties(raw, org, |p| matching.contains(&p))
                } else {
                    self.set_values(raw, org, vec![(key, value.unwrap())])?
                }
            }
            Edit::Properties { values } => self.set_values(raw, org, values)?,

            Edit::Planning { which, value } => {
                if !matches!(which.as_str(), "Scheduled" | "Deadline" | "Closed") {
                    return Err("Invalid planning kind".into());
                }
                let matches: Vec<&Planning> =
                    self.planning.iter().filter(|p| p.kind == which).collect();
                if let Some(value) = value {
                    if value.contains(['\n', '\r']) {
                        return Err("Invalid planning value".into());
                    }
                    let text = format!("{}: {value}", which.to_ascii_uppercase());
                    if let Some(first) = matches.first() {
                        let suffix = &raw[first.timestamp.1..first.line.1];
                        let text = if suffix.trim().is_empty() {
                            text
                        } else {
                            format!("{text}{}", newline(raw))
                        };
                        let mut edits = vec![(first.timestamp, text)];
                        edits.extend(
                            matches
                                .iter()
                                .skip(1)
                                .map(|p| (Self::planning_removal(raw, p), String::new())),
                        );
                        splice(raw, edits)
                    } else {
                        self.inserted(raw, self.title_end(raw), &text)
                    }
                } else {
                    splice(
                        raw,
                        matches
                            .iter()
                            .map(|p| (Self::planning_removal(raw, p), String::new()))
                            .collect(),
                    )
                }
            }
            Edit::StripCopy { template } => self.remove_properties(raw, org, |p| {
                p.key.eq_ignore_ascii_case("id")
                    || (template
                        && matches!(
                            p.key.to_ascii_lowercase().as_str(),
                            "template" | "template-including-parent"
                        ))
            }),
            Edit::Visible => {
                let edits: Vec<_> = self
                    .property_regions
                    .iter()
                    .filter(|r| !self.literal_at(r.0))
                    .map(|r| (*r, String::new()))
                    .collect();
                if edits.is_empty() {
                    raw.to_string()
                } else {
                    splice(raw, edits).trim_end_matches('\n').to_string()
                }
            }
            Edit::NormalizePlanning => {
                if self.planning.is_empty() || self.literal_at(0) {
                    raw.to_string()
                } else {
                    self.hoist_planning(raw, "")
                        .trim_end_matches('\n')
                        .to_string()
                }
            }
            Edit::DrawerRow { name, value } => {
                if value.contains(['\n', '\r']) {
                    return Err("Invalid drawer row".into());
                }
                if let Some(d) = self
                    .drawers
                    .iter()
                    .find(|d| d.name.eq_ignore_ascii_case(&name))
                {
                    self.inserted(raw, d.close, &value)
                } else {
                    let mut at = self.head_end(raw);
                    if let Some(r) = self.property_regions.iter().find(|r| r.0 == at) {
                        at = r.1;
                    }
                    self.inserted(
                        raw,
                        at,
                        &format!(":{name}:{}{value}{}:END:", newline(raw), newline(raw)),
                    )
                }
            }
            Edit::ClockOut { value } => {
                if value.contains(['\n', '\r']) {
                    return Err("Invalid clock row".into());
                }
                let row = self
                    .drawers
                    .iter()
                    .find(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
                    .and_then(|d| d.clocks.last());
                row.map_or_else(
                    || raw.to_string(),
                    |r| splice(raw, vec![(trimmed_range(raw, *r), value)]),
                )
            }
        };
        #[cfg(debug_assertions)]
        if out != raw {
            let after = parse(&out, org);
            debug_assert!(
                !after.quarantined,
                "I-2: edited region shape must remain parseable"
            );
            debug_assert_eq!(
                self.header, after.header,
                "I-4: structural edits preserve the parsed header"
            );
            let properties = |r: &BlockRegions| {
                r.properties
                    .iter()
                    .filter(|p| {
                        !all_properties && !property_keys.contains(&p.key.to_ascii_lowercase())
                    })
                    .map(|p| (p.key.clone(), p.value.clone()))
                    .collect::<Vec<_>>()
            };
            debug_assert_eq!(
                properties(self),
                properties(&after),
                "I-4: unrelated property shape changed"
            );
            let planning = |r: &BlockRegions, source: &str| {
                let mut entries = r
                    .planning
                    .iter()
                    .filter(|p| Some(&p.kind) != planning_kind.as_ref())
                    .map(|p| (p.kind.clone(), p.timestamp.slice(source).to_string()))
                    .collect::<Vec<_>>();
                entries.sort();
                entries
            };
            debug_assert_eq!(
                planning(self, raw),
                planning(&after, &out),
                "I-4: unrelated planning shape changed"
            );
            let drawers = |r: &BlockRegions, source: &str| {
                r.drawers
                    .iter()
                    .filter(|d| {
                        !drawer_name
                            .as_ref()
                            .is_some_and(|n| d.name.eq_ignore_ascii_case(n))
                    })
                    .map(|d| {
                        (
                            d.name.clone(),
                            d.range
                                .slice(source)
                                .trim_end_matches(['\r', '\n'])
                                .to_string(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            debug_assert_eq!(
                drawers(self, raw),
                drawers(&after, &out),
                "I-4: unrelated drawer shape changed"
            );
            // Literal payload bytes remain identical. Edits may move their offsets,
            // so compare source slices rather than stale absolute coordinates.
            let before_literals: Vec<_> = self
                .literals
                .iter()
                .map(|r| r.slice(raw).trim_end_matches(['\r', '\n']))
                .collect();
            let after_literals: Vec<_> = after
                .literals
                .iter()
                .map(|r| r.slice(&out).trim_end_matches(['\r', '\n']))
                .collect();
            debug_assert_eq!(
                before_literals, after_literals,
                "I-4: structural edits preserve parser-owned literals"
            );
        }
        Ok(out)
    }
}
/// Parse and edit one block. Use `BlockRegions::apply` when its AST is cached.
pub fn edit(raw: &str, org: bool, request: Edit) -> Result<String, String> {
    parse(raw, org).apply(raw, org, request)
}
