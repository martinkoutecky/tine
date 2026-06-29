//! Org-mode (`.org`) document model: parse a Logseq org file into the SAME
//! [`Document`]/[`DocBlock`] tree the markdown path uses, and serialize it back
//! **byte-faithfully**.
//!
//! In Logseq org files, blocks are delimited by headlines (`*`, `**`, `***`)
//! rather than `-` bullets, and nesting equals headline level (OG
//! `get-block-pattern` → `"*"` for org). The text under a headline (planning
//! lines, `:PROPERTIES:` drawer, paragraphs, plain `-`/`+` lists, `#+BEGIN_…`
//! blocks) up to the next headline is that block's body, kept **verbatim** in
//! `raw`. We strip only the leading stars; serialize re-emits them from the
//! block's tree depth.
//!
//! ## Corruption safety
//! A `.org` page is only ever rewritten by Tine when it is **round-trip safe**:
//! `serialize_org(parse_org(content)) == content` byte-for-byte (see
//! [`org_editable`]). Files that fail that check are loaded **read-only** — Tine
//! never writes org it cannot reproduce exactly. Headline detection is
//! literal-block aware: a `*`-line inside a `#+BEGIN_…`/`#+END_…` block is
//! content, not a headline (matching org — and, notably, *more* correct than
//! orgize 0.9, which splits the block at such a line). The self-check is the
//! corruption firewall regardless of any parser's classification choices.

use crate::doc::{DocBlock, Document};

/// Heading level of a line if it is an org headline (`*`/`**`/… followed by a
/// space, tab, CR or end-of-line), else `None`. Headlines start at column 0.
/// `**bold**` (stars immediately followed by a non-space) is NOT a headline,
/// matching org; `** title` (stars then space) IS a level-2 headline.
fn headline_level(line: &str) -> Option<usize> {
    let stars = line.bytes().take_while(|&b| b == b'*').count();
    if stars == 0 {
        return None;
    }
    let rest = &line[stars..];
    if rest.is_empty()
        || rest.starts_with(' ')
        || rest.starts_with('\t')
        || rest.starts_with('\r')
    {
        Some(stars)
    } else {
        None
    }
}

/// Scan a file's lines for the real headlines, in document order, returning
/// `(line_index, level)` for each. Lines inside an org block
/// (`#+BEGIN_x` … `#+END_x`, any `x`, case-insensitive, leading whitespace
/// allowed) are content, so a `*`-line there is not mistaken for a headline.
fn scan_headlines(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut block_depth: usize = 0;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start_matches([' ', '\t']);
        let b = trimmed.as_bytes();
        if b.len() >= 2 && b[0] == b'#' && b[1] == b'+' {
            let kw = &trimmed[2..];
            if kw.len() >= 6 && kw[..6].eq_ignore_ascii_case("begin_") {
                block_depth += 1;
                continue;
            }
            if kw.len() >= 4 && kw[..4].eq_ignore_ascii_case("end_") {
                block_depth = block_depth.saturating_sub(1);
                continue;
            }
        }
        if block_depth == 0 {
            if let Some(level) = headline_level(line) {
                out.push((i, level));
            }
        }
    }
    out
}

/// Number of trailing `\n` bytes (the document-level trailing-newline run),
/// stripped on parse and reproduced on serialize so block bodies stay free of
/// trailing-blank artifacts.
fn trailing_newlines(s: &str) -> usize {
    s.bytes().rev().take_while(|&b| b == b'\n').count()
}

/// Parse org `content` into a [`Document`]: headlines become blocks (nesting =
/// headline level), the pre-headline region becomes `pre_block`, and each
/// block's body is kept verbatim in `raw` (leading stars stripped).
pub fn parse_org(content: &str) -> Document {
    let body = content.trim_end_matches('\n');
    if body.is_empty() {
        return Document::default();
    }
    let lines: Vec<&str> = body.split('\n').collect();
    let heads = scan_headlines(&lines);

    let first = heads.first().map(|h| h.0).unwrap_or(lines.len());
    let pre_block = if first == 0 {
        None
    } else {
        Some(lines[..first].join("\n"))
    };

    // One (level, block) per headline; body = lines up to the next headline.
    let mut flat: Vec<(usize, DocBlock)> = Vec::with_capacity(heads.len());
    for (n, &(start, level)) in heads.iter().enumerate() {
        let end = heads.get(n + 1).map(|h| h.0).unwrap_or(lines.len());
        let seg = &lines[start..end];
        // Drop the `level` stars and exactly one following space, so a block's raw
        // is the title text with no leading marker (matching the markdown path,
        // where `- ` is stripped). A second space, a tab, or no space is kept, so
        // serialize re-adds one space and still round-trips multi-space headlines.
        let after = &seg[0][level..];
        let first_content = after.strip_prefix(' ').unwrap_or(after);
        let raw = if seg.len() == 1 {
            first_content.to_string()
        } else {
            let mut s = String::with_capacity(first_content.len() + 16);
            s.push_str(first_content);
            for l in &seg[1..] {
                s.push('\n');
                s.push_str(l);
            }
            s
        };
        let mut b = DocBlock::new(raw);
        b.is_org = true; // org-format block → lsdoc parses inline refs in org mode
        flat.push((level, b));
    }

    Document { pre_block, roots: build_tree(flat) }
}

