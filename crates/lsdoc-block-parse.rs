//! Tine's one-block lsdoc boundary.
//!
//! mldoc/lsdoc classify a Markdown line that starts with an inline-code span
//! containing `::` as a property drawer before inline parsing. Tine deliberately
//! treats that narrow shape as code. We replace the separators with equal-width
//! bytes for the parse, then restore the complete code node from the source.
//! Equal width keeps every parser-owned source span valid; restoring by span (not
//! by a supposedly unique sentinel) makes the transform collision-proof.

use lsdoc::ast::{Block, Inline, ListItem, Projection, Span};

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

pub(crate) fn parse_block(raw: &str, is_org: bool) -> Vec<Block> {
    let prepared = match prepare(raw, is_org) {
        Preparation::Plain(input) => {
            return lsdoc::parse(&input, if is_org { "org" } else { "md" })
        }
        Preparation::Protected(prepared) => prepared,
    };
    let mut blocks = lsdoc::parse(&prepared.input, "md");
    let mut restored = vec![false; prepared.protected.len()];
    restore_blocks(&mut blocks, &prepared.protected, &mut restored);
    if restored.iter().all(|value| *value) {
        blocks
    } else {
        // Fail closed: an unanticipated parser shape may retain lsdoc's original
        // classification, but transformed bytes must never leak into the AST.
        lsdoc::parse(&format!("- {}", raw.trim_start()), "md")
    }
}

#[allow(dead_code)] // the wasm crate needs block parsing only; tine-core uses the full projection
pub(crate) fn parse_projection(raw: &str, is_org: bool) -> Projection {
    let prepared = match prepare(raw, is_org) {
        Preparation::Plain(input) => {
            return lsdoc::parse_format(&input, if is_org { "org" } else { "md" })
        }
        Preparation::Protected(prepared) => prepared,
    };
    let mut projection = lsdoc::parse_format(&prepared.input, "md");
    let mut restored = vec![false; prepared.protected.len()];
    restore_blocks(&mut projection.blocks, &prepared.protected, &mut restored);
    if restored.iter().all(|value| *value) {
        projection
    } else {
        lsdoc::parse_format(&format!("- {}", raw.trim_start()), "md")
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;

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
