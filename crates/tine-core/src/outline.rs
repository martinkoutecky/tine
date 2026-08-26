//! Shared storage adapter over lsdoc's source-oriented outline API.
//!
//! This module owns no Markdown or Org recognition. It validates parser-owned
//! source events, maps their exact ranges into Tine's existing document model,
//! and refuses event shapes that that model cannot represent without assigning
//! two semantic blocks to the same physical source line.

use crate::doc::{DocBlock, Document, ParsedDocument, PromotedHeadingLayout};
use lsdoc::{OutlineHeader, OutlineHeaderKind};
use std::fmt;
use std::ops::Range;

#[cfg(test)]
thread_local! {
    static OUTLINE_PARSE_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_parse_attempts() {
    OUTLINE_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
}

#[cfg(test)]
pub(crate) fn parse_attempts() -> usize {
    OUTLINE_PARSE_ATTEMPTS.with(std::cell::Cell::get)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutlineFormat {
    Markdown,
    Org,
}

impl OutlineFormat {
    fn lsdoc_name(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Org => "org",
        }
    }

    fn accepts(self, kind: OutlineHeaderKind) -> bool {
        matches!(
            (self, kind),
            (
                Self::Markdown,
                OutlineHeaderKind::MarkdownUnbulletedAtxHeading
                    | OutlineHeaderKind::MarkdownDashBullet
            ) | (Self::Org, OutlineHeaderKind::OrgHeadline)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OutlineAdapterError {
    ParserOwnership,
    InvalidSourceRange { event: usize },
    WrongFormatKind { event: usize },
    ZeroLevel { event: usize },
    NonPhysicalLineHeader { event: usize },
    OverlappingPhysicalLines { first: usize, second: usize },
}

impl fmt::Display for OutlineAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParserOwnership => f.write_str("lsdoc did not take ownership of the source"),
            Self::InvalidSourceRange { event } => {
                write!(f, "lsdoc outline event {event} has invalid source ranges")
            }
            Self::WrongFormatKind { event } => {
                write!(f, "lsdoc outline event {event} has the wrong format kind")
            }
            Self::ZeroLevel { event } => {
                write!(f, "lsdoc outline event {event} has structural level zero")
            }
            Self::NonPhysicalLineHeader { event } => write!(
                f,
                "lsdoc outline event {event} starts inside a physical line"
            ),
            Self::OverlappingPhysicalLines { first, second } => write!(
                f,
                "lsdoc outline events {first} and {second} occupy the same or overlapping physical lines"
            ),
        }
    }
}

impl std::error::Error for OutlineAdapterError {}

#[derive(Clone, Debug)]
struct PhysicalLine {
    content: Range<usize>,
    full: Range<usize>,
}

fn physical_lines(source: &str) -> Vec<PhysicalLine> {
    let bytes = source.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0_usize;
    while start < bytes.len() {
        let mut end = start;
        while end < bytes.len() && !matches!(bytes[end], b'\r' | b'\n') {
            end += 1;
        }
        let mut full_end = end;
        if full_end < bytes.len() {
            if bytes[full_end] == b'\r' && bytes.get(full_end.saturating_add(1)) == Some(&b'\n') {
                full_end = full_end.saturating_add(2);
            } else {
                full_end = full_end.saturating_add(1);
            }
        }
        lines.push(PhysicalLine {
            content: start..end,
            full: start..full_end,
        });
        start = full_end;
    }
    lines
}

fn normalized_lines(source: &str, lines: &[PhysicalLine]) -> String {
    lines
        .iter()
        .map(|line| &source[line.content.clone()])
        .collect::<Vec<_>>()
        .join("\n")
}

fn whitespace_only(source: &str, line: &PhysicalLine) -> bool {
    source[line.content.clone()]
        .bytes()
        .all(|byte| byte.is_ascii_whitespace())
}

fn strip_leading_layout_whitespace(line: &str, count: usize) -> &str {
    let bytes = line.as_bytes();
    let mut offset = 0_usize;
    while offset < count && offset < bytes.len() && matches!(bytes[offset], b' ' | b'\t' | 0x0c) {
        offset += 1;
    }
    &line[offset..]
}

fn validate_events(
    source: &str,
    format: OutlineFormat,
    headers: &[OutlineHeader],
    lines: &[PhysicalLine],
) -> Result<Vec<usize>, OutlineAdapterError> {
    let mut line_indexes = Vec::with_capacity(headers.len());
    let mut line_cursor = 0_usize;
    for (event, header) in headers.iter().enumerate() {
        if !format.accepts(header.kind) {
            return Err(OutlineAdapterError::WrongFormatKind { event });
        }
        if header.level == 0 {
            return Err(OutlineAdapterError::ZeroLevel { event });
        }
        let ranges_are_valid = header.structural_prefix.slice(source).is_some()
            && header.line_content.slice(source).is_some()
            && header.line.slice(source).is_some()
            && header.line_content.start == header.line.start
            && header.line_content.end <= header.line.end
            && header.structural_prefix.start == header.line.start
            && header.structural_prefix.end <= header.line_content.end
            && header.line.end <= source.len();
        if !ranges_are_valid {
            return Err(OutlineAdapterError::InvalidSourceRange { event });
        }
        while lines
            .get(line_cursor)
            .is_some_and(|line| line.full.start < header.line.start)
        {
            line_cursor = line_cursor.saturating_add(1);
        }
        let Some(line) = lines
            .get(line_cursor)
            .filter(|line| line.full.start == header.line.start)
        else {
            return Err(OutlineAdapterError::InvalidSourceRange { event });
        };
        if line.content != header.line_content.as_range() || line.full != header.line.as_range() {
            return Err(OutlineAdapterError::InvalidSourceRange { event });
        }
        // A later parser event can legitimately begin in a suffix of a line
        // already owned by another AST block. Tine's Document model has one
        // structural block per physical header line, so accepting that shape
        // would overlap receipts and source spans.
        if header.header_start != header.line.start {
            return Err(OutlineAdapterError::NonPhysicalLineHeader { event });
        }
        if let Some(previous_line) = line_indexes.last().copied() {
            if previous_line >= line_cursor {
                return Err(OutlineAdapterError::OverlappingPhysicalLines {
                    first: event.saturating_sub(1),
                    second: event,
                });
            }
        }
        line_indexes.push(line_cursor);
    }
    Ok(line_indexes)
}

fn attach(stack: &mut Vec<(u32, usize, DocBlock)>, roots: &mut Vec<DocBlock>, block: DocBlock) {
    match stack.last_mut() {
        Some((_, _, parent)) => parent.children.push(block),
        None => roots.push(block),
    }
}

fn build_tree(flat: Vec<(u32, DocBlock)>) -> (Vec<DocBlock>, usize) {
    let mut roots = Vec::new();
    let mut stack: Vec<(u32, usize, DocBlock)> = Vec::new();
    let mut maximum_depth = 0_usize;
    for (level, block) in flat {
        while stack.last().is_some_and(|(open, _, _)| *open >= level) {
            let (_, _, done) = stack.pop().expect("checked nonempty");
            attach(&mut stack, &mut roots, done);
        }
        let depth = stack.len().saturating_add(1);
        maximum_depth = maximum_depth.max(depth);
        stack.push((level, depth, block));
    }
    while let Some((_, _, done)) = stack.pop() {
        attach(&mut stack, &mut roots, done);
    }
    (roots, maximum_depth)
}

fn markdown_preamble(
    source: &str,
    lines: &[PhysicalLine],
    first_header_line: usize,
) -> (Option<String>, usize, usize) {
    let pre_lines = &lines[..first_header_line];
    let mut semantic_end = pre_lines.len();
    while semantic_end > 0 && whitespace_only(source, &pre_lines[semantic_end - 1]) {
        semantic_end -= 1;
    }
    let pre_block =
        (semantic_end > 0).then(|| normalized_lines(source, &pre_lines[..semantic_end]));
    let separators = pre_lines.len().saturating_sub(semantic_end);
    if pre_block.is_some() {
        (pre_block, separators, 0)
    } else {
        (None, 1, separators)
    }
}

fn no_header_preamble(source: &str, lines: &[PhysicalLine]) -> Option<String> {
    let mut semantic_end = lines.len();
    while semantic_end > 0
        && lines[semantic_end - 1].content.is_empty()
        && lines[semantic_end - 1].full.end > lines[semantic_end - 1].content.end
    {
        semantic_end -= 1;
    }
    (semantic_end > 0).then(|| normalized_lines(source, &lines[..semantic_end]))
}

pub(crate) fn parse_document(
    source: &str,
    format: OutlineFormat,
) -> Result<ParsedDocument, OutlineAdapterError> {
    #[cfg(test)]
    OUTLINE_PARSE_ATTEMPTS.with(|attempts| attempts.set(attempts.get().saturating_add(1)));
    crate::perf_count::add(crate::perf_count::P::ParseBytes, source.len() as u64);
    let outline = crate::perf_count::timed(crate::perf_count::P::ParseCall, || {
        lsdoc::parse_outline(source, format.lsdoc_name())
    })
    .map_err(|_| OutlineAdapterError::ParserOwnership)?;
    let lines = physical_lines(source);
    let line_indexes = validate_events(source, format, &outline.headers, &lines)?;

    if outline.headers.is_empty() {
        return Ok(ParsedDocument {
            document: Document {
                pre_block: no_header_preamble(source, &lines),
                roots: Vec::new(),
            },
            block_spans: Vec::new(),
            blank_lines_before_blocks: Vec::new(),
            blank_lines_after_preamble: 0,
            leading_blank_lines: 0,
            promoted_heading_layout: None,
            outline_nodes: 0,
            outline_depth: 0,
        });
    }

    let first_header_line = line_indexes[0];
    let (pre_block, blank_lines_after_preamble, leading_blank_lines) = match format {
        OutlineFormat::Markdown => markdown_preamble(source, &lines, first_header_line),
        OutlineFormat::Org => (
            (first_header_line > 0).then(|| normalized_lines(source, &lines[..first_header_line])),
            0,
            0,
        ),
    };

    let mut flat = Vec::with_capacity(outline.headers.len());
    let mut blank_lines_before_blocks = vec![0; outline.headers.len()];
    let mut block_spans = Vec::with_capacity(outline.headers.len());
    for (event, header) in outline.headers.iter().enumerate() {
        let header_line = line_indexes[event];
        let next_header_line = line_indexes
            .get(event.saturating_add(1))
            .copied()
            .unwrap_or(lines.len());
        let mut body_end_line = next_header_line;
        let before_next_header = event.saturating_add(1) < outline.headers.len();
        while body_end_line > header_line.saturating_add(1) {
            let trailing = &lines[body_end_line - 1];
            let structural_blank = if before_next_header || format == OutlineFormat::Markdown {
                whitespace_only(source, trailing)
            } else {
                // Org retains whitespace-bearing terminal body lines, but a
                // pure final newline run belongs to the serializer's existing
                // trailing-newline counter.
                trailing.content.is_empty()
            };
            if !structural_blank {
                break;
            }
            body_end_line -= 1;
        }
        let separators = next_header_line.saturating_sub(body_end_line);
        if let Some(next_blank_lines) = blank_lines_before_blocks.get_mut(event.saturating_add(1)) {
            *next_blank_lines = separators;
        }

        let mut raw = source[header.structural_prefix.end..header.line_content.end].to_owned();
        let markdown_content_indent = match (format, header.kind) {
            (OutlineFormat::Markdown, OutlineHeaderKind::MarkdownDashBullet) => {
                let prefix_len = header
                    .structural_prefix
                    .end
                    .saturating_sub(header.line.start);
                if header
                    .structural_prefix
                    .slice(source)
                    .is_some_and(|prefix| prefix.ends_with("- "))
                {
                    prefix_len
                } else {
                    prefix_len.saturating_add(1)
                }
            }
            _ => 0,
        };
        for line in &lines[header_line.saturating_add(1)..body_end_line] {
            raw.push('\n');
            let content = &source[line.content.clone()];
            if format == OutlineFormat::Markdown {
                raw.push_str(strip_leading_layout_whitespace(
                    content,
                    markdown_content_indent,
                ));
            } else {
                raw.push_str(content);
            }
        }
        let mut block = DocBlock::new(raw);
        block.is_org = format == OutlineFormat::Org;
        flat.push((header.level, block));
        block_spans.push(
            header.line.start
                ..outline
                    .headers
                    .get(event.saturating_add(1))
                    .map_or(source.len(), |next| next.line.start),
        );
    }

    let promoted_heading_layout = match (format, outline.headers.first()) {
        (OutlineFormat::Markdown, Some(first))
            if first.kind == OutlineHeaderKind::MarkdownUnbulletedAtxHeading =>
        {
            if outline
                .headers
                .get(1)
                .is_some_and(|second| second.level > first.level)
            {
                Some(PromotedHeadingLayout::NestedChildren)
            } else {
                Some(PromotedHeadingLayout::UnbulletedRoot)
            }
        }
        _ => None,
    };
    let outline_nodes = flat.len();
    let (roots, outline_depth) = build_tree(flat);
    Ok(ParsedDocument {
        document: Document { pre_block, roots },
        block_spans,
        blank_lines_before_blocks,
        blank_lines_after_preamble,
        leading_blank_lines,
        promoted_heading_layout,
        outline_nodes,
        outline_depth,
    })
}

/// Return a parser-owned insertion point that keeps an unbulleted Markdown
/// heading structural when projection span instrumentation decorates its raw
/// bytes. Tine deliberately does not recover the ATX title boundary itself:
/// inserting immediately before the parser-reported physical-line terminator
/// is sufficient for the temporary marker and leaves the exact source intact
/// when that marker is removed.
pub(crate) fn markdown_unbulleted_heading_line_end(raw: &str) -> Option<usize> {
    let outline = lsdoc::parse_outline(raw, OutlineFormat::Markdown.lsdoc_name()).ok()?;
    let first = outline.headers.first()?;
    (first.kind == OutlineHeaderKind::MarkdownUnbulletedAtxHeading
        && first.header_start == 0
        && first.line.start == 0
        && first.line_content.slice(raw).is_some())
    .then_some(first.line_content.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{self, DocBlock};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Corpus {
        schema: u32,
        provenance: Provenance,
        cases: Vec<CorpusCase>,
    }

    #[derive(Deserialize)]
    struct Provenance {
        repository: String,
        revision: String,
        sources: Vec<String>,
        selection: String,
    }

    #[derive(Deserialize)]
    struct CorpusCase {
        source: String,
        id: String,
        format: String,
        input: String,
    }

    fn semantic_blocks<'a>(document: &'a Document) -> Vec<(Vec<u32>, &'a DocBlock)> {
        fn visit<'a>(
            blocks: &'a [DocBlock],
            locator: &mut Vec<u32>,
            output: &mut Vec<(Vec<u32>, &'a DocBlock)>,
        ) {
            for (position, block) in blocks.iter().enumerate() {
                locator.push(position as u32);
                output.push((locator.clone(), block));
                visit(&block.children, locator, output);
                locator.pop();
            }
        }
        let mut output = Vec::new();
        visit(&document.roots, &mut Vec::new(), &mut output);
        output
    }

    fn event_locators(headers: &[OutlineHeader]) -> Vec<Vec<u32>> {
        let mut roots = 0_u32;
        let mut stack: Vec<(u32, Vec<u32>, u32)> = Vec::new();
        let mut locators = Vec::with_capacity(headers.len());
        for header in headers {
            while stack
                .last()
                .is_some_and(|(level, _, _)| *level >= header.level)
            {
                stack.pop();
            }
            let locator = match stack.last_mut() {
                Some((_, parent, child_count)) => {
                    let mut locator = parent.clone();
                    locator.push(*child_count);
                    *child_count = child_count.saturating_add(1);
                    locator
                }
                None => {
                    let locator = vec![roots];
                    roots = roots.saturating_add(1);
                    locator
                }
            };
            locators.push(locator.clone());
            stack.push((header.level, locator, 0));
        }
        locators
    }

    fn is_expected_refusal(headers: &[OutlineHeader]) -> bool {
        headers
            .iter()
            .any(|header| header.header_start != header.line.start)
            || headers
                .windows(2)
                .any(|pair| pair[0].line.start >= pair[1].line.start)
    }

    fn assert_differential(label: &str, input: &str, format: OutlineFormat) {
        let direct = lsdoc::parse_outline(input, format.lsdoc_name())
            .unwrap_or_else(|error| panic!("{label}: lsdoc ownership failure: {error}"));
        let parsed = match parse_document(input, format) {
            Ok(parsed) => parsed,
            Err(error) => {
                assert!(
                    is_expected_refusal(&direct.headers),
                    "{label}: unexpected adapter refusal {error}; events={:?}",
                    direct.headers
                );
                return;
            }
        };
        assert!(
            !is_expected_refusal(&direct.headers),
            "{label}: adapter accepted an overlapping/non-physical event shape"
        );

        let blocks = semantic_blocks(&parsed.document);
        assert_eq!(
            blocks.len(),
            direct.headers.len(),
            "{label}: event/node count"
        );
        assert_eq!(
            blocks
                .iter()
                .map(|(locator, _)| locator.clone())
                .collect::<Vec<_>>(),
            event_locators(&direct.headers),
            "{label}: parser-level topology"
        );
        assert_eq!(parsed.outline_nodes, direct.headers.len(), "{label}");
        assert_eq!(
            parsed.outline_depth,
            blocks
                .iter()
                .map(|(locator, _)| locator.len())
                .max()
                .unwrap_or(0),
            "{label}: tree depth"
        );

        for (index, ((_, block), header)) in blocks.iter().zip(direct.headers.iter()).enumerate() {
            let expected_first_line = &input[header.structural_prefix.end..header.line_content.end];
            assert_eq!(
                block.raw.split('\n').next().unwrap_or(""),
                expected_first_line,
                "{label}: event {index} kind/prefix mapping for {:?}",
                header.kind
            );
            assert_eq!(
                parsed.block_spans[index],
                header.line.start
                    ..direct
                        .headers
                        .get(index.saturating_add(1))
                        .map_or(input.len(), |next| next.line.start),
                "{label}: event {index} exact source span"
            );
        }

        let canonical = match format {
            OutlineFormat::Markdown => {
                doc::serialize_with(&parsed.document, &doc::SerializeOpts::detect(Some(input)))
            }
            OutlineFormat::Org => crate::org::serialize_org_detect(&parsed.document, Some(input)),
        };
        let reparsed = parse_document(&canonical, format)
            .unwrap_or_else(|error| panic!("{label}: canonical output refused: {error}"));
        if reparsed.document != parsed.document {
            let safely_refused = match format {
                OutlineFormat::Markdown => !doc::markdown_structurally_round_trips(input),
                OutlineFormat::Org => !crate::org::org_editable(input),
            };
            assert!(
                safely_refused,
                "{label}: semantic canonicalization mismatch was not refused"
            );
        }
    }

    #[test]
    fn public_lsdoc_fixed_harness_is_a_permanent_outline_differential() {
        let corpus: Corpus = serde_json::from_str(include_str!(
            "../tests/fixtures/lsdoc-outline/public-harness.json"
        ))
        .expect("vendored public lsdoc outline corpus");
        assert_eq!(corpus.schema, 1);
        assert_eq!(
            corpus.provenance.repository,
            "https://github.com/martinkoutecky/lsdoc"
        );
        assert_eq!(
            corpus.provenance.sources,
            [
                "harness/corpus.json",
                "harness/corpus.blockgate.json",
                "harness/corpus.blocks.json",
                "harness/corpus.inline.json",
                "harness/corpus.mined.json",
                "harness/corpus.org.json",
                "harness/corpus.org.mined.json",
                "harness/reported-divergences.json",
            ]
        );
        assert_eq!(
            corpus.provenance.revision,
            "c79cb059da5b4360ebde2e5fd953fa1f43ddabc3"
        );
        assert!(corpus.provenance.selection.contains("tracked public cases"));
        assert_eq!(corpus.cases.len(), 1_895);

        for case in corpus.cases {
            assert!(
                corpus.provenance.sources.contains(&case.source),
                "{} has unrecorded provenance",
                case.id
            );
            let format = if case.format == "org" {
                OutlineFormat::Org
            } else {
                OutlineFormat::Markdown
            };
            assert_differential(&format!("{}:{}", case.source, case.id), &case.input, format);
        }
    }

    #[test]
    fn large_flat_outline_keeps_semantic_source_order() {
        const BLOCKS: usize = 100_000;
        let mut source = String::with_capacity(BLOCKS.saturating_mul(16));
        for index in 0..BLOCKS {
            use std::fmt::Write as _;
            writeln!(&mut source, "- block {index}").expect("write to String");
        }

        let parsed = parse_document(&source, OutlineFormat::Markdown)
            .expect("large flat parser-owned outline");
        assert_eq!(parsed.outline_nodes, BLOCKS);
        assert_eq!(parsed.outline_depth, 1);
        assert_eq!(parsed.document.roots.len(), BLOCKS);
        assert_eq!(parsed.document.roots[0].raw, "block 0");
        assert_eq!(
            parsed.document.roots[BLOCKS - 1].raw,
            format!("block {}", BLOCKS - 1)
        );
        assert!(parsed
            .document
            .roots
            .iter()
            .all(|block| block.children.is_empty()));
    }

    fn with_line_endings(source: &str, ending: &str) -> String {
        source.split('\n').collect::<Vec<_>>().join(ending)
    }

    #[test]
    fn generated_layout_mutations_preserve_topology_or_refuse_safely() {
        let markdown = concat!(
            "title:: café Ω\n",
            "\n",
            "# Project\n",
            "{i}- child α\n",
            "{i}  wrapped line\n",
            "{i}  \n",
            "{i}  final paragraph\n",
            "- fence owner\n",
            "  ```md\n",
            "  - fenced fake\n",
            "  ```\n",
            "- malformed owned container\n",
            "  #+BEGIN_NOTE\n",
            "  - parser decides this\n",
        );
        let org = concat!(
            "#+TITLE: café Ω\n",
            "\n",
            "* root\n",
            "wrapped line\n",
            "   \n",
            "*** indentation jump\n",
            "#+BEGIN_SRC text\n",
            "* literal fake\n",
            "#+END_SRC\n",
            "** recovered child\n",
            "* malformed owned tail\n",
            "#+BEGIN_NOTE\n",
            "* parser decides this\n",
        );
        for indent in ["\t", "  ", "    "] {
            let markdown = markdown.replace("{i}", indent);
            for (ending_name, ending) in [("lf", "\n"), ("crlf", "\r\n"), ("cr", "\r")] {
                assert_differential(
                    &format!("generated-md-{indent:?}-{ending_name}"),
                    &with_line_endings(&markdown, ending),
                    OutlineFormat::Markdown,
                );
                assert_differential(
                    &format!("generated-org-{indent:?}-{ending_name}"),
                    &with_line_endings(org, ending),
                    OutlineFormat::Org,
                );
            }
        }
    }

    #[test]
    fn overlapping_same_physical_line_events_are_a_safe_refusal() {
        let input = "- $$x$$ # #+BEGIN_NOTE\r\nx\r\n#+END_NOTE";
        let direct = lsdoc::parse_outline(input, "md").expect("lsdoc owns regression");
        assert_eq!(direct.headers.len(), 2);
        assert_eq!(direct.headers[0].line, direct.headers[1].line);
        assert!(matches!(
            parse_document(input, OutlineFormat::Markdown),
            Err(OutlineAdapterError::NonPhysicalLineHeader { event: 1 })
                | Err(OutlineAdapterError::OverlappingPhysicalLines {
                    first: 0,
                    second: 1
                })
        ));
    }

    #[test]
    fn lone_cr_fence_reclassification_is_a_minimized_safe_refusal() {
        let input = "- root\r  ```\r  - fake\r  ```";
        let parsed = parse_document(input, OutlineFormat::Markdown)
            .expect("source events are representable");
        assert_eq!(parsed.document.roots.len(), 1);
        assert_eq!(parsed.document.roots[0].children.len(), 1);

        let canonical =
            doc::serialize_with(&parsed.document, &doc::SerializeOpts::detect(Some(input)));
        let reparsed =
            parse_document(&canonical, OutlineFormat::Markdown).expect("canonical Markdown");
        assert!(reparsed.document.roots[0].children.is_empty());
        assert!(!doc::markdown_structurally_round_trips(input));
    }
}
