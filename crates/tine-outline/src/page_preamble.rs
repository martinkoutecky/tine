//! Where a page header ends: its page properties and the prose after them.
//! The parser owns the answer (`block_regions::parse_document` marks the page
//! properties primary, OG's rule); these functions only read it.

use crate::block_regions::{parse_document, BlockRegions, Range};
use lsdoc::ast::Block;
use serde::Serialize;

/// One Markdown page-header property: the parser's accepted key and value, its
/// 0-based line index, and the byte range of its line without the newline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeaderEntry {
    pub key: String,
    pub value: String,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// A Markdown text's page properties (see [`page_header`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PageHeader {
    /// Byte end of the last header line, newline excluded; 0 with no entries.
    pub end: usize,
    pub entries: Vec<HeaderEntry>,
    /// Every line of the text is one of `entries` (at least one), none with a
    /// `#` key, which OG reads as no property (`valid-property-name?`): the
    /// text is a complete page header ([`page_header_only`]).
    pub whole: bool,
    /// The 0-based index of the first non-blank line that is no such entry:
    /// the line an incomplete header names.
    pub stray: Option<usize>,
}

/// The page header of a Markdown page preamble or first root block: its page
/// properties as the parser marks them (the first top-level property node;
/// OG `block.cljs extract-blocks`). A parser-refused input has no header.
/// O(bytes).
pub fn page_header(raw: &str) -> PageHeader {
    page_header_in(raw, &parse_document(raw, false))
}

/// The page header of `raw` when every line of the text is a page property
/// (at least one). Single answerer (I-12) for "is this first block a
/// page-properties block": a header line is a parser page property, so
/// `key::value` (no space), literal-fence content, a `#` key and a line after
/// a blank one (OG reads only the first property node) are not
/// (`block_regions::mapped_regions` owns the rule).
pub fn page_header_only(raw: &str) -> Option<PageHeader> {
    let header = page_header(raw);
    header.whole.then_some(header)
}

/// [`page_header`] over `raw`'s Markdown regions already parsed (one parse
/// for callers that also read other answers from it).
pub fn page_header_in(raw: &str, regions: &BlockRegions) -> PageHeader {
    let mut header = PageHeader::default();
    let (mut at, mut line) = (0, 0);
    for p in regions.page_properties() {
        line += raw[at..p.line.0].matches('\n').count();
        at = p.line.0;
        let end = p.line.1 - usize::from(raw.as_bytes()[p.line.1 - 1] == b'\n');
        header.entries.push(HeaderEntry {
            key: p.key.clone(),
            value: p.value.clone(),
            line,
            start: p.line.0,
            end,
        });
        header.end = end;
    }
    // OG reads a `#` key as no property (`valid-property-name?`).
    let mut good = header
        .entries
        .iter()
        .filter(|e| !e.key.starts_with('#'))
        .map(|e| e.line)
        .peekable();
    header.stray = raw.split('\n').enumerate().position(|(i, text)| {
        while good.next_if(|&g| g < i).is_some() {}
        !text.trim().is_empty() && good.peek() != Some(&i)
    });
    header.whole = header.stray.is_none()
        && !header.entries.is_empty()
        && header.end == raw.len()
        && header.entries.iter().enumerate().all(|(i, e)| e.line == i);
    header
}

/// Where the prose of a page preamble is: the text between the page-property
/// lines before it and those after it (Org: whole drawers), without the
/// newlines at either edge; the whole text when it has none. `None` when
/// nothing is left. Every other non-blank byte is a page-property line
/// (`block_regions::mapped_regions` keeps them out of the prose: Markdown has
/// none after it, Org none inside it).
/// OG reads the text before a page's first block as its first block
/// (graph-parser `block.cljs with-pre-block-if-exists`); Tine shows this part
/// as that block's row (`Page.tsx`) and promotes it to a real block when
/// edited (GH #85), so every reference question reads it as a block. The one
/// answer for both sides (the window asks it through wasm). O(bytes).
pub fn preamble_content_range(raw: &str, is_org: bool) -> Option<Range> {
    preamble_content_range_in(raw, is_org, &parse_document(raw, is_org))
}

