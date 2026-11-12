//! Tine's one-block lsdoc boundary.
//!
//! mldoc/lsdoc classify a Markdown line that starts with an inline-code span
//! containing `::` as a property drawer before inline parsing. Tine deliberately
//! treats that narrow shape as code. We replace the separators with equal-width
//! bytes for the parse, then restore the complete code node from the source.
//! Equal width keeps every parser-owned source span valid; restoring by span (not
//! by a supposedly unique sentinel) makes the transform collision-proof.

use lsdoc::ast::{Block, Inline, ListItem, Projection, Span};

const MAX_PARSED_TREE_DEPTH: usize = 1024;
/// Source-side ceiling on a line's quote staircase for free-text doors; the
/// same 128-level ceiling the page admission applies (`PARSE_INPUT_MAX_DEPTH`).
#[allow(dead_code)] // each crate that includes this file uses a subset
pub const SOURCE_QUOTE_DEPTH_MAX: usize = 128;

/// `>` markers that open a quote staircase on one line: after leading spaces
/// and any list markers, each `>` optionally followed by spaces. lsdoc parses
/// every level by recursing *during the parse* (`markdown_blockquote_sequence`
/// re-parses the stripped body), so a tree check after the parse is too late:
/// ~1,300 levels overflow a 2 MiB debug stack before any tree exists (I-22).
/// Counts markers, an upper bound on levels (lsdoc may strip two per level).
pub fn line_quote_depth(line: &str) -> usize {
    let spaces = |bytes: &[u8], mut at: usize| {
        while matches!(bytes.get(at), Some(b' ' | b'\t' | b'\x0c' | b'\x1a')) {
            at += 1;
        }
        at
    };
    let bytes = line.as_bytes();
    let mut at = spaces(bytes, 0);
    loop {
        let digits = bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let marker_end = match bytes.get(at + digits) {
            Some(b'.' | b')') if digits > 0 => at + digits + 1,
            Some(b'-' | b'+' | b'*') if digits == 0 => at + 1,
            _ => break,
        };
        if !matches!(bytes.get(marker_end), Some(b' ' | b'\t')) {
            break;
        }
        at = spaces(bytes, marker_end);
    }
    let mut depth = 0;
    while bytes.get(at) == Some(&b'>') {
        depth += 1;
        at = spaces(bytes, at + 1);
    }
    depth
}

fn quote_depth_within(text: &str, limit: usize) -> bool {
    text.split(['\n', '\r'])
        .all(|line| line_quote_depth(line) <= limit)
}

/// Refuse before lsdoc recurses: the block doors keep their panic contract
/// (the page parse isolates it), sized to the tree bound so no input that
/// parsed before is refused by this check alone.
fn admit_block_source(raw: &str) {
    if !quote_depth_within(raw, MAX_PARSED_TREE_DEPTH) {
        panic!("lsdoc quote nesting exceeds 1024 levels");
    }
}

