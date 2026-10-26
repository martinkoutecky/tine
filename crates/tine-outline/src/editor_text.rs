//! The editor's view of one block: its text without the hidden property lines,
//! the hidden lines themselves, and the offset maps between that text and raw.
//!
//! Transcribes the window's `classifyLines` / `splitProps` / `joinProps` /
//! `rawOffsetToVisibleOffset` (`src/editor/properties.ts:161-280` at
//! `b217e8727`), so the kernel's verbs and the editor share one projection
//! (outline-kernel DESIGN §5.1, twin 13). Offsets here are UTF-8 bytes; the
//! window converts its UTF-16 caret with [`utf16_to_byte`] / [`byte_to_utf16`].
//!
//! Which lines are metadata is the parser's answer (`block_regions`), never a
//! line pattern: a `key:: value` line inside a fence stays visible text. Cost is
//! O(block bytes) given the block's regions.

use crate::block_regions::{self, BlockRegions};
use serde::{Deserialize, Serialize};

/// The built-in properties every block editor hides (OG hides the same three):
/// they persist in the file but never appear in the edit buffer.
pub const BUILTIN_HIDDEN: [&str; 3] = ["id", "collapsed", "logseq.order-list-type"];

/// Which accepted primary properties the editor hides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HidePolicy {
    /// An ordinary block editor: [`BUILTIN_HIDDEN`].
    Builtin,
    /// A sheet cell: the built-ins plus every `tine.*` key.
    SheetCell,
    /// An annotation (PDF highlight) block edits only its text.
    All,
    /// Exactly these keys, lower-cased; with `norm`, compared after
    /// [`property_key_norm`] instead (the list-type transform, which the window
    /// spells through `propertyKeyNorm`).
    Keys {
        keys: Vec<String>,
        #[serde(default)]
        norm: bool,
    },
}

impl HidePolicy {
    /// Whether this policy hides `key` (as the parser spelled it).
    pub fn hides(&self, key: &str) -> bool {
        let lower = key.to_lowercase();
        match self {
            HidePolicy::Builtin => BUILTIN_HIDDEN.contains(&lower.as_str()),
            HidePolicy::SheetCell => {
                BUILTIN_HIDDEN.contains(&lower.as_str()) || lower.starts_with("tine.")
            }
            HidePolicy::All => true,
            HidePolicy::Keys { keys, norm: true } => {
                let norm = property_key_norm(key);
                keys.iter().any(|k| property_key_norm(k) == norm)
            }
            HidePolicy::Keys { keys, norm: false } => {
                keys.iter().any(|k| k.to_lowercase() == lower)
            }
        }
    }
}

/// The window's `propertyKeyNorm` (`src/propertyKey.ts:7`): trimmed, ASCII
/// upper case lowered, spaces and underscores as hyphens.
pub fn property_key_norm(key: &str) -> String {
    key.trim()
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            ' ' | '_' => '-',
            c => c,
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Line {
    Visible,
    /// A hidden property line: kept in `hidden`.
    Hidden,
    /// An Org drawer wrapper whose every entry is hidden: rebuilt by `join`.
    Dropped,
}

/// One block split for the editor. `visible + hidden` rejoin through [`join`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EditorText {
    pub visible: String,
    pub hidden: String,
}

/// The raw lines (byte ranges without their `\n`) and their classes.
struct Classified {
    lines: Vec<(usize, usize)>,
    class: Vec<Line>,
}

fn classify(raw: &str, is_org: bool, regions: &BlockRegions, hide: &HidePolicy) -> Classified {
    let mut lines = Vec::new();
    let mut start = 0;
    for (at, _) in raw.match_indices('\n') {
        lines.push((start, at));
        start = at + 1;
    }
    lines.push((start, raw.len()));
    let mut class = vec![Line::Visible; lines.len()];
    if regions.quarantined {
        return Classified { lines, class };
    }
    let line_at = |byte: usize| lines.partition_point(|&(s, _)| s <= byte).saturating_sub(1);
    let own = || regions.properties.iter().filter(|p| p.primary);
    for p in own() {
        if hide.hides(&p.key) {
            class[line_at(p.line.0)] = Line::Hidden;
        }
    }
    if is_org {
        for (index, range) in regions.property_regions.iter().enumerate() {
            let mut entries = own().filter(|p| p.region == index).peekable();
            if entries.peek().is_none() || !entries.all(|p| hide.hides(&p.key)) {
                continue;
            }
            class[line_at(range.0)] = Line::Dropped;
            class[line_at(range.0.max(range.1.saturating_sub(1)))] = Line::Dropped;
        }
    }
    Classified { lines, class }
}