/// [`preamble_content_range`] over already parsed regions, with no second parse.
pub fn preamble_content_range_in(raw: &str, is_org: bool, regions: &BlockRegions) -> Option<Range> {
    if regions.quarantined {
        return None;
    }
    // An Org drawer's `:END:` is header; a Markdown node's line after its
    // last page property is prose.
    let spans = if is_org {
        regions.org_page_property_spans(raw)
    } else {
        regions.page_properties().map(|p| p.line).collect()
    };
    let blank = |s: &str| s.chars().all(|c| c.is_whitespace() || c == '\u{feff}');
    let mut start = 0;
    for span in &spans {
        if !blank(&raw[start.min(span.0)..span.0]) {
            break;
        }
        start = start.max(span.1);
    }
    let mut end = raw.len();
    for span in spans.iter().rev() {
        if span.0 < start || !blank(&raw[span.1.min(end)..end]) {
            break;
        }
        end = end.min(span.0);
    }
    let (start, end) = (start.min(raw.len()), end.max(start).min(raw.len()));
    let rest = &raw[start..end];
    let leading = rest.len() - rest.trim_start_matches('\n').len();
    let content = rest.trim_start_matches('\n');
    let content = if end < raw.len() {
        content.trim_end_matches(['\r', '\n'])
    } else {
        content.trim_end_matches('\n')
    };
    (!content.is_empty()).then(|| Range(start + leading, start + leading + content.len()))
}

/// Where a page's opening text (the prose [`preamble_content_range`] finds)
/// has property groups: every top-level `key:: value` group or `#+key:
/// value` line of the text read as untitled body text
/// ([`parse_untitled_block`](crate::lsdoc_block_parse::parse_untitled_block),
/// the parse the window renders it with: no block header, so a task word or
/// priority is text), as whole source lines in source order
/// (D-20261119-PG: groups after the header are opening text, shown exactly as
/// written). The window shows these ranges as source and the rest as body
/// text. Empty for parser-refused text, which renders wholly as body text.
/// O(bytes).
pub fn opening_groups(text: &str, is_org: bool) -> Vec<Range> {
    use crate::lsdoc_block_parse::parse_untitled_block;
    #[cfg(not(target_arch = "wasm32"))]
    let Ok(blocks) = std::panic::catch_unwind(|| parse_untitled_block(text, is_org)) else {
        return Vec::new();
    };
    #[cfg(target_arch = "wasm32")]
    let blocks = parse_untitled_block(text, is_org);
    top_property_nodes(text, &blocks, false)
        .map(|(r, _)| r)
        .collect()
}

