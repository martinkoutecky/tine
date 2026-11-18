//! The document reader's answer with its own source map ([`SourceMap`]),
//! assembled per header line: the fields, drawers and alias members the
//! reader found there, at the native bytes it read them from. A physical
//! writer places these ranges instead of recognizing the file again.
//! Ephemeral, one outline read, O(file bytes + reader projections), no I/O or
//! retained cache.
use super::{Document, SourceMap};
use crate::{block_regions, page_header, page_properties, reference_evidence};
use std::{collections::HashMap, ops::Range};

/// Reset and return actual outline-reader calls on this thread, for downstream
/// operation cost tests (including reads outside the native adapter).
#[cfg(feature = "test-counters")]
pub fn take_outline_reads() -> usize {
    crate::outline::READS.with(|n| n.replace(0))
}

/// One physical file: the reader's document, its source map and its header.
pub struct NativeDocument {
    pub document: Document,
    pub map: SourceMap,
    /// Where the header ends: the first block's start, or the second's when
    /// the first root is the header.
    pub end: usize,
    pub first_root: bool,
    pub properties: bool,
    /// The text before the first block that is no header line.
    pub prose: Option<Range<usize>>,
    /// The physical lines before `end` outside the prose (Org page
    /// properties can follow it).
    pub lines: Vec<NativeLine>,
    pub drawers: Vec<(usize, usize)>,
    pub eol: &'static str,
    /// Comparisons by the forward drawer cursor (for operation cost tests).
    pub drawer_steps: usize,
    pub title: Option<String>,
    pub title_range: Option<Range<usize>>,
}

/// A physical header line with its native field form and source ranges.
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

/// A header entry's canonical reference name and reader rule.
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
}

impl NativeDocument {
    /// Read a file once, exposing the same tree as `parse` / `parse_org` and
    /// the reader's source map. No format/index semantics change.
    pub fn read(text: &str, org: bool) -> Self {
        let read = super::parse_outlined(text, org, true);
        Self::from_read(text, org, read.document, read.map.expect("mapped read"))
    }

    pub(crate) fn from_read(text: &str, org: bool, document: Document, map: SourceMap) -> Self {
        let first_root = document.pre_block.is_none()
            && page_header::page_header_text(None, document.roots.first().map(|b| b.raw()))
                .is_some();
        let owner = first_root.then_some(0);
        let read = if first_root {
            map.block(0)
        } else {
            map.pre_block()
        };
        let end = map
            .block_start(usize::from(first_root))
            .unwrap_or(text.len());
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
        let prose = document
            .pre_block
            .as_deref()
            .and_then(|pre| page_properties::preamble_range_in(pre, org, &regions));
        let prose = prose.map(|r| map.native(None, r.start)..map.native(None, r.end));
        let has_properties = first_root
            || regions.page_properties().next().is_some()
            || (org
                && regions
                    .find_org_preamble_property_region(observed)
                    .is_some());
        let title_field = crate::model::title_property_in(&regions);
        let title = title_field.map(|p| observed[p.value_range.0..p.value_range.1].to_owned());
        let title_range = title_field
            .map(|p| map.native(owner, p.value_range.0)..map.native(owner, p.value_range.1));
        let (mut cursor, mut drawer_steps) = (0, 0);
        // Org page properties can follow the prose: those lines are header too.
        let tail = prose
            .as_ref()
            .is_some_and(|p| !text[p.end..end.max(p.end)].trim().is_empty());
        let mut lines: Vec<_> = map
            .lines()
            .map(|i| (i, map.line(i)))
            .take_while(|(_, range)| range.start < end)
            .filter(|(_, range)| {
                prose
                    .as_ref()
                    .is_none_or(|p| range.start < p.start || (tail && range.start >= p.end))
            })
            .map(|(i, range)| {
                let read = read.contains(&i);
                let at = read.then(|| map.offset(i));
                let content = range.start + if read { map.removed(i) } else { 0 }..range.end;
                let payload = if read {
                    map.payload(i)
                } else {
                    content.start..content.start
                };
                let found = at.and_then(|at| properties.get(&at));
                let bytes = |r: Range<usize>| map.native(owner, r.start)..map.native(owner, r.end);
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
                                names.push((member.name.clone(), bytes(run.clone())));
                                tokens.push(bytes(member.range.clone()));
                            } else {
                                movable = false;
                            }
                        }
                        NativeAlias {
                            value: bytes(p.value_range.0..p.value_range.1).start,
                            names,
                            tokens,
                            movable,
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
                    key: found.map_or(0..0, |p| bytes(p.key_range.0..p.key_range.1)),
                    value: found.map_or(0..0, |p| bytes(p.value_range.0..p.value_range.1)),
                    drawer: boxes
                        .get(cursor)
                        .filter(|&&(open, _)| at.is_some_and(|at| open <= at))
                        .map(|_| cursor),
                    blank: crate::text_trace::blank(&text[content.clone()]),
                    unterminated_end: map.end(i),
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
        Self {
            document,
            map,
            end,
            first_root,
            properties: has_properties,
            prose,
            lines,
            drawers,
            eol: super::line_convention(text),
            drawer_steps,
            title,
            title_range,
        }
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
                    n: &mut usize,
                    map: &SourceMap,
                    text: &str,
                ) {
                    for block in blocks {
                        let raw = map
                            .block(*n)
                            .map(|i| &text[map.payload(i)])
                            .collect::<Vec<_>>()
                            .join("\n");
                        assert_eq!(raw, block.raw());
                        *n += 1;
                        check(&block.children, n, map, text);
                    }
                }
                let mut n = 0;
                check(&native.document.roots, &mut n, &native.map, &text);
                assert!(native.map.block_start(n).is_none());
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