/// Split `raw` into the editor's text and its hidden property lines, given the
/// regions parsed from exactly this `raw`.
pub fn split_with(
    raw: &str,
    is_org: bool,
    regions: &BlockRegions,
    hide: &HidePolicy,
) -> EditorText {
    let c = classify(raw, is_org, regions, hide);
    let mut visible: Vec<&str> = Vec::new();
    let mut hidden: Vec<&str> = Vec::new();
    for (&(s, e), class) in c.lines.iter().zip(&c.class) {
        match class {
            Line::Visible => visible.push(&raw[s..e]),
            Line::Hidden => hidden.push(&raw[s..e]),
            Line::Dropped => {}
        }
    }
    EditorText {
        visible: visible.join("\n"),
        hidden: hidden.join("\n"),
    }
}

/// [`split_with`], parsing `raw` once.
pub fn split(raw: &str, is_org: bool, hide: &HidePolicy) -> EditorText {
    split_with(raw, is_org, &block_regions::parse(raw, is_org), hide)
}

/// The editor offset of raw byte `raw_offset`. An offset inside a hidden line
/// maps to the point where that line would have appeared.
pub fn raw_to_visible_with(
    raw: &str,
    is_org: bool,
    regions: &BlockRegions,
    hide: &HidePolicy,
    raw_offset: usize,
) -> usize {
    let c = classify(raw, is_org, regions, hide);
    let target = floor_boundary(raw, raw_offset.min(raw.len()));
    let mut visible_len = 0;
    let mut any_visible = false;
    for (&(s, e), class) in c.lines.iter().zip(&c.class) {
        let here = s <= target && target <= e;
        if *class == Line::Visible {
            let line_start = visible_len + usize::from(any_visible);
            if here {
                return line_start + (target - s);
            }
            any_visible = true;
            visible_len = line_start + (e - s);
        } else if here {
            return visible_len;
        }
    }
    visible_len
}

/// The raw byte offset of editor offset `visible_offset`: always inside a
/// visible line, so [`raw_to_visible_with`] maps it back unchanged.
pub fn visible_to_raw_with(
    raw: &str,
    is_org: bool,
    regions: &BlockRegions,
    hide: &HidePolicy,
    visible_offset: usize,
) -> usize {
    let c = classify(raw, is_org, regions, hide);
    let mut visible_len = 0;
    let mut any_visible = false;
    let mut last_end = None;
    for (&(s, e), class) in c.lines.iter().zip(&c.class) {
        if *class != Line::Visible {
            continue;
        }
        let line_start = visible_len + usize::from(any_visible);
        let line_end = line_start + (e - s);
        if visible_offset <= line_end {
            return floor_boundary(raw, s + visible_offset.saturating_sub(line_start));
        }
        any_visible = true;
        visible_len = line_end;
        last_end = Some(e);
    }
    last_end.unwrap_or(0)
}

/// Reattach hidden property lines to editor text: the inverse of [`split`].
/// Markdown puts them at the end of the block's own property group when the
/// text has one, else before a final quote, else below the body; Org folds them
/// into the property drawer at its canonical place. A quarantined Org parse
/// refuses.
pub fn join(visible: &str, hidden: &str, is_org: bool) -> Result<String, String> {
    if hidden.is_empty() || !is_org {
        return join_with(visible, hidden, is_org, &BlockRegions::default());
    }
    join_with(visible, hidden, true, &block_regions::parse(visible, true))
}

/// [`join`], given the regions parsed from exactly `visible` (read for Org only).
pub fn join_with(
    visible: &str,
    hidden: &str,
    is_org: bool,
    visible_regions: &BlockRegions,
) -> Result<String, String> {
    if hidden.is_empty() {
        return Ok(visible.to_string());
    }
    if !is_org {
        if let Some(joined) = join_into_visible_group(visible, hidden) {
            return Ok(joined);
        }
        // Metadata appended after a final quote is its lazy continuation, so it goes
        // before the quote (GH #684, #721 F3).
        if let Some(start) = crate::swallowed_rows::quote_tail(visible).and_then(|t| t.start) {
            let hidden = hidden.strip_suffix('\r').unwrap_or(hidden);
            return Ok(format!(
                "{}{hidden}\n{}",
                &visible[..start],
                &visible[start..]
            ));
        }
        return Ok(if visible.is_empty() {
            hidden.to_string()
        } else {
            format!("{visible}\n{hidden}")
        });
    }
    visible_regions.reattach_org_properties(visible, hidden)
}

