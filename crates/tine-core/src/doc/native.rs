//! Source coordinates of the document reader's answer. A physical writer uses
//! these ranges rather than recognizing the file again. Ephemeral, O(file
//! bytes + reader projections), one outline read, no I/O or retained cache.
use super::{continuation_raw, physical_lines, Document};
use crate::{block_regions, outline, page_header, page_properties, reference_evidence};
use std::{collections::HashMap, ops::Range};

/// Reset and return actual outline-reader calls on this thread, for downstream
/// operation cost tests (including reads outside the native adapter).
#[cfg(feature = "test-counters")]
pub fn take_outline_reads() -> usize {
    outline::READS.with(|n| n.replace(0))
}

/// One physical file, its logical tree and the coordinates that produced it.
pub struct NativeDocument {
    pub document: Document,
    pub end: usize,
    pub body: NativeBody,
    pub first_root: bool,
    pub properties: bool,
    pub overlapping_prose: bool,
    pub prose: Option<Range<usize>>,
    pub prose_lines: Vec<Range<usize>>,
    pub prose_separator: Range<usize>,
    pub lines: Vec<NativeLine>,
    pub drawers: Vec<(usize, usize)>,
    pub eol: &'static str,
    pub join_end: usize,
    pub header_insert: usize,
    pub cuts: Vec<usize>,
    /// Comparisons by the forward drawer cursor (for operation cost tests).
    pub drawer_steps: usize,
    pub title: Option<String>,
    pub title_range: Option<Range<usize>>,
}

/// A logical header line with its native field form and source ranges.
pub struct NativeLine {
    pub range: Range<usize>,
    pub content: Range<usize>,
    pub payload: Range<usize>,
    pub unterminated_end: usize,
    pub declaration: Range<usize>,
    pub property: Option<(String, String)>,
    pub key: Range<usize>,
    pub value: Range<usize>,
    pub drawer: Option<usize>,
    pub frame: bool,
    pub blank: bool,
    pub alias: Option<NativeAlias>,
    pub references: Vec<NativeReference>,
}

/// A header entry's canonical reference name and reader rule after name edits.
pub struct NativeReference {
    pub name: String,
    pub rule: &'static str,
}

/// Canonically selected members, decoded names and complete native tokens.
pub struct NativeAlias {
    pub value: usize,
    pub names: Vec<(String, Range<usize>)>,
    pub tokens: Vec<Range<usize>>,
    pub movable: bool,
    /// Nonmember fragments the existing complete-text certificate cannot
    /// consume as declaration syntax. Native ranges, independent of allocation.
    pub protected: Vec<Range<usize>>,
}

/// Preorder block coordinates; child counts describe the reader's tree.
pub struct NativeBlock {
    pub lines: Vec<Range<usize>>,
    pub children: usize,
}

/// Native body payloads and the removable per-root continuation prefixes.
pub struct NativeBody {
    pub blocks: Vec<NativeBlock>,
    pub prefixes: Vec<Range<usize>>,
    pub normalization_lines: usize,
}