// lsdoc's parser uses heap frames, but its returned AST has recursive drop.
// Check the *actual* AST before any of Tine's recursive consumers see it.
fn parsed_tree_within_limit(blocks: &[Block], limit: usize) -> bool {
    enum Node<'a> {
        Block(&'a Block, usize),
        Item(&'a ListItem, usize),
        Inline(&'a Inline, usize),
    }
    let mut todo: Vec<_> = blocks.iter().map(|block| Node::Block(block, 1)).collect();
    while let Some(node) = todo.pop() {
        match node {
            Node::Block(block, depth) => {
                if depth > limit {
                    return false;
                }
                match block {
                    Block::Quote { children, .. } | Block::Custom { children, .. } => {
                        todo.extend(children.iter().map(|child| Node::Block(child, depth + 1)));
                    }
                    Block::List { items, .. } => {
                        todo.extend(items.iter().map(|item| Node::Item(item, depth + 1)));
                    }
                    Block::Paragraph { inline, .. }
                    | Block::Heading { inline, .. }
                    | Block::Bullet { inline, .. }
                    | Block::FootnoteDef { inline, .. } => {
                        todo.extend(inline.iter().map(|item| Node::Inline(item, depth + 1)));
                    }
                    _ => {}
                }
            }
            Node::Item(item, depth) => {
                if depth > limit {
                    return false;
                }
                todo.extend(item.items.iter().map(|child| Node::Item(child, depth + 1)));
                todo.extend(
                    item.content
                        .iter()
                        .map(|child| Node::Block(child, depth + 1)),
                );
                todo.extend(item.name.iter().map(|child| Node::Inline(child, depth + 1)));
            }
            Node::Inline(item, depth) => {
                if depth > limit {
                    return false;
                }
                let children = match item {
                    Inline::Emphasis { children, .. }
                    | Inline::Subscript { children, .. }
                    | Inline::Superscript { children, .. }
                    | Inline::Tag { children, .. } => Some(children),
                    Inline::Link { label, .. } => Some(label),
                    Inline::Fnref { definition, .. } => Some(definition),
                    _ => None,
                };
                if let Some(children) = children {
                    todo.extend(children.iter().map(|child| Node::Inline(child, depth + 1)));
                }
            }
        }
    }
    true
}

// Drain children before dropping a refused tree. Ordinary `Drop` on a deep
// `Block`/`ListItem`/`Inline` chain would overflow the parser thread's stack.
fn drop_parsed_tree(blocks: Vec<Block>) {
    enum Node {
        Block(Block),
        Item(ListItem),
        Inline(Inline),
    }
    let mut todo: Vec<_> = blocks.into_iter().map(Node::Block).collect();
    while let Some(node) = todo.pop() {
        match node {
            Node::Block(mut block) => match &mut block {
                Block::Quote { children, .. } | Block::Custom { children, .. } => {
                    todo.extend(std::mem::take(children).into_iter().map(Node::Block));
                }
                Block::List { items, .. } => {
                    todo.extend(std::mem::take(items).into_iter().map(Node::Item));
                }
                Block::Paragraph { inline, .. }
                | Block::Heading { inline, .. }
                | Block::Bullet { inline, .. }
                | Block::FootnoteDef { inline, .. } => {
                    todo.extend(std::mem::take(inline).into_iter().map(Node::Inline));
                }
                _ => {}
            },
            Node::Item(mut item) => {
                todo.extend(std::mem::take(&mut item.items).into_iter().map(Node::Item));
                todo.extend(
                    std::mem::take(&mut item.content)
                        .into_iter()
                        .map(Node::Block),
                );
                todo.extend(std::mem::take(&mut item.name).into_iter().map(Node::Inline));
            }
            Node::Inline(mut item) => {
                let children = match &mut item {
                    Inline::Emphasis { children, .. }
                    | Inline::Subscript { children, .. }
                    | Inline::Superscript { children, .. }
                    | Inline::Tag { children, .. } => Some(children),
                    Inline::Link { label, .. } => Some(label),
                    Inline::Fnref { definition, .. } => Some(definition),
                    _ => None,
                };
                if let Some(children) = children {
                    todo.extend(std::mem::take(children).into_iter().map(Node::Inline));
                }
            }
        }
    }
}

fn bounded_blocks(blocks: Vec<Block>) -> Vec<Block> {
    if parsed_tree_within_limit(&blocks, MAX_PARSED_TREE_DEPTH) {
        blocks
    } else {
        drop_parsed_tree(blocks);
        panic!("lsdoc parsed tree exceeds 1024 levels");
    }
}

fn bounded_projection(mut projection: Projection) -> Projection {
    projection.blocks = bounded_blocks(std::mem::take(&mut projection.blocks));
    projection
}

#[derive(Debug)]
struct ProtectedCode {
    start: usize,
    end: usize,
    text: String,
}

struct Prepared {
    input: String,
    protected: Vec<ProtectedCode>,
}

enum Preparation {
    /// The ordinary path: exactly the re-bulleted input that lsdoc requires.
    /// In particular, there is no cloned source buffer or eager fallback.
    Plain(String),
    /// The exceptional line-leading-code path. `original` is kept borrowed by
    /// the caller and its fallback input is allocated only if restoration fails.
    Protected(Prepared),
}

#[derive(Debug)]
struct Candidate {
    opener: usize,
    ticks: usize,
    close: usize,
}

fn run_len(bytes: &[u8], at: usize, byte: u8) -> usize {
    bytes[at..]
        .iter()
        .take_while(|candidate| **candidate == byte)
        .count()
}

fn matching_code_close(bytes: &[u8], opener: usize, ticks: usize) -> Option<usize> {
    let mut at = opener + ticks;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let run = run_len(bytes, at, b'`');
        if run == ticks && at > opener + ticks {
            return Some(at);
        }
        at += run;
    }
    None
}

fn fence_marker(line: &[u8]) -> Option<(u8, usize)> {
    let indent = line.iter().take_while(|byte| **byte == b' ').count();
    if indent > 3 || indent >= line.len() {
        return None;
    }
    let marker = line[indent];
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let len = run_len(line, indent, marker);
    (len >= 3).then_some((marker, len))
}

fn discover_markdown_candidates(trimmed: &str) -> Vec<Candidate> {
    let bytes = trimmed.as_bytes();
    let mut candidates = Vec::new();
    let mut line_start = 0;
    let mut covered_until = 0;
    let mut fence: Option<(u8, usize)> = None;

    while line_start < bytes.len() {
        let line_end = bytes[line_start..]
            .iter()
            .position(|byte| *byte == b'\n' || *byte == b'\r')
            .map(|offset| line_start + offset)
            .unwrap_or(bytes.len());
        let line = &bytes[line_start..line_end];

        if line_start < covered_until {
            // Fence-looking continuation text is still inside the selected code
            // span and must not alter block-fence state for following lines.
        } else if let Some((marker, minimum)) = fence {
            if fence_marker(line)
                .is_some_and(|(candidate, len)| candidate == marker && len >= minimum)
            {
                fence = None;
            }
        } else if let Some(marker) = fence_marker(line) {
            fence = Some(marker);
        } else {
            let leading = line
                .iter()
                .take_while(|byte| **byte == b' ' || **byte == b'\t')
                .count();
            let opener = line_start + leading;
            if opener < line_end && bytes[opener] == b'`' {
                let ticks = run_len(bytes, opener, b'`');
                // Three or more line-leading ticks are a fenced-code opener. The
                // one-block parser supports one- and two-tick inline code spans.
                if ticks <= 2 {
                    if let Some(close) = matching_code_close(bytes, opener, ticks) {
                        let content_start = opener + ticks;
                        let has_separator = bytes[content_start..close]
                            .windows(2)
                            .any(|window| window == b"::");
                        if has_separator {
                            candidates.push(Candidate {
                                opener,
                                ticks,
                                close,
                            });
                            covered_until = close + ticks;
                        }
                    }
                }
            }
        }

        if line_end == bytes.len() {
            break;
        }
        line_start = line_end
            + if bytes[line_end] == b'\r' && bytes.get(line_end + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
    }

    candidates
}

fn prepare_markdown(trimmed: &str) -> Preparation {
    // Most blocks have neither a backtick nor a property separator. Keep their
    // pre-fix cost shape: one required re-bulleted String and no protection
    // census, source clone, restoration walk, or fallback. This cheap immutable
    // prefilter also keeps ordinary properties (`key:: value`) on the fast path.
    if !trimmed.contains('`') || !trimmed.contains("::") {
        return Preparation::Plain(format!("- {trimmed}"));
    }

    // Discovery is immutable. Only an actual line-leading inline-code candidate
    // earns the exceptional copy/mutation work below.
    let candidates = discover_markdown_candidates(trimmed);
    if candidates.is_empty() {
        return Preparation::Plain(format!("- {trimmed}"));
    }

    let mut protected_input = trimmed.as_bytes().to_vec();
    let mut protected = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let content_start = candidate.opener + candidate.ticks;
        let mut at = content_start;
        while at < candidate.close {
            if protected_input[at] == b':'
                && at + 1 < candidate.close
                && protected_input[at + 1] == b':'
            {
                protected_input[at] = b';';
                protected_input[at + 1] = b';';
                at += 2;
            } else {
                // Hide non-closing tick runs and EOLs from lsdoc's narrower
                // inline scanners. Every byte keeps its width and the full code
                // text is restored by exact span below.
                if protected_input[at] == b'`' {
                    protected_input[at] = b'~';
                } else if protected_input[at] == b'\n' || protected_input[at] == b'\r' {
                    protected_input[at] = b' ';
                }
                at += 1;
            }
        }
        protected.push(ProtectedCode {
            // The prepared outline bullet contributes `- `.
            start: candidate.opener + 2,
            end: candidate.close + candidate.ticks + 2,
            text: trimmed[content_start..candidate.close].to_string(),
        });
    }

    let protected_text =
        String::from_utf8(protected_input).expect("ASCII replacement preserves UTF-8");
    Preparation::Protected(Prepared {
        input: format!("- {protected_text}"),
        protected,
    })
}

fn prepare(raw: &str, is_org: bool) -> Preparation {
    let trimmed = raw.trim_start();
    if is_org {
        Preparation::Plain(format!("* {trimmed}"))
    } else {
        prepare_markdown(trimmed)
    }
}

fn restore_inlines(inline: &mut [Inline], protected: &[ProtectedCode], restored: &mut [bool]) {
    for node in inline {
        match node {
            Inline::Code {
                text,
                span: Some(Span(start, end)),
            } => {
                if let Ok(index) =
                    protected.binary_search_by_key(&(*start, *end), |code| (code.start, code.end))
                {
                    *text = protected[index].text.clone();
                    restored[index] = true;
                }
            }
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. }
            | Inline::Tag { children, .. } => restore_inlines(children, protected, restored),
            Inline::Link { label, .. } => restore_inlines(label, protected, restored),
            _ => {}
        }
    }
}