/// The property nodes among `blocks` (one level, as the parser split them):
/// each `key:: value` group (`true`) or `#+key: value` line (`false`), as
/// whole source lines of `raw` with their last line end. The one answer to
/// "which nodes are property nodes, and which lines are theirs": a page's
/// header is the first group before its first block ([`parse_document`](crate::block_regions::parse_document)),
/// and its opening text shows every group as written
/// ([`opening_groups`]). `document`: spans are `raw`'s own, not
/// a re-bulleted block's. O(nodes).
pub(crate) fn top_property_nodes<'a>(
    raw: &'a str,
    blocks: &'a [Block],
    document: bool,
) -> impl Iterator<Item = (Range, bool)> + 'a {
    let coords = crate::lsdoc_block_parse::Coords::of(raw, document);
    blocks.iter().filter_map(move |b| match b {
        Block::Properties { span, .. } | Block::Directive { span, .. } => {
            crate::block_regions::raw_range(raw, span, coords).map(|r| {
                (
                    crate::block_regions::whole_line(raw, r),
                    matches!(b, Block::Properties { .. }),
                )
            })
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::{opening_groups, page_header, preamble_content_range};
    use crate::block_regions::Range;
    use serde::Deserialize;

    fn text(raw: &str) -> Option<&str> {
        preamble_content_range(raw, false).map(|r| &raw[r.0..r.1])
    }

    /// D-20261119-PG: opening text's top-level property nodes, as the parser
    /// splits them, whole lines in source order; nothing nested, and no
    /// paragraph that only looks like a group.
    #[test]
    fn opening_groups_are_the_top_level_property_nodes_as_whole_lines() {
        let groups = |t: &str, org: bool| -> Vec<String> {
            opening_groups(t, org)
                .iter()
                .map(|r| t[r.0..r.1].to_owned())
                .collect()
        };
        assert_eq!(groups("prose\n\nk:: v\n\nmore", false), ["k:: v\n"]);
        assert_eq!(groups("prose\nk:: v", false), ["k:: v"]);
        assert_eq!(groups("a:: 1\n\nb:: 2", false), ["a:: 1\n", "b:: 2"]);
        assert_eq!(groups("  k:: v\n\tj:: w", false), ["  k:: v\n\tj:: w"]);
        assert_eq!(groups("prose\r\n\r\nk:: v\r\nmore", false), ["k:: v\r\n"]);
        assert_eq!(groups("prose\n#+k: v\nmore", false), ["#+k: v\n"]);
        assert_eq!(groups("prose\n#+k: v", true), ["#+k: v"]);
        assert_eq!(
            groups("#+BEGIN_QUOTE\n#+k: v\n#+END_QUOTE", false),
            Vec::<String>::new()
        );
        assert_eq!(groups("> q\n#+k: v", false), Vec::<String>::new());
        assert_eq!(groups("plain prose", false), Vec::<String>::new());
        // Untitled: a task word or priority is text, so its line is prose.
        assert_eq!(groups("TODO k:: v", false), Vec::<String>::new());
        assert_eq!(
            groups("TODO [#A] k:: v\nprose", false),
            Vec::<String>::new()
        );
        assert_eq!(groups("TODO [#A] Prose [[T]]", false), Vec::<String>::new());
    }

    /// Review 0e6bd6530 F1: one untitled parse answers which lines are groups,
    /// and each piece between them, parsed alone as the window renders it,
    /// has the same top-level nodes it has in that parse (no directive hidden
    /// in a quote one parse never makes). A task word, a priority or both
    /// before `key::` lines, fences, quotes, notes, `#+` directives, query
    /// containers and drawers; LF, CRLF, tab and blank-led; both formats.
    #[test]
    fn opening_groups_and_pieces_share_one_untitled_parse() {
        use crate::lsdoc_block_parse::parse_untitled_block;
        use lsdoc::ast::Block;
        fn kinds(blocks: &[Block]) -> Vec<String> {
            blocks
                .iter()
                .filter(|b| match b {
                    Block::Properties { .. } | Block::Directive { .. } => false,
                    Block::Bullet { inline, .. } | Block::Paragraph { inline, .. } => {
                        !inline.iter().all(|i| format!("{i:?}").contains("Break")
                            || matches!(i, lsdoc::ast::Inline::Plain { text, .. } if text.trim().is_empty()))
                    }
                    _ => true,
                })
                .map(|b| format!("{b:?}").split([' ', '{', '(']).next().unwrap().to_owned())
                .collect()
        }
        for (shape, md, org) in OPENING_SHAPES {
            for prefix in ["TODO ", "[#A] ", "TODO [#A] ", "TODO\t"] {
                for (variant, is_org) in ["lf", "crlf", "blank"]
                    .iter()
                    .flat_map(|v| [(*v, false), (*v, true)])
                {
                    let base = shape.replace("{P}", prefix);
                    let text = match variant {
                        "crlf" => base.replace('\n', "\r\n"),
                        "blank" => format!("\n{base}"),
                        _ => base,
                    };
                    let want = if is_org { org } else { md };
                    let ranges = opening_groups(&text, is_org);
                    let got: Vec<String> = ranges
                        .iter()
                        .map(|r| text[r.0..r.1].trim_end().replace("\r\n", "\n"))
                        .collect();
                    assert_eq!(got, *want, "{text:?} org={is_org}");
                    let mut pieces = Vec::new();
                    let mut at = 0;
                    for r in ranges.iter().chain([&Range(text.len(), text.len())]) {
                        let piece = text[at..r.0].trim_matches(['\n', '\r']);
                        if !piece.trim().is_empty() {
                            pieces.extend(kinds(&parse_untitled_block(piece, is_org)));
                        }
                        at = r.1;
                    }
                    assert_eq!(
                        pieces,
                        kinds(&parse_untitled_block(&text, is_org)),
                        "{text:?} org={is_org}"
                    );
                }
            }
        }
    }

    /// The review's opening shapes, `{P}` the task word and/or priority, with
    /// the groups the parser finds (Markdown, Org), line ends as `\n`.
    const OPENING_SHAPES: &[(&str, &[&str], &[&str])] = &[
        ("{P}x\nk:: v", &["k:: v"], &[]),
        ("{P}k:: **v**", &[], &[]),
        ("{P}#+k: v", &[], &[]),
        ("{P}> quote\n#+k: v", &["#+k: v"], &["#+k: v"]),
        ("{P}> quote\nk:: v", &["k:: v"], &[]),
        ("{P}\n```\nc\n```\n#+k: v", &["#+k: v"], &["#+k: v"]),
        ("{P}\n~~~\nc\n~~~\nk:: v", &["k:: v"], &[]),
        (
            "{P}\n#+BEGIN_SRC\nc\n#+END_SRC\n#+k: v",
            &["#+k: v"],
            &["#+k: v"],
        ),
        ("{P}\n#+BEGIN_NOTE\nn\n#+END_NOTE\nk:: v", &["k:: v"], &[]),
        (
            "{P}\n#+BEGIN_QUOTE\nq\n#+END_QUOTE\n#+k: v",
            &["#+k: v"],
            &["#+k: v"],
        ),
        (
            "{P}\n#+BEGIN_QUERY\n(task TODO)\n#+END_QUERY\n#+k: v",
            &["#+k: v"],
            &["#+k: v"],
        ),
        ("{P}\n{{query (task TODO)}}\nk:: v", &["k:: v"], &[]),
        ("{P}\n:LOGBOOK:\nCLOCK: x\n:END:\nk:: v", &["k:: v"], &[]),
        ("{P}\n:custom:\nv\n:END:\n#+k: v", &["#+k: v"], &["#+k: v"]),
        (
            "{P}\n:PROPERTIES:\n:a: 1\n:END:\nk:: v",
            &[":PROPERTIES:\n:a: 1\n:END:\nk:: v"],
            &[":PROPERTIES:\n:a: 1\n:END:"],
        ),
    ];

    #[test]
    fn the_prose_after_the_page_header_without_edge_newlines() {
        assert_eq!(text("[[Target]] in prose"), Some("[[Target]] in prose"));
        assert_eq!(
            text("tags:: x\n\n\nsee [[Target]]\n\n"),
            Some("see [[Target]]")
        );
        assert_eq!(text("tags:: x"), None);
        assert_eq!(text(""), None);
    }

    /// One row of `og_page_header.json`: an input and OG's answer for it,
    /// taken from mldoc 1.5.9's AST (the lsdoc oracle harness) under OG's
    /// `extract-blocks` rule, never from Tine. Generator (private evidence):
    /// `tine-agents/evidence/cm-header-boundary/oracle/fixture.sh`.
    #[derive(Deserialize)]
    struct Row {
        format: String,
        input: String,
        /// OG's page property keys, lowercased, in file order.
        properties: Vec<String>,
        /// Where OG's page-property node ends (UTF-8 bytes); `null` without one.
        header_end: Option<usize>,
        /// A named Tine difference from OG on this row (asserted to persist).
        #[serde(default)]
        known_difference: Option<String>,
    }

    /// Tine's page properties are OG's, and no page property lies inside the
    /// prose (the boundary has one owner).
    #[test]
    fn page_properties_and_the_prose_boundary_are_ogs() {
        let rows: Vec<Row> =
            serde_json::from_str(include_str!("page_preamble/og_page_header.json")).unwrap();
        assert!(rows.len() >= 40, "{}", rows.len());
        let mut wrong = Vec::new();
        for row in rows {
            let org = row.format == "org";
            let regions = crate::block_regions::parse_document(&row.input, org);
            let keys: Vec<String> = regions
                .page_properties()
                .map(|p| p.key.to_lowercase())
                .collect();
            // The prose of the pre-block (callers pass the text before the
            // first block): Org page properties can follow it.
            let pre = match org {
                true => match row.input.find("\n* ") {
                    _ if row.input.starts_with("* ") => "",
                    Some(at) => &row.input[..at],
                    None => &row.input,
                },
                false => &row.input,
            };
            let prose = preamble_content_range(pre, org);
            if let Some(prose) = prose {
                assert!(
                    // Org page properties can follow the prose, never sit in it.
                    regions
                        .page_properties()
                        .all(|p| p.line.1 <= prose.0 || (org && p.line.0 >= prose.1)),
                    "a page property inside the prose: {:?}",
                    row.input
                );
                // A node with no valid key (`#tag:: x`) leaves its text to the
                // prose. Org prose can precede the head drawer; the assertion
                // above keeps it out of the drawer.
                let same = row.known_difference.is_none() && !org;
                if let Some(end) = row.header_end.filter(|_| same && !keys.is_empty()) {
                    assert!(prose.0 >= end, "{:?}", row.input);
                }
            }
            if !org {
                let header = page_header(&row.input);
                let entries: Vec<String> = header
                    .entries
                    .iter()
                    .map(|e| e.key.to_lowercase())
                    .collect();
                assert_eq!(entries, keys, "{:?}", row.input);
            }
            if (keys == row.properties) != row.known_difference.is_none() {
                wrong.push(format!(
                    "{:?}: Tine {keys:?}, OG {:?}, known {:?}",
                    row.input, row.properties, row.known_difference
                ));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// Where `raw` breaks the header/prose partition: a non-blank byte in
    /// both a page-property line (Org: its whole drawer) and the prose, or in
    /// neither (shown nowhere, indexed nowhere, skipped by rename).
    fn partition_error(raw: &str, org: bool) -> Option<String> {
        let regions = crate::block_regions::parse_document(raw, org);
        if regions.quarantined {
            return None;
        }
        let prose = preamble_content_range(raw, org);
        let lines: Vec<_> = if org {
            regions.org_page_property_spans(raw)
        } else {
            regions.page_properties().map(|p| p.line).collect()
        };
        let (i, c) = raw.char_indices().find(|&(i, c)| {
            let property = lines.iter().any(|l| l.0 <= i && i < l.1);
            let in_prose = prose.is_some_and(|r| r.0 <= i && i < r.1);
            !c.is_whitespace() && c != '\u{feff}' && property == in_prose
        })?;
        Some(format!(
            "{raw:?}: byte {i} ({c:?}) lines {lines:?} prose {prose:?}"
        ))
    }

    /// Every non-blank byte of a Markdown pre-block is in exactly one of the
    /// page header and the prose row (REG-CM-HEADER-BOUNDARY, review F1):
    /// over the OG fixture and every sequence of up to three generated lines.
    #[test]
    fn every_markdown_preamble_byte_is_header_or_prose_once() {
        let rows: Vec<Row> =
            serde_json::from_str(include_str!("page_preamble/og_page_header.json")).unwrap();
        let mut inputs: Vec<String> = (rows.into_iter())
            .filter(|r| r.format == "md")
            .map(|r| r.input)
            .collect();
        let pieces = [
            "",
            " ",
            "alias:: y",
            " k:: v",
            "#tag:: x",
            "Intro [[X]]",
            "#+b: [[X]]",
            "```",
            ":PROPERTIES:",
            ":END:",
            "k::v",
            "title:: T",
            "\u{feff}a:: 1",
            "> q:: r",
        ];
        for a in pieces {
            for b in pieces {
                for c in pieces {
                    for eol in ["\n", "\r\n"] {
                        inputs.push([a, b, c].join(eol));
                        inputs.push([a, b, c, ""].join(eol));
                    }
                }
            }
        }
        let wrong: Vec<_> = inputs
            .iter()
            .filter_map(|raw| partition_error(raw, false))
            .collect();
        assert!(
            wrong.is_empty(),
            "{} of {}: {:#?}",
            wrong.len(),
            inputs.len(),
            &wrong[..wrong.len().min(20)]
        );
    }

    /// The same partition for an Org pre-block (REG-CM-ORG-TEXT-BETWEEN-
    /// DIRECTIVES): text between directives or around the head drawer is
    /// prose, never neither. Over the Org fixture rows' pre-blocks and every
    /// sequence of up to four generated lines (a pre-block has no headline).
    #[test]
    fn every_org_preamble_byte_is_header_or_prose_once() {
        let rows: Vec<Row> =
            serde_json::from_str(include_str!("page_preamble/og_page_header.json")).unwrap();
        let mut inputs: Vec<String> = (rows.into_iter())
            .filter(|r| r.format == "org")
            .map(|r| r.input.split("\n* ").next().unwrap_or("").to_string())
            .filter(|pre| !pre.starts_with("* ") && !pre.contains('\r'))
            .collect();
        let pieces = [
            "",
            " ",
            "#+title: T",
            "[[X]] text",
            ":PROPERTIES:",
            ":a: x",
            ":END:",
            "#+alias: y",
            "#+BEGIN_QUOTE",
            "#+END_QUOTE",
            "\u{feff}#+b: z",
        ];
        for a in pieces {
            for b in pieces {
                for c in pieces {
                    for d in pieces {
                        inputs.push([a, b, c, d].join("\n"));
                    }
                }
            }
        }
        let wrong: Vec<_> = inputs
            .iter()
            .filter_map(|raw| partition_error(raw, true))
            .collect();
        assert!(
            wrong.is_empty(),
            "{} of {}: {:#?}",
            wrong.len(),
            inputs.len(),
            &wrong[..wrong.len().min(20)]
        );
    }

    /// Review 28d081755 F1 (PC-3): mapping the groups measures the leading
    /// trim once, not twice per group, so whitespace before many groups costs
    /// one scan. Counted, and timed at 2,000 groups after 16 KB of lead.
    #[test]
    fn opening_groups_cost_is_linear() {
        let text = format!(
            "{}{}",
            "  \t\n".repeat(4_000),
            "k:: v\nprose\n\n".repeat(2_000)
        );
        crate::lsdoc_block_parse::LEAD_SCANS.with(|n| n.set(0));
        let started = std::time::Instant::now();
        let groups = opening_groups(&text, false);
        let elapsed = started.elapsed();
        assert_eq!(groups.len(), 2_000);
        let scans = crate::lsdoc_block_parse::LEAD_SCANS.with(|n| n.replace(0));
        assert!(scans <= 1, "{scans} leading-trim scans for one call");
        // Linear is ~12 ms in a debug build; one scan per group was ~1.1 s.
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "{elapsed:?}"
        );
    }

    /// Review e852e1b58: opening text is read with an empty title, so a task
    /// word or priority on its first line is text, never a block header.
    #[test]
    fn untitled_parse_lifts_no_header() {
        use lsdoc::ast::Block;
        for is_org in [false, true] {
            for text in [
                "TODO x",
                "[#A] y",
                "TODO [#A] z",
                "TODO \n:LOGBOOK:\nCLOCK: x\n:END:\nNext",
                "DONE [#B]\n:custom:\nv\n:END:\nNext",
            ] {
                let blocks = crate::lsdoc_block_parse::parse_untitled_block(text, is_org);
                assert!(
                    blocks.iter().all(|b| !matches!(
                        b,
                        Block::Bullet {
                            marker: Some(_),
                            ..
                        } | Block::Bullet {
                            priority: Some(_),
                            ..
                        }
                    )),
                    "{text:?} org={is_org}: {blocks:?}"
                );
            }
        }
    }
}