/// Markdown text that keeps a property group of its own takes the hidden lines
/// at the end of that group. Appending them below the body started a second
/// group, and OG keeps only a block's first one (`extract-blocks`,
/// graph_parser/block.cljs): an edited `Title / id:: / owner:: / body` block lost
/// its `id::` in OG, breaking its references there. OG itself writes built-ins
/// at the group's head (`with-built-in-properties`, frontend/util/property.cljs);
/// the end keeps the order Tine has always written, and both are one group.
/// `None` when the text has no group (or its parse is quarantined).
fn join_into_visible_group(visible: &str, hidden: &str) -> Option<String> {
    let regions = block_regions::parse(visible, false);
    if regions.quarantined {
        return None;
    }
    let region = regions.properties.iter().find(|p| p.primary)?.region;
    let last = regions
        .properties
        .iter()
        .filter(|p| p.primary && p.region == region)
        .map(|p| p.line.0)
        .max()?;
    let Some(at) = visible[last..].find('\n').map(|n| last + n) else {
        return Some(format!("{visible}\n{hidden}"));
    };
    // A CRLF block keeps CRLF on the moved-in last hidden line.
    let cr = if visible[..at].ends_with('\r') && !hidden.ends_with('\r') {
        "\r"
    } else {
        ""
    };
    Some(format!(
        "{}\n{hidden}{cr}{}",
        &visible[..at],
        &visible[at..]
    ))
}