/// Assemble a flat list of `(level, block)` in document order into a forest,
/// nesting each block under the nearest preceding block of smaller level.
fn build_tree(flat: Vec<(usize, DocBlock)>) -> Vec<DocBlock> {
    let mut roots: Vec<DocBlock> = Vec::new();
    let mut stack: Vec<(usize, DocBlock)> = Vec::new();
    fn attach(stack: &mut Vec<(usize, DocBlock)>, roots: &mut Vec<DocBlock>, done: DocBlock) {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(done),
            None => roots.push(done),
        }
    }
    for (level, blk) in flat {
        while stack.last().is_some_and(|(l, _)| *l >= level) {
            let (_, done) = stack.pop().unwrap();
            attach(&mut stack, &mut roots, done);
        }
        stack.push((level, blk));
    }
    while let Some((_, done)) = stack.pop() {
        attach(&mut stack, &mut roots, done);
    }
    roots
}

/// Serialize a [`Document`] to org text with one trailing newline (the common
/// Logseq style). For exact byte-fidelity to a specific file, use
/// [`serialize_org_with`] with that file's trailing-newline count.
pub fn serialize_org(doc: &Document) -> String {
    serialize_org_with(doc, 1)
}

/// Serialize a [`Document`] to org text, ending with exactly `trailing` newline
/// bytes. The inverse of [`parse_org`] for round-trip-safe input: stars come
/// from tree depth (depth 0 → `*`), the pre-block and each block body verbatim.
pub fn serialize_org_with(doc: &Document, trailing: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    if let Some(pre) = &doc.pre_block {
        for line in pre.split('\n') {
            out.push(line.to_string());
        }
    }
    for block in &doc.roots {
        emit_org(block, 1, &mut out);
    }
    let mut s = out.join("\n");
    s.push_str(&"\n".repeat(trailing));
    s
}

fn emit_org(block: &DocBlock, level: usize, out: &mut Vec<String>) {
    let stars = "*".repeat(level);
    let mut lines = block.raw.split('\n');
    let first = lines.next().unwrap_or("");
    // Re-add the single space dropped on parse (an empty title is just the stars).
    if first.is_empty() {
        out.push(stars);
    } else {
        out.push(format!("{stars} {first}"));
    }
    for line in lines {
        out.push(line.to_string());
    }
    for child in &block.children {
        emit_org(child, level + 1, out);
    }
}

/// Serialize a [`Document`] to org text, reproducing `existing`'s
/// trailing-newline run (default one newline for a new file). The org analogue
/// of `doc::serialize_with(&doc, &SerializeOpts::detect(existing))`.
pub fn serialize_org_detect(doc: &Document, existing: Option<&str>) -> String {
    serialize_org_with(doc, existing.map(trailing_newlines).unwrap_or(1))
}

/// Whether `serialize_org(parse_org(content))` reproduces `content`
/// byte-for-byte (including its exact trailing-newline run).
pub fn org_round_trips(content: &str) -> bool {
    serialize_org_with(&parse_org(content), trailing_newlines(content)) == content
}

