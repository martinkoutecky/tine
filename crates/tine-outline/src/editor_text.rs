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

/// Properties kept in the file but projected out of the block editor. OG also
/// hides other built-ins outside Tine's current policy. Comment context and
/// author are hidden only on blocks whose own primary properties include
/// `quote` (see [`comment_quote`]).
/// Use this same projection for all editor and kernel text mutations.
pub const BUILTIN_HIDDEN: [&str; 3] = ["id", "collapsed", "logseq.order-list-type"];
const COMMENT_HIDDEN: [&str; 4] = ["quote", "quote-prefix", "quote-suffix", "author"];

/// The shared comment discriminator, also used by the window's `quoteSelectorOf`.
/// The caller supplies only the block's own primary properties. An empty quote
/// still marks a comment; a parent is required only for its card presentation.
pub fn comment_quote<'a>(
    mut properties: impl Iterator<Item = (&'a str, &'a str)>,
) -> Option<&'a str> {
    properties.find_map(|(key, value)| (property_key_norm(key) == "quote").then_some(value))
}

/// Which accepted primary properties the editor hides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HidePolicy {
    /// An ordinary block editor: LOGBOOK, [`BUILTIN_HIDDEN`], and comment metadata on comments.
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
    /// Whether this policy hides `key` for exactly these parsed block regions.
    pub fn hides(&self, key: &str, regions: &BlockRegions) -> bool {
        self.hides_for_comment(key, has_comment_quote(regions))
    }

    /// A session may already hold the accepted own-property facts of a joined
    /// buffer. Query those without parsing the same edited text a second time.
    pub fn hides_properties<'a>(
        &self,
        key: &str,
        properties: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> bool {
        self.hides_for_comment(key, comment_quote(properties).is_some())
    }

    fn hides_for_comment(&self, key: &str, comment: bool) -> bool {
        let lower = key.to_lowercase();
        let builtin = || {
            BUILTIN_HIDDEN.contains(&lower.as_str())
                || (COMMENT_HIDDEN.contains(&property_key_norm(key).as_str()) && comment)
        };
        match self {
            HidePolicy::Builtin => builtin(),
            HidePolicy::SheetCell => builtin() || lower.starts_with("tine."),
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

fn has_comment_quote(regions: &BlockRegions) -> bool {
    comment_quote(
        regions
            .properties
            .iter()
            .filter(|p| p.primary)
            .map(|p| (p.key.as_str(), p.value.as_str())),
    )
    .is_some()
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
    let comment = has_comment_quote(regions);
    for p in own() {
        if hide.hides_for_comment(&p.key, comment) {
            class[line_at(p.line.0)] = Line::Hidden;
        }
    }
    if !matches!(hide, HidePolicy::Keys { .. }) {
        for drawer in regions
            .drawers
            .iter()
            .filter(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
        {
            class[line_at(drawer.range.0)..=line_at(drawer.range.1.saturating_sub(1))]
                .fill(Line::Hidden);
        }
    }
    if is_org {
        for (index, range) in regions.property_regions.iter().enumerate() {
            let mut entries = own().filter(|p| p.region == index).peekable();
            if entries.peek().is_none() || !entries.all(|p| hide.hides_for_comment(&p.key, comment))
            {
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

/// Reattach hidden metadata to editor text. Properties retain their established
/// placement; LOGBOOK goes after title, planning and head properties. A supplied
/// LOGBOOK replaces the retained one. A quarantined Org parse refuses.
pub fn join(visible: &str, hidden: &str, is_org: bool) -> Result<String, String> {
    if hidden.is_empty() {
        return Ok(visible.to_string());
    }
    let regions = block_regions::parse(visible, is_org);
    join_with(visible, hidden, is_org, &regions)
}

/// Extract only real LOGBOOK drawers, leaving property lines for their existing
/// join. Keys-only policies (notably block copy's HIDE_ID) keep drawers visible.
pub fn logbook_hidden(raw: &str, is_org: bool) -> String {
    logbook_parts(raw, is_org).1.join("\n")
}

fn logbook_parts(raw: &str, is_org: bool) -> (String, Vec<String>) {
    let regions = block_regions::parse(raw, is_org);
    let drawers: Vec<_> = regions
        .drawers
        .iter()
        .filter(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
        .collect();
    let mut at = 0;
    let properties = raw
        .split('\n')
        .filter(|line| {
            let start = at;
            at += line.len() + 1;
            !drawers
                .iter()
                .any(|d| d.range.0 <= start && start < d.range.1)
        })
        .collect::<Vec<_>>()
        .join("\n");
    (
        properties,
        drawers
            .iter()
            .map(|d| {
                d.range
                    .slice(raw)
                    .trim_end_matches(['\r', '\n'])
                    .to_string()
            })
            .collect(),
    )
}

fn join_properties(
    visible: &str,
    hidden: &str,
    is_org: bool,
    regions: &BlockRegions,
) -> Result<String, String> {
    if hidden.is_empty() {
        return Ok(visible.to_string());
    }
    if !is_org {
        // Metadata appended after a final quote is its lazy continuation, so it goes
        // before the quote, as the window's `joinProps` places it (GH #684, #721 F3).
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
    regions.reattach_org_properties(visible, hidden)
}

/// Restore property rows independently before restoring history. Every group
/// (including rows originally after LOGBOOK) must retain parser ownership.
fn join_properties_preserving(
    visible: &str,
    hidden: &str,
    is_org: bool,
    regions: &BlockRegions,
) -> Result<String, String> {
    let joined = join_properties(visible, hidden, is_org, regions)?;
    if hidden.is_empty() {
        return Ok(joined);
    }
    let payload = if is_org {
        format!(":PROPERTIES:\n{hidden}\n:END:")
    } else {
        hidden.to_string()
    };
    if regions.insertion_preserves_metadata(visible, is_org, &payload, &joined) {
        return Ok(joined);
    }
    // A lazy container can swallow the usual property position too.
    regions.insert_drawer(visible, is_org, &payload)
}

/// [`join`], given the regions parsed from exactly `visible`.
pub fn join_with(
    visible: &str,
    hidden: &str,
    is_org: bool,
    visible_regions: &BlockRegions,
) -> Result<String, String> {
    if hidden.is_empty() {
        return Ok(visible.to_string());
    }
    // A negative-only fast path; accepted drawer ownership below is always
    // parser-owned. Avoid parsing ordinary hidden property payloads as drawers.
    if !block_regions::may_have_logbook(hidden) {
        // A newly authored/supplied drawer needs the same ownership guard even
        // when there was no prior history. Org's title heuristic otherwise
        // inserts the retained ID inside a titleless supplied drawer.
        if visible_regions
            .drawers
            .iter()
            .any(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
        {
            return join_properties_preserving(visible, hidden, is_org, visible_regions);
        }
        return join_properties(visible, hidden, is_org, visible_regions);
    }
    // Hidden Org property entries have no PROPERTIES wrapper. Parse this
    // transport as Markdown so a standalone :id: cannot own the next drawer.
    let (properties, drawers) = logbook_parts(hidden, false);
    if !drawers.is_empty()
        && visible_regions
            .drawers
            .iter()
            .any(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
    {
        let (text, supplied) = logbook_parts(visible, is_org);
        let retained = [properties, supplied.join("\n")]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return join_with(
            &text,
            &retained,
            is_org,
            &block_regions::parse(&text, is_org),
        );
    }
    // Markdown metadata that preceded the hidden drawer stays in the head;
    // trailing metadata keeps the established trailing-property convention.
    let hidden_regions = block_regions::parse(hidden, false);
    let first_drawer = hidden_regions
        .drawers
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case("LOGBOOK"));
    let raw = if !is_org && first_drawer.is_some_and(|d| d.range.0 > 0) {
        let at = first_drawer.unwrap().range.0;
        let head = hidden[..at].trim_end_matches(['\r', '\n']);
        let (tail, _) = logbook_parts(&hidden[at..], false);
        let base = join_properties_preserving(visible, &tail, false, visible_regions)?;
        block_regions::parse(&base, false).insert_drawer(&base, false, head)?
    } else if !drawers.is_empty() {
        join_properties_preserving(visible, &properties, is_org, visible_regions)?
    } else {
        join_properties(visible, &properties, is_org, visible_regions)?
    };
    if drawers.is_empty() {
        return Ok(raw);
    }
    // Merge preserves both histories in one drawer: survivor rows first, then
    // absorbed rows, without trimming/reformatting clock or other drawer lines.
    let mut drawer = drawers[0].clone();
    for extra in &drawers[1..] {
        let first_regions = block_regions::parse(&drawer, is_org);
        let extra_regions = block_regions::parse(extra, is_org);
        let first = &first_regions.drawers[0];
        let other = &extra_regions.drawers[0];
        let body = extra.find('\n').map_or(other.close, |at| at + 1);
        drawer.insert_str(first.close, &extra[body..other.close]);
    }
    let regions = block_regions::parse(&raw, is_org);
    regions.insert_drawer(&raw, is_org, &drawer)
}

/// Set a block's text as typing `text` into its editor would (Spec §2.4
/// "set-text"): `text` replaces the editor text of `raw` and the hidden
/// property lines ([`HidePolicy::Builtin`]) are kept. The caller's marker
/// transition (`logbook`) follows, as the window's `setRaw`. The command
/// line's `edit` calls this; it is the editor's own projection, so the two
/// cannot disagree on which lines the text owns. A quarantined Org block
/// with hidden lines refuses.
pub fn set_text(raw: &str, is_org: bool, text: &str) -> Result<String, String> {
    let current = split(raw, is_org, &HidePolicy::Builtin);
    join_preserving(text, &current.hidden, is_org, raw)
}

/// Text-only edits can retain the exact metadata layout of the source buffer.
/// The projection's two strings alone cannot recover whether a hidden property
/// preceded a visible planning line. Splice a contiguous visible edit through
/// the same offset map when it touches no hidden bytes; structural replacements
/// and explicit drawer content use the canonical join above.
pub fn join_preserving(
    visible: &str,
    hidden: &str,
    is_org: bool,
    source: &str,
) -> Result<String, String> {
    let regions = block_regions::parse(source, is_org);
    if regions
        .drawers
        .iter()
        .any(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
    {
        let policy = HidePolicy::Builtin;
        let previous = split_with(source, is_org, &regions, &policy);
        if previous.hidden == hidden
            && !block_regions::parse(visible, is_org)
                .drawers
                .iter()
                .any(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
        {
            if previous.visible == visible {
                return Ok(source.to_string());
            }
            if previous.visible.is_empty() {
                return join(visible, hidden, is_org);
            }
            let prefix = previous
                .visible
                .bytes()
                .zip(visible.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            let start = floor_boundary(&previous.visible, prefix);
            let suffix = previous.visible[start..]
                .bytes()
                .rev()
                .zip(visible[start..].bytes().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let old_end = ceil_boundary(&previous.visible, previous.visible.len() - suffix);
            let new_end = ceil_boundary(visible, visible.len() - suffix);
            let from = visible_to_raw_with(source, is_org, &regions, &policy, start);
            let to = visible_to_raw_with(source, is_org, &regions, &policy, old_end);
            let c = classify(source, is_org, &regions, &policy);
            if !c
                .lines
                .iter()
                .zip(c.class)
                .any(|(&(s, e), class)| class != Line::Visible && s < to && e >= from)
            {
                let edited = format!(
                    "{}{}{}",
                    &source[..from],
                    &visible[start..new_end],
                    &source[to..]
                );
                // A syntax edit (for example a new quote opener) can swallow
                // otherwise untouched hidden bytes. Retain their ownership,
                // not only their spelling, or use canonical placement below.
                if split(&edited, is_org, &policy).hidden == hidden {
                    return Ok(edited);
                }
            }
        }
    }
    join(visible, hidden, is_org)
}

fn ceil_boundary(s: &str, mut at: usize) -> usize {
    while !s.is_char_boundary(at) {
        at += 1;
    }
    at
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

    #[test]
    fn gh741_logbook_canonical_restoration_preserves_ownership() {
        let drawer = ":LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]  \nopaque row\n:END:";
        let id = "11111111-1111-4111-8111-111111111111";
        for org in [false, true] {
            let props = if org {
                format!(":PROPERTIES:\n:owner: Ada\n:id: {id}\n:END:")
            } else {
                format!("owner:: Ada\nid:: {id}")
            };
            let raw = format!("DOING x\n{props}\n{drawer}\nBody");
            for text in [
                "",
                "> quoted",
                "> quoted\n> more",
                "- list",
                "1. list",
                "| a | b |",
                "```\nunclosed",
                "#+BEGIN_QUOTE\nunclosed",
            ] {
                let out = set_text(&raw, org, text).unwrap();
                let parsed = block_regions::parse(&out, org);
                assert_eq!(
                    parsed.id.as_ref().map(|p| p.value.as_str()),
                    Some(id),
                    "{org}/{text}: {out}"
                );
                assert_eq!(logbook_hidden(&out, org), drawer, "{org}/{text}: {out}");
                assert_eq!(
                    split(&out, org, &HidePolicy::Builtin).visible,
                    text,
                    "{org}/{text}: {out}"
                );
                // A second edit must still find the retained history.
                assert_eq!(
                    logbook_hidden(&set_text(&out, org, "new").unwrap(), org),
                    drawer
                );
            }
            // An ordinary property remains independently parser-owned too.
            let out = set_text(&raw, org, &format!("{props}\n> quoted")).unwrap();
            let parsed = block_regions::parse(&out, org);
            assert!(
                parsed
                    .properties
                    .iter()
                    .any(|p| p.primary && p.key == "owner" && p.value == "Ada"),
                "{out}"
            );
            assert_eq!(logbook_hidden(&out, org), drawer);
        }
    }

    #[test]
    fn gh741_logbook_canonical_restoration_checks_both_property_groups() {
        let drawer = ":LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:";
        let raw = format!("DOING x\nquote:: selected words\n{drawer}\nauthor:: Ada");
        let out = set_text(&raw, false, "> quoted").unwrap();
        let parsed = block_regions::parse(&out, false);
        for (key, value) in [("quote", "selected words"), ("author", "Ada")] {
            assert!(
                parsed
                    .properties
                    .iter()
                    .any(|p| p.primary && p.key == key && p.value == value),
                "{key}: {out}"
            );
        }
        assert_eq!(logbook_hidden(&out, false), drawer);
    }

    #[test]
    fn gh741_logbook_supplied_titleless_drawer_keeps_hidden_identity() {
        let id = "11111111-1111-4111-8111-111111111111";
        let drawer = ":LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:";
        for org in [false, true] {
            let props = if org {
                format!(":PROPERTIES:\n:id: {id}\n:END:")
            } else {
                format!("id:: {id}")
            };
            let out = set_text(&format!("plain\n{props}"), org, drawer).unwrap();
            assert_eq!(block_regions::parse(&out, org).id.unwrap().value, id);
            assert_eq!(logbook_hidden(&out, org), drawer);
        }
    }

    #[test]
    fn gh741_logbook_hidden_roundtrip_and_content_wins() {
        let drawer = ":LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]  \n:END:";
        for org in [false, true] {
            for raw in [drawer.to_string(), format!("DOING x\n{drawer}\nBody")] {
                let t = split(&raw, org, &HidePolicy::Builtin);
                assert!(!t.visible.contains(":LOGBOOK:"));
                assert_eq!(join(&t.visible, &t.hidden, org).unwrap(), raw);
                assert_eq!(
                    set_text(&raw, org, &t.visible.replace("x", "new")).unwrap(),
                    raw.replace("x", "new")
                );
                let replacement = "DOING new\n:LOGBOOK:\nCLOCK: [2026-10-22 Thu 09:00:00]\n:END:";
                assert_eq!(set_text(&raw, org, replacement).unwrap(), replacement);
                let pasted = "DOING new\nBody\n:LOGBOOK:\nCLOCK: [2026-10-22 Thu 09:00:00]\n:END:";
                assert_eq!(
                    set_text(&raw, org, pasted).unwrap(),
                    format!("{replacement}\nBody")
                );
            }
            let fence = if org {
                "#+BEGIN_SRC text\n:LOGBOOK:\nCLOCK: literal\n:END:\n#+END_SRC"
            } else {
                "```\n:LOGBOOK:\nCLOCK: literal\n:END:\n```"
            };
            assert_eq!(split(fence, org, &HidePolicy::Builtin).visible, fence);
        }
    }

    #[test]
    fn gh741_logbook_planning_properties_and_crlf_keep_bytes() {
        for (org, raw) in [
            (false, "DOING x\nSCHEDULED: <2026-10-21 Wed>\nowner:: Ada\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]  \n:END:\nBody"),
            (false, "DOING x\nid:: 6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:\nBody"),
            (true, "DOING x\n:PROPERTIES:\n:owner: Ada\n:id: 6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b\n:END:\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:\nBody"),
            (false, "DOING x\r\n:LOGBOOK:\r\nCLOCK: [2026-10-21 Wed 08:54:45]  \r\n:END:\r\nBody"),
            (false, "DOING x\ncollapsed:: true\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:\nDEADLINE: <2026-10-22 Thu>"),
        ] {
            let text = split(raw, org, &HidePolicy::Builtin).visible.replace("x", "new");
            assert_eq!(set_text(raw, org, &text).unwrap(), raw.replacen("x", "new", 1));
        }
    }

    #[test]
    fn gh741_logbook_titleless_and_unicode_edits() {
        for org in [false, true] {
            let drawer = ":LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:";
            assert_eq!(
                set_text(drawer, org, "new").unwrap(),
                format!("new\n{drawer}")
            );
            for (before, after) in [("é🙂", "ô🙂"), ("é🙂", "é🙃"), ("plain", "é🙂")]
            {
                let raw = format!("DOING {before}\n{drawer}\nBody");
                assert_eq!(
                    set_text(&raw, org, &format!("DOING {after}\nBody")).unwrap(),
                    format!("DOING {after}\n{drawer}\nBody")
                );
            }
        }
    }

    #[test]
    fn gh741_logbook_survives_replacing_text_with_literal_containers() {
        for org in [false, true] {
            let raw = "DOING x\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:";
            let text = if org {
                "#+BEGIN_SRC text\n:LOGBOOK:\nCLOCK: literal\n:END:\n#+END_SRC"
            } else {
                "```\n:LOGBOOK:\nCLOCK: literal\n:END:\n```"
            };
            let edited = set_text(raw, org, text).unwrap();
            assert!(crate::logbook::has_logbook_drawer(&edited, org));
            assert_eq!(split(&edited, org, &HidePolicy::Builtin).visible, text);
        }
        let raw = "DOING x\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:";
        let edited = set_text(raw, false, "> quoted\n> continuation").unwrap();
        assert!(crate::logbook::has_logbook_drawer(&edited, false));
        assert_eq!(
            split(&edited, false, &HidePolicy::Builtin).visible,
            "> quoted\n> continuation"
        );
    }

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
            let out = set_text(raw, false, text).unwrap();
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

    #[test]
    fn set_text_keeps_hidden_properties_and_takes_the_text() {
        let md = "TODO old\nid:: 6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b\ncolor:: red";
        assert_eq!(
            set_text(md, false, "DONE new\ncolor:: blue").unwrap(),
            "DONE new\ncolor:: blue\nid:: 6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b"
        );
        let org = "* old\n:PROPERTIES:\n:id: 6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b\n:END:";
        let set = set_text(org, true, "new").unwrap();
        assert!(set.starts_with("new"), "{set:?}");
        assert!(
            set.contains(":id: 6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b"),
            "{set:?}"
        );
        assert_eq!(set_text("plain", false, "other").unwrap(), "other");
    }

    #[test]
    fn ordinary_metadata_stays_visible_and_body_edits_preserve_bytes() {
        for (is_org, raw) in [
            (false, "The obstacle is the way.\nauthor:: Marcus Aurelius  \t\nquote-suffix:: after  "),
            (true, "The obstacle is the way.\n:PROPERTIES:\n:author: Marcus Aurelius  \t\n:quote-suffix: after  \n:END:"),
        ] {
            let t = split(raw, is_org, &HidePolicy::Builtin);
            assert_eq!(t.visible, raw);
            assert_eq!(t.hidden, "");
            let edited = raw.replacen("The obstacle", "An obstacle", 1);
            assert_eq!(set_text(raw, is_org, &edited).unwrap(), edited);
        }
    }

    #[test]
    fn key_queries_and_projection_share_the_own_quote_discriminator() {
        for (is_org, raw, comment) in [
            (false, "Body\nauthor:: Ada\nquote-suffix:: after", false),
            (
                false,
                "Body\nQuOtE:: \nauthor:: Ada\nquote-suffix:: after",
                true,
            ),
            (
                true,
                "Body\n:PROPERTIES:\n:author: Ada\n:quote-suffix: after\n:END:",
                false,
            ),
            (
                true,
                "Body\n:PROPERTIES:\n:QuOtE: \n:author: Ada\n:quote-suffix: after\n:END:",
                true,
            ),
            (
                false,
                "Body\nauthor:: Ada\n```\nquote:: literal\n```",
                false,
            ),
        ] {
            let regions = block_regions::parse(raw, is_org);
            for hide in [HidePolicy::Builtin, HidePolicy::SheetCell] {
                let t = split_with(raw, is_org, &regions, &hide);
                for key in COMMENT_HIDDEN {
                    assert_eq!(hide.hides(key, &regions), comment, "{raw:?}: {key}");
                    assert_eq!(
                        hide.hides_properties(
                            key,
                            regions
                                .properties
                                .iter()
                                .filter(|p| p.primary)
                                .map(|p| (p.key.as_str(), p.value.as_str()))
                        ),
                        comment,
                    );
                }
                assert_eq!(t.hidden.contains("author"), comment);
                assert_eq!(t.visible.contains("author"), !comment);
                for key in BUILTIN_HIDDEN {
                    assert!(hide.hides(key, &regions));
                }
            }
        }
    }

    #[test]
    fn comment_metadata_is_hidden_and_survives_body_edits_byte_exact() {
        for (is_org, raw, hidden) in [
            (false, "My view\nQuOtE:: selected words  \nquote-prefix:: before\nquote-suffix:: after\nauthor:: Ada Test\ncollapsed:: false", "QuOtE:: selected words  \nquote-prefix:: before\nquote-suffix:: after\nauthor:: Ada Test\ncollapsed:: false"),
            (true, "My view\n:PROPERTIES:\n:QuOtE: selected words  \n:quote-prefix: before\n:quote-suffix: after\n:author: Ada Test\n:collapsed: false\n:END:", ":QuOtE: selected words  \n:quote-prefix: before\n:quote-suffix: after\n:author: Ada Test\n:collapsed: false"),
        ] {
            let t = split(raw, is_org, &HidePolicy::Builtin);
            assert_eq!(t.visible, "My view", "{raw:?}");
            assert_eq!(t.hidden, hidden);
            let edited = set_text(raw, is_org, "My revised view").unwrap();
            assert_eq!(split(&edited, is_org, &HidePolicy::Builtin).hidden, hidden);
            assert_eq!(edited, raw.replacen("My view", "My revised view", 1));
        }
    }

    #[test]
    fn handwritten_comment_properties_remain_accepted_content_on_save() {
        for (is_org, typed) in [
            (false, "My view\nquote:: selected words\nauthor:: Ada"),
            (
                true,
                "My view\n:PROPERTIES:\n:quote: selected words\n:author: Ada\n:END:",
            ),
        ] {
            // Typing metadata by hand saves the entered bytes, just as before;
            // the next editor session projects those accepted properties out.
            assert_eq!(set_text("old", is_org, typed).unwrap(), typed);
            let regions = block_regions::parse(typed, is_org);
            assert!(regions
                .properties
                .iter()
                .any(|p| p.key == "quote" && p.value == "selected words"));
        }
        let fenced = "```\nquote:: literal words\nauthor:: example\n```";
        assert_eq!(split(fenced, false, &HidePolicy::Builtin).visible, fenced);
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