impl NativeDocument {
    /// Read edited header entries in their original context. A whole-title
    /// replacement can remove or introduce inner references; it must not be
    /// split at an old reference's coordinates. No outline read; linear in
    /// header coordinates and changed entry bytes, with transient entry buffers.
    pub(crate) fn map_header_references(
        &mut self,
        text: &str,
        trace: &crate::text_trace::TextTrace,
    ) -> Result<(), crate::text_trace::TraceError> {
        let edits: Vec<_> = trace
            .pieces
            .iter()
            .enumerate()
            .take_while(|(_, p)| p.range.start < self.end)
            .filter_map(|(id, p)| trace.substitution.get(&id).map(|s| (&p.range, s)))
            .collect();
        let mut cursor = 0;
        for n in 0..self.lines.len() {
            let line = &self.lines[n];
            while edits
                .get(cursor)
                .is_some_and(|(r, _)| r.end <= line.content.start)
            {
                cursor += 1;
            }
            if !edits
                .get(cursor)
                .is_some_and(|(r, _)| r.start < line.content.end)
            {
                continue;
            }
            let mut raw = String::new();
            let drawer = line.drawer.map(|d| self.drawers[d]);
            if let Some((open, _)) = drawer {
                raw.push_str(&text[self.lines[open].content.clone()]);
            }
            let mut at = line.content.start;
            while let Some((range, value)) = edits
                .get(cursor)
                .filter(|(r, _)| r.start < line.content.end)
            {
                if range.start < at || range.end > line.content.end {
                    return Err(crate::text_trace::TraceError::Partition);
                }
                raw.push_str(&text[at..range.start]);
                raw.push_str(value);
                at = range.end;
                cursor += 1;
            }
            raw.push_str(&text[at..line.content.end]);
            if super::line_terminator(&raw).is_empty() {
                raw.push('\n');
            }
            if let Some((_, close)) = drawer {
                raw.push_str(&text[self.lines[close].content.clone()]);
                if super::line_terminator(&raw).is_empty() {
                    raw.push('\n');
                }
            }
            let raw = crate::org::lone_cr_to_lf(&raw);
            self.lines[n].references = page_properties::pre_block_projections(&raw, true)
                .into_iter()
                .flat_map(|(_, b)| {
                    b.projection()
                        .reference_source()
                        .explicit
                        .iter()
                        .filter(|r| !matches!(r.rule, "explicit_task_marker" | "explicit_priority"))
                        .map(|r| NativeReference {
                            name: r.name.clone(),
                            rule: r.rule,
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
        }
        Ok(())
    }

    /// Read a file once, exposing the same tree as `parse` / `parse_org` and
    /// source coordinates from that outline. No format/index semantics change.
    pub fn read(text: &str, org: bool) -> Self {
        let lf = super::normalize_line_endings(text);
        let (document, headers) = super::parse_outlined(&lf, org);
        Self::from_outlined(text, org, document, &headers)
    }

    pub(crate) fn from_outlined(
        text: &str,
        org: bool,
        document: Document,
        headers: &[outline::Header],
    ) -> Self {
        let outline = outline::native_ranges(text, headers);
        let first_root = document.pre_block.is_none()
            && page_header::page_header_text(None, document.roots.first().map(|b| b.raw()))
                .is_some();
        let end = outline
            .first()
            .map_or(text.len(), |b| if first_root { b.0.end } else { b.0.start });
        let indent = if first_root {
            outline.first().map_or(0, |b| b.3)
        } else {
            0
        };
        let observed = if first_root {
            document.roots[0].raw()
        } else {
            document.pre_block.as_deref().unwrap_or("")
        };
        let regions = block_regions::parse_document(observed, org);
        let properties: HashMap<_, _> = regions.page_properties().map(|p| (p.line.0, p)).collect();
        let mut block = super::DocBlock::new(observed);
        block.set_org(org);
        let source = (org
            || properties
                .values()
                .any(|p| page_header::is_alias_key(&p.key)))
        .then(|| block.read_for_rename());
        let mut alias_members = source
            .as_ref()
            .map(|source| {
                reference_evidence::linkable_property_references_in(
                    source.explicit.iter(),
                    &block.projection().regions,
                    |r| &r.reference,
                )
                .map(|(_, r)| r)
                .filter(|r| !r.slot.encoding && !r.slot.nested)
                .collect::<Vec<_>>()
            })
            .unwrap_or_default()
            .into_iter()
            .peekable();
        let mut references = source
            .as_ref()
            .map(|s| s.explicit.iter())
            .into_iter()
            .flatten()
            .peekable();
        let boxes = regions.property_drawer_boundaries(observed);
        let mut observed_lines = (!observed.is_empty())
            .then(|| observed.split('\n'))
            .into_iter()
            .flatten();
        let (mut physical, mut logical) = (0, 0);
        let mut coordinates = Vec::new();
        for raw in physical_lines(&text[..end]) {
            let skip = if first_root && outline.first().is_some_and(|b| b.0.start == physical) {
                outline[0].1 - physical
            } else if first_root && outline.first().is_some_and(|b| physical > b.0.start) {
                raw.len() - continuation_raw(raw, indent).len()
            } else {
                0
            };
            let observed_line =
                if first_root && outline.first().is_some_and(|b| physical < b.0.start) {
                    None
                } else {
                    observed_lines.next()
                };
            let at = observed_line.map(|line| {
                let at = logical;
                logical += line.len() + 1;
                at
            });
            coordinates.push((
                physical..physical + raw.len(),
                physical + skip..physical + raw.len(),
                physical + skip..physical + skip + observed_line.map_or(0, str::len),
                at,
            ));
            physical += raw.len();
        }
        let prose_logical = document
            .pre_block
            .as_deref()
            .and_then(|pre| page_properties::preamble_range_in(pre, org, &regions));
        let overlapping_prose = prose_logical
            .as_ref()
            .is_some_and(|r| regions.page_properties().any(|p| p.line.0 >= r.start));
        let has_properties = first_root
            || regions.page_properties().next().is_some()
            || (org
                && regions
                    .find_org_preamble_property_region(observed)
                    .is_some());
        let prose = prose_logical.map(|r| {
            let (mut start, mut finish, mut extra) = (r.start, r.end, 0);
            for (at, _) in text[..end].match_indices("\r\n") {
                if at - extra >= r.end {
                    break;
                }
                start += usize::from(at - extra < r.start);
                finish += 1;
                extra += 1;
            }
            start..finish
        });
        let title_field = crate::model::title_property_in(&regions);
        let title = title_field.map(|p| observed[p.value_range.0..p.value_range.1].to_owned());
        let title_range = title_field.and_then(|p| {
            coordinates.iter().find_map(|(_, content, _, at)| {
                (*at == Some(p.line.0)).then(|| {
                    content.start + p.value_range.0 - p.line.0
                        ..content.start + p.value_range.1 - p.line.0
                })
            })
        });
        let (mut cursor, mut drawer_steps) = (0, 0);
        let mut lines: Vec<_> = coordinates
            .into_iter()
            .filter(|(r, _, _, _)| prose.as_ref().is_none_or(|p| r.start < p.start))
            .map(|(range, content, payload, at)| {
                let found = at.and_then(|at| properties.get(&at));
                let bytes = |r: &block_regions::Range| {
                    content.start + r.0 - at.unwrap()..content.start + r.1 - at.unwrap()
                };
                let alias = found
                    .filter(|p| page_header::is_alias_key(&p.key))
                    .map(|p| {
                        while alias_members
                            .peek()
                            .is_some_and(|m| m.range.start < p.value_range.0)
                        {
                            alias_members.next();
                        }
                        let (mut names, mut tokens, mut movable) = (Vec::new(), Vec::new(), true);
                        while alias_members
                            .peek()
                            .is_some_and(|m| m.range.start < p.value_range.1)
                        {
                            let member = alias_members.next().unwrap();
                            if let Some(run) = member.slot.run() {
                                let physical = |r: &Range<usize>| {
                                    content.start + r.start - at.unwrap()
                                        ..content.start + r.end - at.unwrap()
                                };
                                names.push((member.name.clone(), physical(run)));
                                tokens.push(physical(&member.range));
                            } else {
                                movable = false;
                            }
                        }
                        let value = bytes(&p.value_range).start;
                        let mut protected = Vec::new();
                        if !crate::text_trace::alias_key(&text[content.start..value]) {
                            protected.push(content.start..value);
                        }
                        let mut at = value;
                        for token in &tokens {
                            if !crate::text_trace::alias_syntax(&text[at..token.start]) {
                                protected.push(at..token.start);
                            }
                            at = token.end;
                        }
                        if !crate::text_trace::alias_syntax(&text[at..content.end]) {
                            protected.push(at..content.end);
                        }
                        NativeAlias {
                            value,
                            names,
                            tokens,
                            movable,
                            protected,
                        }
                    });
                while boxes
                    .get(cursor)
                    .is_some_and(|&(_, close)| Some(close) < at)
                {
                    drawer_steps += 1;
                    cursor += 1;
                }
                drawer_steps += 1;
                let mut line_refs = Vec::new();
                if let Some(at) = at {
                    while references
                        .peek()
                        .is_some_and(|r| r.range.start < at + payload.len())
                    {
                        let r = references.next().unwrap();
                        if r.range.start >= at
                            && !matches!(r.rule, "explicit_task_marker" | "explicit_priority")
                        {
                            line_refs.push(NativeReference {
                                name: r.name.clone(),
                                rule: r.rule,
                            });
                        }
                    }
                }
                NativeLine {
                    property: found.map(|p| (super::property_key_norm(&p.key), p.value.clone())),
                    key: found.map_or(0..0, |p| bytes(&p.key_range)),
                    value: found.map_or(0..0, |p| bytes(&p.value_range)),
                    drawer: boxes
                        .get(cursor)
                        .filter(|&&(open, _)| at.is_some_and(|at| open <= at))
                        .map(|_| cursor),
                    blank: crate::text_trace::blank(&text[content.clone()]),
                    unterminated_end: content.end
                        - super::line_terminator(&text[content.clone()]).len(),
                    declaration: content.start
                        ..content.start + text[content.clone()].trim_end().len(),
                    range,
                    content,
                    payload,
                    frame: false,
                    alias,
                    references: line_refs,
                }
            })
            .collect();
        let mut drawers: Vec<(usize, usize)> = Vec::new();
        let mut index = HashMap::new();
        for (n, line) in lines.iter_mut().enumerate() {
            if let Some(d) = line.drawer {
                let at = *index.entry(d).or_insert_with(|| {
                    drawers.push((n, n));
                    drawers.len() - 1
                });
                drawers[at].1 = n;
                line.drawer = Some(at);
            }
        }
        for &(open, close) in &drawers {
            lines[open].frame = true;
            lines[close].frame = true;
        }
        // A non-field header entry is allocated as its own line, not with the
        // literal-region context of its neighbours. Ask the pre-block reader
        // for that line's references; fields already have their native context.
        if org {
            for line in lines
                .iter_mut()
                .filter(|l| l.property.is_none() && l.drawer.is_none() && !l.frame && !l.blank)
            {
                let raw = super::normalize_line_endings(&text[line.content.clone()]);
                line.references.clear();
                for (_, block) in page_properties::pre_block_projections(&raw, true) {
                    for r in block.read_for_rename().explicit {
                        if !matches!(r.rule, "explicit_task_marker" | "explicit_priority") {
                            line.references.push(NativeReference {
                                name: r.name.clone(),
                                rule: r.rule,
                            });
                        }
                    }
                }
            }
        }
        let header_insert = lines
            .iter()
            .rposition(|l| !text[l.range.clone()].trim().is_empty())
            .map_or(0, |n| n + 1);
        let mut body = NativeBody {
            blocks: Vec::new(),
            prefixes: Vec::new(),
            normalization_lines: 0,
        };
        let (mut root_level, mut offset) = (u32::MAX, 0);
        let mut children = Vec::new();
        fn child_counts(blocks: &[super::DocBlock], out: &mut Vec<usize>) {
            for b in blocks {
                out.push(b.children.len());
                child_counts(&b.children, out);
            }
        }
        child_counts(&document.roots, &mut children);
        for ((range, content, level, indent), children) in outline.iter().zip(children) {
            if *level <= root_level {
                root_level = *level;
                let prefix = &text[range.start..*content];
                offset = if org {
                    0
                } else {
                    prefix.len() - outline::strip_layout_ws(prefix, usize::MAX).len()
                };
            }
            let finish = if org && range.end == text.len() {
                text.trim_end_matches(['\r', '\n']).len()
            } else {
                range.end
            };
            let mut at = range.start;
            let mut payloads = Vec::new();
            for (n, line) in physical_lines(&text[range.start..finish]).enumerate() {
                let raw = line.trim_end_matches(['\r', '\n']);
                let skip = if n == 0 {
                    content - at
                } else {
                    raw.len() - continuation_raw(raw, *indent).len()
                };
                payloads.push(at + skip..at + raw.len());
                if at >= end && offset > 0 {
                    body.normalization_lines += 1;
                    let layout = if n == 0 {
                        raw.len() - outline::strip_layout_ws(raw, usize::MAX).len()
                    } else {
                        skip
                    };
                    let prefix = offset.min(layout);
                    if prefix > 0 {
                        body.prefixes.push(at..at + prefix);
                    }
                }
                at += line.len();
            }
            body.blocks.push(NativeBlock {
                lines: payloads,
                children,
            });
        }
        let mut prose_lines = Vec::new();
        let mut prose_separator = end..end;
        if let Some(p) = &prose {
            let mut at = p.start;
            for line in physical_lines(&text[p.clone()]) {
                prose_lines.push(at..at + line.len());
                at += line.len();
            }
            let terminator = physical_lines(&text[p.end..end])
                .next()
                .map_or(0, |l| super::line_terminator(l).len());
            prose_separator = p.end..p.end + terminator;
        }
        let join_end = if org {
            let at = text.trim_end_matches(['\r', '\n']).len();
            (at + physical_lines(&text[at..]).next().map_or(0, str::len)).max(end)
        } else {
            text.len()
        };
        let mut cuts = vec![end, join_end];
        if let Some(p) = &prose {
            cuts.extend([p.start, p.end]);
            cuts.extend(physical_lines(&text[p.start..end]).scan(p.start, |at, l| {
                *at += l.len();
                Some(*at)
            }));
        }
        for l in &lines {
            cuts.extend([l.range.start, l.content.start, l.content.end]);
            if !l.frame {
                cuts.push(l.payload.end);
            }
            if let Some(a) = &l.alias {
                cuts.extend([
                    a.value,
                    l.content.end - super::line_terminator(&text[l.content.clone()]).len(),
                ]);
                for r in a.names.iter().map(|(_, r)| r).chain(&a.tokens) {
                    cuts.extend([r.start, r.end]);
                }
            }
        }
        cuts.extend(
            body.blocks
                .iter()
                .flat_map(|b| &b.lines)
                .chain(&body.prefixes)
                .flat_map(|r| [r.start, r.end]),
        );
        Self {
            document,
            end,
            body,
            first_root,
            properties: has_properties,
            overlapping_prose,
            prose,
            prose_lines,
            prose_separator,
            lines,
            drawers,
            eol: super::line_convention(text),
            join_end,
            header_insert,
            cuts,
            drawer_steps,
            title,
            title_range,
        }
    }

    /// Whether an output root contains a top-level Org directive outside its
    /// literal regions, through the bounded document reader.
    pub fn has_directive(&self, root: usize) -> bool {
        self.document.roots.get(root).is_some_and(|b| {
            crate::render::parse_text_bounded(b.raw(), true).is_none_or(|ast| {
                ast.blocks
                    .iter()
                    .any(|b| matches!(b, crate::lsdoc::ast::Block::Directive { .. }))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_coordinates_preserve_the_document_readers_tree_and_payloads() {
        for org in [false, true] {
            let lf = if org {
                "#+title: Page\n#+alias: A, [[Two Words]]\n\n* parent\nbody\n** child\n#+BEGIN_SRC\n* literal\n#+END_SRC\n\n"
            } else {
                "title:: Page\nalias:: A, [[Two Words]]\n\n  - parent\n    body\n    - child\n      ```\n      - literal\n      ```\n\n"
            };
            for eol in ["\n", "\r\n", "\r"] {
                let text = lf.replace('\n', eol);
                let native = NativeDocument::read(&text, org);
                let ordinary = if org {
                    crate::org::parse_org(&text)
                } else {
                    super::super::parse(&text)
                };
                assert_eq!(native.document, ordinary);
                fn check(
                    blocks: &[super::super::DocBlock],
                    coordinates: &mut std::slice::Iter<'_, NativeBlock>,
                    text: &str,
                ) {
                    for block in blocks {
                        let native = coordinates.next().unwrap();
                        let raw = native
                            .lines
                            .iter()
                            .map(|r| &text[r.clone()])
                            .collect::<Vec<_>>()
                            .join("\n");
                        assert_eq!(raw, block.raw());
                        assert_eq!(native.children, block.children.len());
                        check(&block.children, coordinates, text);
                    }
                }
                let mut blocks = native.body.blocks.iter();
                check(&native.document.roots, &mut blocks, &text);
                assert!(blocks.next().is_none());
                assert_eq!(native.title.as_deref(), Some("Page"));
                assert_eq!(&text[native.title_range.clone().unwrap()], "Page");
                let alias = native.lines.iter().find_map(|l| l.alias.as_ref()).unwrap();
                assert_eq!(
                    alias
                        .names
                        .iter()
                        .map(|(n, _)| n.as_str())
                        .collect::<Vec<_>>(),
                    ["A", "Two Words"]
                );
                assert_eq!(
                    alias
                        .tokens
                        .iter()
                        .map(|r| &text[r.clone()])
                        .collect::<Vec<_>>(),
                    ["A", "[[Two Words]]"]
                );
                assert_eq!(native.eol, eol);
            }
        }
    }
}