/// Whether Tine may safely **edit and write** this org file — i.e. it
/// round-trips byte-for-byte through [`parse_org`]/[`serialize_org_with`].
/// Otherwise the page is loaded read-only and never written.
pub fn org_editable(content: &str) -> bool {
    org_round_trips(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Logseq-faithful org samples that MUST round-trip byte-for-byte and be
    /// editable. Mirrors what OG / the Logseq mobile app actually write.
    fn corpus() -> Vec<(&'static str, &'static str)> {
        vec![
            ("empty", ""),
            ("single-newline", "\n"),
            ("og-template", "*\n"),
            ("one-block", "* block one\n"),
            ("one-block-no-trailing-nl", "* block one"),
            ("nested", "* parent\n** child\n"),
            ("deep-nest", "* a\n** b\n*** c\n** d\n* e\n"),
            (
                "task-sched-props",
                "* TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>\n:PROPERTIES:\n:id: 6679-abc\n:END:\n",
            ),
            ("priority-task", "* [#A] TODO Important\n** DOING sub task\n"),
            (
                "page-props-directives",
                "#+TITLE: My Page\n#+FILETAGS: :work:proj:\n\n* first\n* second\n",
            ),
            (
                "page-props-drawer",
                ":PROPERTIES:\n:title: My Page\n:END:\n* first\n",
            ),
            (
                "src-with-star",
                "* code\n#+BEGIN_SRC clojure\n* not a headline\n(defn f [] 1)\n#+END_SRC\n",
            ),
            (
                "lowercase-src",
                "* code\n#+begin_src python\n* still content\n#+end_src\n",
            ),
            ("plain-list-in-block", "* shopping\n- milk\n- eggs\n+ also fine\n"),
            ("blank-lines", "* a\n\n\n* b\n"),
            ("quote-block", "* note\n#+BEGIN_QUOTE\nto be or not\n#+END_QUOTE\n"),
            (
                "logbook",
                "* TODO task\n:LOGBOOK:\nCLOCK: [2026-06-25 Thu 09:00]--[2026-06-25 Thu 09:30] =>  00:30\n:END:\n",
            ),
            ("bold-star-content", "* heading\nthis is *bold* and /italic/ text\n"),
            ("trailing-blank-block", "* a\n* b\n\n"),
            ("two-trailing-newlines", "* a\n\n"),
            ("multi-space-after-stars", "*  extra space title\n"),
            ("no-headlines", "#+TITLE: Just directives\n#+FILETAGS: :x:\n"),
        ]
    }

    #[test]
    fn corpus_round_trips_byte_for_byte() {
        for (name, src) in corpus() {
            let got = serialize_org_with(&parse_org(src), trailing_newlines(src));
            assert_eq!(got, src, "round-trip mismatch for sample `{name}`");
            assert!(org_round_trips(src), "org_round_trips false for `{name}`");
        }
    }

    #[test]
    fn corpus_is_editable() {
        for (name, src) in corpus() {
            assert!(org_editable(src), "expected `{name}` to be editable");
        }
    }

    #[test]
    fn structure_simple() {
        let doc = parse_org("* parent\n** child\n*** grand\n* sibling\n");
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(doc.roots[0].raw, "parent");
        assert_eq!(doc.roots[0].children.len(), 1);
        assert_eq!(doc.roots[0].children[0].raw, "child");
        assert_eq!(doc.roots[0].children[0].children[0].raw, "grand");
        assert_eq!(doc.roots[1].raw, "sibling");
        assert!(doc.roots[1].children.is_empty());
    }

    #[test]
    fn body_kept_verbatim_with_block_text() {
        let doc = parse_org("* TODO task\nSCHEDULED: <2026-06-25 Thu>\nbody line\n");
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(doc.roots[0].raw, "TODO task\nSCHEDULED: <2026-06-25 Thu>\nbody line");
        // Marker detection (shared with markdown) sees the leading keyword.
        assert_eq!(doc.roots[0].marker(), Some("TODO"));
    }

    #[test]
    fn star_inside_src_is_not_a_headline() {
        let doc = parse_org("* code\n#+BEGIN_SRC clojure\n* not a headline\n#+END_SRC\n");
        assert_eq!(doc.roots.len(), 1, "the in-src `*` must not become a block");
        assert!(doc.roots[0].raw.contains("* not a headline"));
    }

    #[test]
    fn pre_block_holds_page_directives() {
        let doc = parse_org("#+TITLE: Page\n\n* first\n");
        assert_eq!(doc.pre_block.as_deref(), Some("#+TITLE: Page\n"));
        assert_eq!(doc.roots.len(), 1);
    }

    #[test]
    fn no_headlines_is_all_pre_block() {
        let src = "#+TITLE: Just directives\n#+FILETAGS: :x:\n";
        let doc = parse_org(src);
        assert!(doc.roots.is_empty());
        assert_eq!(serialize_org_with(&doc, trailing_newlines(src)), src);
    }

    #[test]
    fn non_contiguous_levels_are_read_only() {
        // `*` then `***` (skipped `**`): cannot be reproduced from tree depth,
        // so it must NOT be considered editable (loads read-only, never written).
        let src = "* a\n*** c\n";
        assert!(!org_round_trips(src), "skipped-level file should not round-trip");
        assert!(!org_editable(src));
    }

    #[test]
    fn crlf_round_trips_verbatim() {
        let src = "* a\r\n* b\r\n";
        assert!(org_round_trips(src));
    }
}
