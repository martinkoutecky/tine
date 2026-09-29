//! The page-outline authority: which physical lines of a Markdown or Org page
//! open a block, and at which structural level.
//!
//! lsdoc owns the recognition (`lsdoc::parse_outline`, reached through the
//! bounded boundary `crates/lsdoc-block-parse.rs`), exactly as mldoc decides it
//! for OG: dash bullets, unbulleted ATX headings, Org headlines, and every
//! literal region (fences, `#+BEGIN_…` blocks) that hides a look-alike line.
//! This module owns no Markdown or Org grammar. It checks the parser's events
//! against the page's physical lines and maps each one onto Tine's model of
//! one block per header line. `doc::parse` and `org::parse_org` build every
//! [`Document`](crate::doc::Document) from [`headers`]; nothing else decides
//! where a block starts (I-12).
//!
//! Cost: one lsdoc block parse of the page, O(page bytes), plus O(lines).

use crate::doc::DocBlock;
use lsdoc::OutlineHeaderKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutlineFormat {
    Markdown,
    Org,
}

impl OutlineFormat {
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

/// One block-opening line of a page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Header {
    /// Index of the header's physical line in `text.split('\n')`.
    pub(crate) line: usize,
    /// lsdoc's structural level: a later header nests under the nearest
    /// earlier one of a smaller level.
    pub(crate) level: u32,
    /// Leading bytes of the header line that are structure, not block text:
    /// indentation plus `-` (and one space) for a dash bullet, the star run
    /// (and one space) for an Org headline, nothing for an unbulleted heading.
    pub(crate) prefix_len: usize,
    /// Leading tabs/spaces the block's continuation lines are dedented by:
    /// the bullet's content column for a Markdown dash bullet, else 0.
    pub(crate) content_indent: usize,
}

/// Why the page has no representable outline. Each makes `doc::parse` /
/// `org::parse_org` show the whole page as its unparsed preamble text, which a
/// save writes back verbatim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Refusal {
    /// lsdoc did not take ownership of the text, or its quote staircase is
    /// deeper than the bounded boundary admits (malformed imported content).
    Unowned,
    /// Event `event` does not start its own physical line, or shares one
    /// with an earlier event (malformed imported content). Accepting it would
    /// give two blocks the same line, and a save would duplicate or drop text.
    NotOnePerLine { event: usize },
    /// Event `event` has ranges, kind or level outside the parser's contract.
    Invalid { event: usize },
}

/// The block-opening lines of `text`, in source order. `text` must use `\n`
/// line breaks (a CRLF's `\r` may stay in the line; a lone `\r` must already
/// be a `\n`). `Err` names why no outline is representable.
pub(crate) fn headers(text: &str, format: OutlineFormat) -> Result<Vec<Header>, Refusal> {
    let events = crate::render::parse_outline_bounded(text, format == OutlineFormat::Org)
        .ok_or(Refusal::Unowned)?;
    let mut line_starts = vec![0usize];
    line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    let mut out: Vec<Header> = Vec::with_capacity(events.len());
    for (event, header) in events.iter().enumerate() {
        let prefix = header.structural_prefix;
        let valid = format.accepts(header.kind)
            && header.level > 0
            && header.line_content.slice(text).is_some()
            && prefix.slice(text).is_some()
            && prefix.start == header.line.start
            && prefix.end <= header.line_content.end
            && header.line_content.start == header.line.start;
        if !valid {
            return Err(Refusal::Invalid { event });
        }
        let Ok(line) = line_starts.binary_search(&header.line.start) else {
            return Err(Refusal::Invalid { event });
        };
        if header.header_start != header.line.start || out.last().is_some_and(|h| h.line >= line) {
            return Err(Refusal::NotOnePerLine { event });
        }
        let prefix_len = prefix.end - prefix.start;
        let content_indent = match header.kind {
            OutlineHeaderKind::MarkdownDashBullet if text[..prefix.end].ends_with("- ") => {
                prefix_len
            }
            OutlineHeaderKind::MarkdownDashBullet => prefix_len + 1,
            _ => 0,
        };
        out.push(Header {
            line,
            level: header.level,
            prefix_len,
            content_indent,
        });
    }
    Ok(out)
}

/// Headers, or none (with a diagnostic) when the outline is not representable.
pub(crate) fn headers_or_none(text: &str, format: OutlineFormat) -> Vec<Header> {
    headers(text, format).unwrap_or_else(|refusal| {
        crate::diag_line::diagnostic_line(match refusal {
            Refusal::Unowned => "page outline not owned by lsdoc; shown as page text",
            Refusal::NotOnePerLine { .. } => {
                "page outline has two blocks on one line; shown as page text"
            }
            Refusal::Invalid { .. } => "page outline events invalid; shown as page text",
        });
        Vec::new()
    })
}

/// The blocks of `lines` (the page's physical lines), one per header: the
/// header line after its prefix, then each following line up to the next
/// header, dedented by the header's `content_indent`, nested by level.
pub(crate) fn blocks(lines: &[&str], headers: &[Header], is_org: bool) -> Vec<DocBlock> {
    let mut flat = Vec::with_capacity(headers.len());
    for (n, header) in headers.iter().enumerate() {
        let end = headers.get(n + 1).map_or(lines.len(), |next| next.line);
        let mut raw = lines[header.line][header.prefix_len..].to_string();
        for line in &lines[header.line + 1..end] {
            raw.push('\n');
            raw.push_str(strip_layout_ws(line, header.content_indent));
        }
        let mut block = DocBlock::new(raw);
        block.is_org = is_org;
        flat.push((header.level, block));
    }
    build_tree(flat)
}

/// Remove up to `n` leading tabs, spaces or form feeds (mldoc's layout
/// whitespace; master `strip_leading_layout_whitespace`).
fn strip_layout_ws(line: &str, n: usize) -> &str {
    let skip = line
        .bytes()
        .take(n)
        .take_while(|b| matches!(b, b' ' | b'\t' | 0x0c))
        .count();
    &line[skip..]
}

/// Nest a source-order `(level, block)` list: each block goes under the
/// nearest earlier block of a smaller level. Iterative, O(blocks).
fn build_tree(flat: Vec<(u32, DocBlock)>) -> Vec<DocBlock> {
    fn attach(stack: &mut [(u32, DocBlock)], roots: &mut Vec<DocBlock>, done: DocBlock) {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(done),
            None => roots.push(done),
        }
    }
    let mut roots = Vec::new();
    let mut stack: Vec<(u32, DocBlock)> = Vec::new();
    for (level, block) in flat {
        while stack.last().is_some_and(|(open, _)| *open >= level) {
            let (_, done) = stack.pop().expect("checked nonempty");
            attach(&mut stack, &mut roots, done);
        }
        stack.push((level, block));
    }
    while let Some((_, done)) = stack.pop() {
        attach(&mut stack, &mut roots, done);
    }
    roots
}

/// Whether lsdoc reads `line`, alone, as an unbulleted Markdown heading.
pub(crate) fn is_unbulleted_heading_line(line: &str) -> bool {
    !line.contains(['\n', '\r'])
        && crate::render::parse_outline_bounded(line, false).is_some_and(|events| {
            matches!(events.as_slice(), [only]
                if only.kind == OutlineHeaderKind::MarkdownUnbulletedAtxHeading
                    && only.header_start == 0)
        })
}