fn floor_boundary(s: &str, mut at: usize) -> usize {
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The byte offset of UTF-16 offset `units` in `s` (clamped; a unit inside a
/// surrogate pair rounds down to the character's start).
pub fn utf16_to_byte(s: &str, units: usize) -> usize {
    let mut seen = 0;
    for (at, c) in s.char_indices() {
        if seen + c.len_utf16() > units {
            return at;
        }
        seen += c.len_utf16();
    }
    s.len()
}

/// The UTF-16 offset of byte `byte` in `s` (clamped, rounded down to a boundary).
pub fn byte_to_utf16(s: &str, byte: usize) -> usize {
    let at = floor_boundary(s, byte.min(s.len()));
    s[..at].encode_utf16().count()
}

/// JavaScript's `String.prototype.trim() === ""`: ECMAScript WhiteSpace and
/// LineTerminator, which differ from Rust's `char::is_whitespace` at U+0085
/// (not JS whitespace) and U+FEFF (JS whitespace).
pub fn js_blank(s: &str) -> bool {
    s.chars().all(|c| {
        matches!(
            c,
            '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
                ..='\u{200A}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202F}'
                    | '\u{205F}'
                    | '\u{3000}'
                    | '\u{FEFF}'
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(raw: &str, is_org: bool) {
        let t = split(raw, is_org, &HidePolicy::Builtin);
        assert_eq!(join(&t.visible, &t.hidden, is_org).unwrap(), raw, "{raw:?}");
    }

    /// GH #721 follow-up F3: hidden rows rejoined to a block that ends in a quote go
    /// before the quote (where the parser accepts them), never after it, where the
    /// quote swallows them as lazy lines. Same placement as the window's `joinProps`.
    #[test]
    fn hidden_rows_rejoin_before_a_final_quote() {
        for (raw, text, expected) in [
            ("collapsed:: true\n> q", "> q2", "collapsed:: true\n> q2"),
            (
                "Title\ncollapsed:: true\n> q",
                "T2\n> q",
                "T2\ncollapsed:: true\n> q",
            ),
            (
                "collapsed:: true\n> [!TIP]\n> helpful",
                "> [!TIP]\n> more",
                "collapsed:: true\n> [!TIP]\n> more",
            ),
            ("collapsed:: true\r\n> q", "> q2", "collapsed:: true\n> q2"),
            // Not a trailing quote: below the body, as before.
            ("collapsed:: true\nbody", "body2", "body2\ncollapsed:: true"),
            (
                "collapsed:: true\na > b",
                "a > c",
                "a > c\ncollapsed:: true",
            ),
        ] {
            // og names this `set_text`; release/0.8 has no such helper.
            let out = join(text, &split(raw, false, &HidePolicy::Builtin).hidden, false).unwrap();
            assert_eq!(out.replace("\r\n", "\n"), expected, "{raw:?}");
            assert!(tine_outline_facet_collapsed(&out), "{out:?}");
        }
    }
    fn tine_outline_facet_collapsed(raw: &str) -> bool {
        crate::lsdoc_block_parse::parse_block(raw, false)
            .iter()
            .any(|b| {
                matches!(b, lsdoc::ast::Block::Properties { props, .. }
                if props.iter().any(|p| p.0 == "collapsed" && p.1 == "true"))
            })
    }

    /// A block with its own visible property group keeps one group: the hidden
    /// lines join its end, never below the body
    /// as a second group that OG ignores (REG-HIDDEN-PROPS-STAY-IN-THE-GROUP).
    #[test]
    fn hidden_rows_rejoin_the_visible_property_group() {
        let id = "id:: 650e8400-e29b-41d4-a716-446655440000";
        for (raw, edit, expected) in [
            (
                format!("Title\n{id}\nowner:: Ada\nbody line"),
                ("Title", "Title X"),
                format!("Title X\nowner:: Ada\n{id}\nbody line"),
            ),
            (
                format!("Title\ncollapsed:: true\n{id}\nowner:: Ada\nbody"),
                ("body", "body 2"),
                format!("Title\nowner:: Ada\ncollapsed:: true\n{id}\nbody 2"),
            ),
            // A page-properties block: the group is the whole text.
            (
                format!("{id}\ntags:: a\nalias:: b"),
                ("alias:: b", "alias:: c"),
                format!("tags:: a\nalias:: c\n{id}"),
            ),
            // A group before a final quote: hidden lines stay in the group.
            (
                format!("Title\n{id}\nowner:: Ada\nbody\n> quote"),
                ("body", "body 2"),
                format!("Title\nowner:: Ada\n{id}\nbody 2\n> quote"),
            ),
            // CRLF: the moved-in line keeps the block's line ending.
            (
                format!("Title\r\nowner:: Ada\r\nbody\r\n{id}"),
                ("body", "body 2"),
                format!("Title\r\nowner:: Ada\r\n{id}\r\nbody 2\r"),
            ),
            // A block an earlier edit split into two groups is repaired to one.
            (
                format!("Title\nowner:: Ada\nbody\n{id}"),
                ("body", "body 2"),
                format!("Title\nowner:: Ada\n{id}\nbody 2"),
            ),
        ] {
            let t = split(&raw, false, &HidePolicy::Builtin);
            let visible = t.visible.replacen(edit.0, edit.1, 1);
            let out = join(&visible, &t.hidden, false).unwrap();
            assert_eq!(out, expected, "{raw:?}");
            assert_eq!(
                join_with(&visible, &t.hidden, false, &BlockRegions::default()).unwrap(),
                out
            );
            let groups = crate::lsdoc_block_parse::parse_block(&out, false)
                .iter()
                .filter(|b| matches!(b, lsdoc::ast::Block::Properties { .. }))
                .count();
            assert_eq!(groups, 1, "{out:?}");
        }
    }

    #[test]
    fn markdown_hides_the_builtins_and_keeps_fenced_lookalikes() {
        let raw = "text\nid:: 650e8400-e29b-41d4-a716-446655440000\ncollapsed:: true";
        let t = split(raw, false, &HidePolicy::Builtin);
        assert_eq!(t.visible, "text");
        assert_eq!(
            t.hidden,
            "id:: 650e8400-e29b-41d4-a716-446655440000\ncollapsed:: true"
        );
        roundtrip(raw, false);
        let fenced = "```\nid:: x\n```";
        assert_eq!(split(fenced, false, &HidePolicy::Builtin).visible, fenced);
        let user = "text\nowner:: me";
        assert_eq!(split(user, false, &HidePolicy::Builtin).visible, user);
    }

    #[test]
    fn org_drops_an_all_hidden_drawer_and_rebuilds_it() {
        let raw = "title\n:PROPERTIES:\n:id: 650e8400-e29b-41d4-a716-446655440000\n:END:\nbody";
        let t = split(raw, true, &HidePolicy::Builtin);
        assert_eq!(t.visible, "title\nbody");
        assert_eq!(t.hidden, ":id: 650e8400-e29b-41d4-a716-446655440000");
        roundtrip(raw, true);
    }

    #[test]
    fn offsets_round_trip_through_visible_lines() {
        let raw = "ab\nid:: 650e8400-e29b-41d4-a716-446655440000\ncd";
        let regions = block_regions::parse(raw, false);
        let p = HidePolicy::Builtin;
        assert_eq!(split_with(raw, false, &regions, &p).visible, "ab\ncd");
        for v in 0..=5 {
            let r = visible_to_raw_with(raw, false, &regions, &p, v);
            assert_eq!(raw_to_visible_with(raw, false, &regions, &p, r), v);
        }
        // Inside the hidden line: where the line would have been.
        assert_eq!(raw_to_visible_with(raw, false, &regions, &p, 6), 2);
    }

    #[test]
    fn utf16_offsets_convert_at_character_boundaries() {
        let s = "a😀b";
        assert_eq!(utf16_to_byte(s, 1), 1);
        assert_eq!(utf16_to_byte(s, 3), 5);
        assert_eq!(byte_to_utf16(s, 5), 3);
        assert!(js_blank(" \u{FEFF}\n"));
        assert!(!js_blank("\u{85}"));
    }
}