fn restore_list_items(items: &mut [ListItem], protected: &[ProtectedCode], restored: &mut [bool]) {
    for item in items {
        restore_blocks(&mut item.content, protected, restored);
        restore_list_items(&mut item.items, protected, restored);
        restore_inlines(&mut item.name, protected, restored);
    }
}

fn restore_blocks(blocks: &mut [Block], protected: &[ProtectedCode], restored: &mut [bool]) {
    if protected.is_empty() {
        return;
    }
    for block in blocks {
        match block {
            Block::Paragraph { inline, .. }
            | Block::Heading { inline, .. }
            | Block::Bullet { inline, .. }
            | Block::FootnoteDef { inline, .. } => restore_inlines(inline, protected, restored),
            Block::List { items, .. } => restore_list_items(items, protected, restored),
            Block::Quote { children, .. } | Block::Custom { children, .. } => {
                restore_blocks(children, protected, restored)
            }
            Block::Table { header, rows, .. } => {
                if let Some(header) = header {
                    for cell in header {
                        restore_inlines(cell, protected, restored);
                    }
                }
                for row in rows {
                    for cell in row {
                        restore_inlines(cell, protected, restored);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Where a block's parsed text starts: the block boundary's leading trim.
pub(crate) fn block_lead(raw: &str) -> usize {
    raw.len() - raw.trim_start().len()
}
/// A prepared single-block offset (after the `* `/`- ` wrapper) in raw.
pub(crate) fn raw_offset(raw: &str, at: usize) -> usize {
    (at.saturating_sub(2) + block_lead(raw)).min(raw.len())
}
/// The end of the first line after the leading trim, after its newline.
pub(crate) fn first_line_end(raw: &str) -> usize {
    let lead = block_lead(raw);
    raw[lead..].find('\n').map_or(raw.len(), |p| lead + p + 1)
}
/// Where lsdoc's headline (the title with its line terminator) ends, when that
/// is not [`first_line_end`]: an empty title before the properties or drawer
/// lsdoc reads at the title position (`*` then `:PROPERTIES:`, `- id:: x`),
/// where the block's metadata begins. Other nodes at the title position (a
/// quote, `#+TITLE: x`) are the block's first line, so that line stays its
/// title line. A nonempty headline owns the rest of its line when only blanks
/// follow (lsdoc ends a bare `TODO ` before its trailing blanks, and a CRLF
/// page keeps each line's CR in the raw), so rows never split the title line;
/// a headline that already ends with its newline stays where it is, and one
/// that ends inside the next line's indentation ends at that line's start.
pub(crate) fn headline_end(raw: &str, blocks: &[Block]) -> Option<usize> {
    let (Block::Bullet { span, .. } | Block::Heading { span, .. }) = blocks.first()? else {
        return None;
    };
    let mut at = raw_offset(raw, span.as_ref()?.1);
    let drawer = matches!(
        blocks.get(1),
        Some(Block::Properties { .. } | Block::Drawer { .. })
    );
    if at == block_lead(raw) && !drawer {
        return None;
    }
    if !raw.is_char_boundary(at) {
        at = raw.len();
    }
    // The block's parsed text starts after its leading trim, so an empty
    // headline before an indented drawer ends after that line's indentation;
    // the indentation is the metadata line's own.
    if let Some(nl) = raw[..at].rfind('\n') {
        if raw[nl + 1..at].trim().is_empty() {
            at = nl + 1;
        }
    }
    if at > block_lead(raw) && !raw[..at].ends_with('\n') {
        let line_end = raw[at..].find('\n').map_or(raw.len(), |p| at + p + 1);
        if raw[at..line_end].trim().is_empty() {
            at = line_end;
        }
    }
    (at != first_line_end(raw)).then_some(at)
}

pub fn parse_block(raw: &str, is_org: bool) -> Vec<Block> {
    let mut blocks = parse_block_inner(raw, is_org);
    restore_bare_task(raw, is_org, &mut blocks);
    blocks
}

fn restore_bare_task(raw: &str, is_org: bool, blocks: &mut [Block]) {
    // lsdoc (like mldoc) accepts a bare task marker only at end of input, so
    // a parser-owned row after the title turns `TODO` into the plain word.
    // Structural edits add and remove exactly those rows (an id, a fold, a
    // SCHEDULED line, the LOGBOOK), so the task state must not depend on them
    // (I-4): GH741 (a LOGBOOK anywhere after the title) and
    // REG-BARE-TASK-HIDDEN-ROWS (the title followed only by such rows).
    // Repaired at the shared AST boundary so facets, body rendering and writers
    // agree; lsdoc, rather than another spelling grammar, decides the marker.
    let logbook =
        |b: &Block| matches!(b, Block::Drawer { name, .. } if name.eq_ignore_ascii_case("LOGBOOK"));
    let rest = blocks.get(1..).unwrap_or_default();
    // Blank paragraphs beside the rows (`TODO\n\nid:: x`, trailing blank
    // lines, a CRLF raw's final CR) do not count either: removing the last row
    // must leave the task a task.
    if !rest
        .iter()
        .all(|b| logbook(b) || parser_owned_row(b) || blank_paragraph(b))
        && !rest.iter().any(logbook)
    {
        return;
    }
    let Some(
        Block::Bullet {
            marker: marker @ None,
            inline,
            span: Some(Span(head, _)),
            ..
        }
        | Block::Heading {
            marker: marker @ None,
            inline,
            span: Some(Span(head, _)),
            ..
        },
    ) = blocks.first_mut()
    else {
        return;
    };
    // Only a lone marker-shaped word can become a bare task; anything else
    // skips the second parse.
    let [Inline::Plain {
        text,
        span: Some(Span(_, word_end)),
        ..
    }] = inline.as_slice()
    else {
        return;
    };
    if text.is_empty() || !text.bytes().all(|c| c.is_ascii_uppercase() || c == b'-') {
        return;
    }
    // The headline's source up to the word, where lsdoc located them (`# TODO`
    // stays a heading). At the end of input lsdoc has already decided it.
    let head = raw_offset(raw, *head);
    let end = raw_offset(raw, *word_end);
    if end == raw.len() || !raw.is_char_boundary(head) || !raw.is_char_boundary(end) {
        return;
    }
    let title = &raw[head..end];
    let title_blocks = parse_block_inner(title, is_org);
    if let Some(
        Block::Bullet {
            marker: Some(accepted),
            inline: title_inline,
            ..
        }
        | Block::Heading {
            marker: Some(accepted),
            inline: title_inline,
            ..
        },
    ) = title_blocks.first()
    {
        // Continuation prose before the drawer must remain prose. Only the
        // exact bare-marker header can become an empty task body.
        if title_inline.is_empty()
            && matches!(inline.as_slice(), [Inline::Plain { text, .. }] if text == accepted)
        {
            *marker = Some(accepted.clone());
            inline.clear();
        }
    }
}

/// A block-level node that is a row Tine's structural edits own rather than
/// body text: a property drawer or `key::` lines, or a planning line
/// (`SCHEDULED:`/`DEADLINE:`/`CLOSED:` timestamps only).
fn parser_owned_row(block: &Block) -> bool {
    match block {
        Block::Properties { .. } => true,
        Block::Paragraph { inline, .. } => {
            inline.iter().any(|i| matches!(i, Inline::Timestamp { .. }))
                && inline.iter().all(|i| match i {
                    Inline::Timestamp { ts, .. } => {
                        matches!(ts.as_str(), "Scheduled" | "Deadline" | "Closed")
                    }
                    Inline::Break { .. } => true,
                    Inline::Plain { text, .. } => text.trim().is_empty(),
                    _ => false,
                })
        }
        _ => false,
    }
}

fn blank_paragraph(block: &Block) -> bool {
    matches!(block, Block::Paragraph { inline, .. } if inline.iter().all(|i| match i {
        Inline::Break { .. } => true,
        Inline::Plain { text, .. } => text.trim().is_empty(),
        _ => false,
    }))
}

// Parses per operation, pinned by `preparation_tests` (PC-3).
#[cfg(test)]
thread_local! {
    static PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn parse_block_inner(raw: &str, is_org: bool) -> Vec<Block> {
    #[cfg(test)]
    PARSES.with(|n| n.set(n.get() + 1));
    admit_block_source(raw);
    let prepared = match prepare(raw, is_org) {
        Preparation::Plain(input) => {
            return bounded_blocks(lsdoc::parse(&input, if is_org { "org" } else { "md" }))
        }
        Preparation::Protected(prepared) => prepared,
    };
    let mut blocks = lsdoc::parse(&prepared.input, "md");
    let mut restored = vec![false; prepared.protected.len()];
    restore_blocks(&mut blocks, &prepared.protected, &mut restored);
    if restored.iter().all(|value| *value) {
        bounded_blocks(blocks)
    } else {
        // Fail closed: an unanticipated parser shape may retain lsdoc's original
        // classification, but transformed bytes must never leak into the AST.
        bounded_blocks(lsdoc::parse(&format!("- {}", raw.trim_start()), "md"))
    }
}

#[allow(dead_code)] // the wasm crate needs block parsing only; tine-core uses the full projection
pub fn parse_projection(raw: &str, is_org: bool) -> Projection {
    let mut projection = parse_projection_inner(raw, is_org);
    restore_bare_task(raw, is_org, &mut projection.blocks);
    projection
}

fn parse_projection_inner(raw: &str, is_org: bool) -> Projection {
    #[cfg(test)]
    PARSES.with(|n| n.set(n.get() + 1));
    admit_block_source(raw);
    let prepared = match prepare(raw, is_org) {
        Preparation::Plain(input) => {
            return bounded_projection(lsdoc::parse_format(
                &input,
                if is_org { "org" } else { "md" },
            ))
        }
        Preparation::Protected(prepared) => prepared,
    };
    let mut projection = lsdoc::parse_format(&prepared.input, "md");
    let mut restored = vec![false; prepared.protected.len()];
    restore_blocks(&mut projection.blocks, &prepared.protected, &mut restored);
    if restored.iter().all(|value| *value) {
        bounded_projection(projection)
    } else {
        bounded_projection(lsdoc::parse_format(
            &format!("- {}", raw.trim_start()),
            "md",
        ))
    }
}

/// Parse free text that is NOT a block body (a property value, a whole file):
/// no re-bulleting. A quote staircase deeper than [`SOURCE_QUOTE_DEPTH_MAX`]
/// is refused before lsdoc recurses, and a tree deeper than the block bound is
/// drained iteratively; either way `None`, and the caller degrades (no refs
/// from that value, an empty document) instead of aborting (I-22). Text lsdoc
/// does not yet own (an ownership gap in v2, e.g. `"- s::\r"`) is likewise
/// `None` through the strict hook rather than the panicking `parse_format`: in
/// the wasm build (`panic = "abort"`) that panic is an `unreachable` trap with
/// no message and no way to isolate it. Every
/// production `lsdoc::parse*` call is in this file
/// (`tine-core/tests/lsdoc_parse_boundary.rs`).
#[allow(dead_code)] // each crate that includes this file uses a subset
pub fn parse_text_bounded(text: &str, format: &str) -> Option<Projection> {
    if !quote_depth_within(text, SOURCE_QUOTE_DEPTH_MAX) {
        return None;
    }
    let mut projection =
        lsdoc::__try_parse_format_v2(text, if format == "org" { "org" } else { "md" })?;
    let blocks = std::mem::take(&mut projection.blocks);
    if parsed_tree_within_limit(&blocks, MAX_PARSED_TREE_DEPTH) {
        projection.blocks = blocks;
        Some(projection)
    } else {
        drop_parsed_tree(blocks);
        None
    }
}

/// A whole page's outline headers ([`lsdoc::parse_outline`]) under the
/// free-text bound: `None` when the page's quote staircase is deeper than
/// [`SOURCE_QUOTE_DEPTH_MAX`] (refused before lsdoc recurses) or lsdoc does not
/// take ownership of the text. lsdoc drops its own parse tree stack-safely.
#[allow(dead_code)] // each crate that includes this file uses a subset
pub fn parse_outline_bounded(text: &str, format: &str) -> Option<Vec<lsdoc::OutlineHeader>> {
    if !quote_depth_within(text, SOURCE_QUOTE_DEPTH_MAX) {
        return None;
    }
    lsdoc::parse_outline(text, format)
        .ok()
        .map(|outline| outline.headers)
}

/// [`lsdoc::inline`] under the same bound, for inline-only readers.
#[allow(dead_code)] // each crate that includes this file uses a subset
pub fn parse_inline_bounded(text: &str, format: &str) -> Option<Vec<Inline>> {
    let wrapped = vec![Block::Paragraph {
        inline: lsdoc::inline(text, format),
        span: None,
    }];
    if !parsed_tree_within_limit(&wrapped, MAX_PARSED_TREE_DEPTH) {
        drop_parsed_tree(wrapped);
        return None;
    }
    match wrapped.into_iter().next() {
        Some(Block::Paragraph { inline, .. }) => Some(inline),
        _ => None,
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;

    #[test]
    fn gh741_backend_projection_and_render_ast_agree_on_bare_clocked_tasks() {
        for is_org in [false, true] {
            let raw = "DOING\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 09:00:00]\n:END:";
            let blocks = parse_block(raw, is_org);
            let projection = parse_projection(raw, is_org);
            assert_eq!(
                serde_json::to_value(&blocks).unwrap(),
                serde_json::to_value(&projection.blocks).unwrap()
            );
            assert!(
                matches!(blocks.first(), Some(Block::Bullet { marker: Some(m), inline, .. } | Block::Heading { marker: Some(m), inline, .. }) if m == "DOING" && inline.is_empty())
            );
        }
    }

    #[test]
    fn a_bare_task_before_parser_owned_rows_only_is_a_task() {
        let task = |raw: &str, is_org: bool| {
            let blocks = parse_block(raw, is_org);
            assert_eq!(
                serde_json::to_value(&blocks).unwrap(),
                serde_json::to_value(&parse_projection(raw, is_org).blocks).unwrap(),
                "{raw:?}"
            );
            match blocks.first() {
                Some(
                    Block::Bullet { marker, inline, .. } | Block::Heading { marker, inline, .. },
                ) => marker.clone().filter(|_| inline.is_empty()),
                _ => None,
            }
        };
        let todo = Some("TODO".to_string());
        for is_org in [false, true] {
            for raw in [
                "TODO",
                "TODO\nSCHEDULED: <2026-11-12 Thu>",
                "TODO\nSCHEDULED: <2026-11-12 Thu> DEADLINE: <2026-11-13 Fri>",
                "TODO\n:PROPERTIES:\n:id: 6679-abc\n:END:",
                "TODO\nSCHEDULED: <2026-11-12 Thu>\n:PROPERTIES:\n:id: 6679-abc\n:END:",
            ] {
                assert_eq!(task(raw, is_org), todo, "{is_org} {raw:?}");
            }
            // Body text after the title stays text, as in Logseq.
            for raw in [
                "TODO\nbody",
                "TODO\n:PROPERTIES:\n:id: 6679-abc\n:END:\nbody",
                "TODO\nSCHEDULED: <2026-11-12 Thu> later",
            ] {
                assert_eq!(task(raw, is_org), None, "{is_org} {raw:?}");
            }
        }
        assert_eq!(task("TODO\nid:: 6679-abc", false), todo);
        // Review r1: CRLF titles and blank lines beside the rows.
        for (raw, is_org) in [
            ("TODO\r", true),
            ("TODO\r", false),
            ("TODO\r\n:PROPERTIES:\r\n:id: x\r\n:END:", true),
            ("TODO\r\nid:: x", false),
            ("TODO\n\nid:: x", false),
            ("TODO\n:PROPERTIES:\n:id: x\n:END:\n\n", true),
            ("TODO\n:PROPERTIES:\n:id: x\n:END:\n\n", false),
        ] {
            assert_eq!(task(raw, is_org), todo, "{is_org} {raw:?}");
        }
        // Review r2: removing the last row leaves the title with blank lines,
        // which must stay a task in both formats and both line endings.
        for (raw, is_org) in [
            ("TODO\n", true),
            ("TODO\n", false),
            ("TODO\n\n", true),
            ("TODO\n\n", false),
            ("TODO\r\n\r\n", true),
        ] {
            assert_eq!(task(raw, is_org), todo, "{is_org} {raw:?}");
        }
        // Review r4: leading blank lines or spaces before the title, which the
        // block boundary skips, are skipped by the repair too.
        for (raw, is_org) in [
            ("\nTODO\nid:: x", false),
            ("\n\nTODO\ncollapsed:: true", false),
            ("  TODO\nid:: x", false),
            ("\nTODO\n:PROPERTIES:\n:id: x\n:END:", true),
            ("\r\nTODO\r\nid:: x", false),
        ] {
            assert_eq!(task(raw, is_org), todo, "{is_org} {raw:?}");
        }
        assert_eq!(task("\nTODO\nbody\nid:: x", false), None);
        // Review r5: structural edits put rows after the same title line, so a
        // leading-blank bare task stays one through an id, a schedule and a
        // user property, in both formats.
        use crate::block_regions::{edit, Edit};
        let id = || Edit::Property {
            key: "id".into(),
            value: Some("6679-abc".into()),
        };
        let scheduled = || Edit::Planning {
            which: "Scheduled".into(),
            value: Some("<2026-11-12 Thu>".into()),
        };
        let prop = || Edit::Property {
            key: "owner".into(),
            value: Some("me".into()),
        };
        for (raw, is_org, request) in [
            ("\nTODO", true, id as fn() -> Edit),
            ("\nTODO", true, scheduled),
            ("\nTODO", false, id),
            ("\nTODO", false, scheduled),
            ("\nTODO", false, prop),
            ("\n\nTODO\n", false, id),
        ] {
            let after = edit(raw, is_org, request()).unwrap();
            assert!(
                after.trim_start().starts_with("TODO\n"),
                "{is_org} {raw:?} -> {after:?}"
            );
            assert_eq!(task(&after, is_org), todo, "{is_org} {raw:?} -> {after:?}");
        }
        for (raw, is_org) in [("TODO\n\nbody\nid:: x", false), ("TODO\nbody\n\n", true)] {
            assert_eq!(task(raw, is_org), None, "{is_org} {raw:?}");
        }
    }

    /// PC-3 (review r3, r6 follow-up): parses per read. Every block costs one
    /// parse; only a lone marker-shaped word that lsdoc did not see at the end
    /// of input, with nothing but rows or blank lines after it, costs a second
    /// parse of that word. A lone uppercase title (`INTRODUCTION`) costs one.
    #[test]
    fn a_read_parses_once_and_a_bare_task_repair_parses_its_word_once_more() {
        let parses = |raw: &str, is_org: bool| {
            super::PARSES.with(|n| n.set(0));
            parse_block(raw, is_org);
            let block = super::PARSES.with(|n| n.replace(0));
            parse_projection(raw, is_org);
            let projection = super::PARSES.with(|n| n.replace(0));
            assert_eq!(block, projection, "{is_org} {raw:?}");
            block
        };
        for is_org in [false, true] {
            for raw in [
                "INTRODUCTION",
                "INTRODUCTION\nbody",
                "TODO",
                "TODO x\nid:: y",
                "TODO\nbody",
                "plain\nid:: y",
                "\nTODO",
                "TODO [#A]\nid:: y",
            ] {
                assert_eq!(parses(raw, is_org), 1, "{is_org} {raw:?}");
            }
            for raw in [
                "TODO\n:PROPERTIES:\n:id: x\n:END:",
                "TODO\r",
                "TODO\n",
                "\nTODO\n:PROPERTIES:\n:id: x\n:END:",
            ] {
                assert_eq!(parses(raw, is_org), 2, "{is_org} {raw:?}");
            }
        }
    }

    /// Review r6: the title/metadata boundary is lsdoc's headline, so an empty
    /// Org title before its id drawer keeps the drawer as its own; Markdown and
    /// CRLF twins likewise, and rows after a title go after lsdoc's headline.
    #[test]
    fn rows_go_after_the_parsed_headline() {
        use crate::block_regions::{edit, parse, Edit};
        let fold = || Edit::Property {
            key: "collapsed".into(),
            value: Some("true".into()),
        };
        for (raw, is_org, after) in [
            (
                "\n:PROPERTIES:\n:id: x\n:END:",
                true,
                "\n:PROPERTIES:\n:id: x\n:collapsed: true\n:END:",
            ),
            (
                "\r\n:PROPERTIES:\r\n:id: x\r\n:END:\r",
                true,
                "\r\n:PROPERTIES:\r\n:id: x\r\n:collapsed: true\r\n:END:\r",
            ),
            ("x\r", true, "x\r\n:PROPERTIES:\n:collapsed: true\n:END:"),
            ("x\r", false, "x\r\ncollapsed:: true"),
            (
                "\nTODO",
                true,
                "\nTODO\n:PROPERTIES:\n:collapsed: true\n:END:",
            ),
            ("\nTODO", false, "\nTODO\ncollapsed:: true"),
            (
                "title\nbody",
                true,
                "title\n:PROPERTIES:\n:collapsed: true\n:END:\nbody",
            ),
        ] {
            assert_eq!(
                parse(raw, is_org).id.map(|p| p.value),
                (raw.contains("id:")).then(|| "x".to_string()),
                "{raw:?}"
            );
            assert_eq!(
                edit(raw, is_org, fold()).unwrap(),
                after,
                "{is_org} {raw:?}"
            );
        }
        // An empty headline before an indented drawer: the drawer's line,
        // indentation included, is where metadata begins (R9 review), so the
        // drawer stays the block's own and a fold updates it.
        for (raw, at) in [
            ("\n  :PROPERTIES:\n  :id: x\n  :END:\n", 1),
            ("\n\t:PROPERTIES:\n\t:id: x\n\t:END:", 1),
            ("\r\n  :PROPERTIES:\r\n  :id: x\r\n  :END:\r\n", 2),
        ] {
            assert_eq!(parse(raw, true).headline_end, Some(at), "{raw:?}");
            assert_eq!(
                parse(raw, true).id.map(|p| p.value).as_deref(),
                Some("x"),
                "{raw:?}"
            );
            let folded = edit(raw, true, fold()).unwrap();
            assert_eq!(folded.matches(":PROPERTIES:").count(), 1, "{folded:?}");
            let rows = parse(&folded, true);
            assert_eq!(rows.id.map(|p| p.value).as_deref(), Some("x"), "{folded:?}");
            assert!(folded.contains(":collapsed: true"), "{folded:?}");
        }
        // A titleless block's rows begin at the title: a new row joins the end
        // of the run, a schedule goes before it, and both stay readable.
        let owner = || Edit::Property {
            key: "owner".into(),
            value: Some("me".into()),
        };
        let scheduled = || Edit::Planning {
            which: "Scheduled".into(),
            value: Some("<2026-11-12 Thu>".into()),
        };
        for (raw, is_org) in [
            ("id:: x\nfoo:: a", false),
            ("\nid:: x\nfoo:: a", false),
            (":PROPERTIES:\n:id: x\n:foo: a\n:END:", true),
        ] {
            let added = edit(raw, is_org, owner()).unwrap();
            let rows = parse(&added, is_org);
            let keys: Vec<_> = rows.properties.iter().map(|p| p.key.as_str()).collect();
            assert_eq!(keys, ["id", "foo", "owner"], "{added:?}");
            let planned = edit(raw, is_org, scheduled()).unwrap();
            let rows = parse(&planned, is_org);
            assert_eq!(rows.planning.len(), 1, "{planned:?}");
            assert_eq!(
                rows.id.map(|p| p.value).as_deref(),
                Some("x"),
                "{planned:?}"
            );
            assert_eq!(rows.properties.len(), 2, "{planned:?}");
        }
        // Most blocks carry no headline end of their own (their headline is
        // their first line), so their empty regions stay shared.
        for raw in [
            "",
            "x",
            "x\r",
            "title\nbody",
            "\nTODO",
            "TODO\r\n:PROPERTIES:\r\n:id: x\r\n:END:",
            // A title's trailing blanks and terminator are its own (R8).
            "TODO \nbody",
            "TODO \t\r\nbody",
            "TODO \r",
            // A headline that already ends with its newline keeps it there:
            // a blank second line is the body's (R8 review, region sharing).
            "x\n\nbody",
            "x\r\n\r\nbody",
            "TODO\n \nbody",
            // A node at the title position is the title line.
            "#+TITLE: t\n:PROPERTIES:\n:id: x\n:END:",
            "> q\n:PROPERTIES:\n:id: x\n:END:",
        ] {
            assert_eq!(parse(raw, true).headline_end, None, "{raw:?}");
        }
        for raw in [
            "#+TITLE: t\nid:: x",
            "> q\nid:: x",
            "TODO \nbody",
            "## TODO \r\nbody",
            "x\n\nbody",
            "x\r\n\r\nbody",
        ] {
            assert_eq!(parse(raw, false).headline_end, None, "{raw:?}");
        }
        assert_eq!(
            parse("\nid:: x\ncollapsed:: true", false).headline_end,
            Some(1)
        );
        assert_eq!(
            parse("id:: x\ncollapsed:: true", false).headline_end,
            Some(0)
        );
        // An empty title before its rows: metadata begins at the title.
        assert_eq!(
            parse("\n:PROPERTIES:\n:id: x\n:END:", true).headline_end,
            Some(1)
        );
        assert_eq!(
            parse(":PROPERTIES:\n:id: x\n:END:", true).headline_end,
            Some(0)
        );
        assert_eq!(parse("\n:LOGBOOK:\n:END:", true).headline_end, Some(1));
    }

    #[test]
    fn actual_parsed_tree_depth_is_checked_and_drained_iteratively() {
        let make_tree = |depth| {
            let mut tree = vec![Block::Paragraph {
                inline: Vec::new(),
                span: None,
            }];
            for _ in 0..depth {
                tree = vec![Block::Quote {
                    children: tree,
                    span: None,
                }];
            }
            tree
        };
        assert_eq!(bounded_blocks(make_tree(510)).len(), 1);
        assert_eq!(bounded_blocks(make_tree(1_020)).len(), 1);
        assert!(std::panic::catch_unwind(|| bounded_blocks(make_tree(20_000))).is_err());
    }

    #[test]
    fn quote_staircase_is_counted_as_lsdoc_nests_it() {
        for (line, depth) in [
            ("> > > x", 3),
            (">>>x", 3),
            ("  - > > x", 2),
            ("1. >\t> x", 2),
            ("text > not a quote", 0),
            ("-> arrow", 0),
            ("a:: >>> value", 0),
        ] {
            assert_eq!(line_quote_depth(line), depth, "{line}");
        }
        let at_cap = format!("{}x", "> ".repeat(SOURCE_QUOTE_DEPTH_MAX));
        assert!(parse_text_bounded(&at_cap, "md").is_some());
        let deep = format!("{}x", "> ".repeat(20_000));
        assert!(parse_text_bounded(&deep, "md").is_none());
        assert!(std::panic::catch_unwind(|| parse_projection(&deep, false)).is_err());
    }

    #[test]
    fn lsdoc_nesting_forms_are_actual_deep_trees() {
        let mut callouts = String::from("- root\n");
        for index in 0..600 {
            callouts.push_str(&format!("  #+BEGIN_a{index}\n"));
        }
        callouts.push_str("  text\n");
        for index in (0..600).rev() {
            callouts.push_str(&format!("  #+END_a{index}\n"));
        }
        let mut lists = String::from("- root\n");
        for depth in 0..600 {
            lists.push_str(&" ".repeat(depth + 2));
            lists.push_str("+ item\n");
        }
        for (source, format) in [
            (callouts, "md"),
            (lists, "md"),
            (format!("{}x\n", ">".repeat(1_200)), "org"),
        ] {
            let blocks = lsdoc::parse(&source, format);
            let deep = !parsed_tree_within_limit(&blocks, 512);
            drop_parsed_tree(blocks);
            assert!(deep, "{format} construct should build a deep lsdoc tree");
        }
    }

    #[test]
    fn ordinary_blocks_keep_the_single_input_fast_path() {
        for raw in [
            "ordinary **formatted** text",
            "tine.view:: grid",
            "inline `code` without a separator",
            "a:: value with `later code`",
        ] {
            let Preparation::Plain(input) = prepare(raw, false) else {
                panic!("ordinary block entered protection path: {raw}")
            };
            assert_eq!(input, format!("- {raw}"));
        }

        let Preparation::Plain(input) = prepare("DONE finished", true) else {
            panic!("Org must never enter the Markdown protection path")
        };
        assert_eq!(input, "* DONE finished");
    }

    #[test]
    fn property_lookalike_enters_the_exceptional_path() {
        let Preparation::Protected(prepared) = prepare("`a:: b` tail", false) else {
            panic!("line-leading inline code requires protection")
        };
        assert_eq!(prepared.protected.len(), 1);
        assert_eq!(prepared.input, "- `a;; b` tail");
    }
}
