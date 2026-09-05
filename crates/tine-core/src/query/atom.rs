//! The ONE property atomizer and classifier (SPEC §6.2).
//!
//! **Atomization is transcribed (D-9, M22); typing is Tine's own — OG is
//! untyped.** Every transcribed rule cites `deps/graph-parser/src/logseq/
//! graph_parser/text.cljs` (and `property.cljs`) at the read-only OG checkout
//! `/aux/koutecky/logseq/og` commit `6e7afa8eb` (`git describe`:
//! `1.0.0-12-g6e7afa8eb`; the spec's "0.10.15" label is imprecise — Wave A
//! recorded the correction).
//!
//! The rule, in OG's order (`parse-property`, `text.cljs:165-186`), with v12's
//! reading of step 1 (VERIFY-11 A1):
//!
//! 1. key ∈ `unparsed-built-in-properties` ∪ `config.ignored_page_references_keywords`
//!    → **no reference parsing**: step 3 is skipped entirely and the value's
//!    `[[x]]`/`#x` text stays literal inside the plain segments. Steps 2 and 4
//!    still run — Q21's comma split is decided for EVERY key, and a literally
//!    transcribed one-atom rule would silently remove matches today's
//!    `value_matches` gives (it splits these keys too).
//! 2. the value is wrapped in double quotes (`wrapped-by-quotes?`) → one
//!    `Plain` atom, the trimmed raw text **including the quotes** (K19).
//! 3. the value's page refs ∪, for a comma-configured key, the comma-split
//!    plain segments → if non-empty, those are the atoms and any other plain
//!    text is dropped. **Ordering and collision are Tine's (K19, J8):** refs
//!    first in document order, then comma segments in text order,
//!    de-duplicated by [`atom_key`] with first occurrence winning.
//! 4. else (**Q21**): split the trimmed text on `,`/`，` into `Plain` atoms —
//!    Tine's intentional divergence from OG, which keeps one string here.
//!
//! The value is parsed with `lsdoc::inline(value, format)` — the transcription
//! of OG parsing the value with mldoc in `extract-refs-by-commas` /
//! `extract-refs-from-mldoc-ast`. It is **not** read off
//! `BlockProjection.refs_page`, which aggregates the whole block's refs without
//! saying which property value produced each (K11).

use unicode_normalization::UnicodeNormalization;

use crate::config::ParseConfig;
use crate::date::{JournalDate, JournalFormat};
use crate::doc::property_key_norm;
use crate::query::ir::ObservedType;

/// Whether a page is Markdown or Org — the only thing the atomizer needs to
/// know about the file it came from (the inline grammar differs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AtomFormat {
    #[default]
    Markdown,
    Org,
}

impl From<crate::model::Format> for AtomFormat {
    fn from(format: crate::model::Format) -> AtomFormat {
        match format {
            crate::model::Format::Org => AtomFormat::Org,
            crate::model::Format::Md => AtomFormat::Markdown,
        }
    }
}

impl AtomFormat {
    fn lsdoc_name(self) -> &'static str {
        match self {
            AtomFormat::Markdown => "markdown",
            AtomFormat::Org => "org",
        }
    }
}

/// Where an atom came from: an explicit page reference in the value, or a plain
/// text segment. Only `ordinal` and `origin` depend on the ordering rule, and
/// `origin` affects only the registry's `ref` class, never a match (§6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AtomOrigin {
    Ref,
    Plain,
}

/// One atom of one property element (SPEC §3.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Atom {
    /// The atom text exactly as the value spelled it (quotes kept for a quoted
    /// value; brackets kept for a literal `[[y]]` under a step-1 key).
    pub text: String,
    /// NFC-lowercased trimmed `text` (Q20, M11) — the comparison and de-dup key.
    pub key: String,
    pub origin: AtomOrigin,
    /// Filled whenever the atom classifies as a number.
    pub num: Option<f64>,
    /// Filled whenever the atom classifies as a date (`yyyymmdd`).
    pub day: Option<i64>,
    /// Position within the key's flattened atom list, renumbered `0..n` by the
    /// registry producer (§5.8); the atomizer numbers within one value.
    pub ordinal: u32,
    /// The UTF-16 length of OG's stored value for the `key:: value` line this
    /// atom came from, when OG's `text/parse-property` holds that value as a
    /// bare **string**; `None` when OG holds a set (refs, or a comma-separated
    /// key) or a parsed number/boolean.
    ///
    /// It exists because OG's `:property` rule ends in `(contains? ?v ?val)`,
    /// and `contains?` on a ClojureScript string is an INDEX lookup, so the
    /// only thing the rule needs from the stored string is its length
    /// (`rules.cljc:129-138`; measured, `shapes.cljs`). Read ONLY by the four
    /// counterfactual §8.1 modes — never by production matching, which is
    /// [`CompareMode::Both`] (SPEC §8 v16 evidence correction: "Do not add OG's
    /// string-index quirk to production typed matching").
    pub og_string_len: Option<u32>,
}

/// The comparison form of an atom's text: **NFC-lowercased trimmed** (Q20).
///
/// This is deliberately NOT `refs::page_key`: that function additionally removes
/// one slash at each boundary because a page's *identity* does not depend on
/// them, and an atom is not a page name (`/x/` is a value a user may have typed
/// and compared against). The Unicode steps — `to_lowercase` then NFC, never
/// NFKC and never accent folding — are the same, so a ref atom and the page it
/// names still compare equal for every name without a boundary slash.
pub fn atom_key(text: &str) -> String {
    text.trim().to_lowercase().nfc().collect()
}

/// OG `gp-property/unparsed-built-in-properties` (`property.cljs:110-121`):
/// `(hidden-built-in ∪ editable-built-in) − built-in-extended(∅) −
/// editable-linkable − keys(built-in-property-types)`, expanded here because
/// Tine has no Clojure set algebra at runtime. Names are `(name k)` of the
/// keyword, i.e. what a `key::` line spells.
const UNPARSED_BUILT_IN_PROPERTIES: &[&str] = &[
    // from hidden-built-in-properties (`property.cljs:68-79`)
    "id",
    "custom-id",
    "background-color",
    "background_color",
    "query-properties",
    "query-sort-by",
    "ls-type",
    "hl-type",
    "hl-color",
    "logseq.macro-name",
    "logseq.macro-arguments",
    "logseq.order-list-type",
    "logseq.tldraw.page",
    "logseq.tldraw.shape",
    // from editable-built-in-properties (`property.cljs:58-66`)
    "title",
    "icon",
    "template",
    "filters",
    "macro",
    "filetags",
    "logseq.color",
    "logseq.table.version",
    "logseq.table.compact",
    "logseq.table.headers",
    "logseq.table.hover",
    "logseq.table.borders",
    "logseq.table.stripes",
    "logseq.table.max-width",
];

/// OG `gp-property/editable-linkable-built-in-properties` (`property.cljs:46-48`).
const EDITABLE_LINKABLE_BUILT_IN_PROPERTIES: &[&str] = &["alias", "aliases", "tags"];

fn contains_key(list: &[&str], key: &str) -> bool {
    list.iter()
        .any(|candidate| property_key_norm(candidate) == key)
}

fn contains_config_key(list: &[String], key: &str) -> bool {
    list.iter()
        .any(|candidate| property_key_norm(candidate.trim_start_matches(':')) == key)
}

/// OG `separated-by-commas?` (`text.cljs:141-146`).
fn separated_by_commas(key: &str, config: &ParseConfig) -> bool {
    contains_key(EDITABLE_LINKABLE_BUILT_IN_PROPERTIES, key)
        || contains_config_key(&config.separated_by_commas, key)
}

/// Step 1's key test: OG `parse-property`'s first `cond` branch
/// (`text.cljs:169-176`).
fn reference_parsing_suppressed(key: &str, config: &ParseConfig) -> bool {
    contains_key(UNPARSED_BUILT_IN_PROPERTIES, key)
        || contains_config_key(&config.ignored_page_references_keywords, key)
}

/// OG `gp-util/wrapped-by-quotes?`: a trimmed value of length > 1 starting and
/// ending with `"`.
fn wrapped_by_quotes(value: &str) -> bool {
    value.len() > 1 && value.starts_with('"') && value.ends_with('"')
}

/// OG `text/parse-non-string-property-value` (`text.cljs:87-98`): `"true"` and
/// `"false"` become booleans, an unsigned run of ASCII digits becomes an
/// integer, everything else stays the string it was written as.
fn og_parses_as_non_string(value: &str) -> bool {
    value == "true"
        || value == "false"
        || (!value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
}

/// The [`Atom::og_string_len`] of a value OG holds as a bare string: its length
/// in UTF-16 code units, because that is what ClojureScript's `(.-length s)`
/// counts and what the `contains?` index bound compares against.
fn og_string_len(value: &str) -> Option<u32> {
    Some(value.encode_utf16().count() as u32)
}

/// OG `sep-by-comma` (`text.cljs:132-139`): split on one `,` or `，`, trim, drop
/// blanks. OG returns a set; Tine keeps text order (K19).
fn sep_by_comma(value: &str) -> Vec<&str> {
    value
        .split([',', '，'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// OG `get-ref-from-ast` (`text.cljs:100-119`) over one lsdoc inline node: a
/// `Link` whose url is `Page_ref` or `Search`, a `Nested_link`, or a `Tag`.
fn ref_from_inline(node: &lsdoc::ast::Inline, out: &mut Vec<String>) {
    use lsdoc::ast::{Inline, Url};
    match node {
        Inline::Link { url, .. } => match url {
            Url::PageRef { v } | Url::Search { v } => out.push(v.trim().to_string()),
            _ => {}
        },
        Inline::NestedLink { content, .. } => out.push(content.trim().to_string()),
        Inline::Tag { children, .. } => {
            let mut text = String::new();
            tag_plain_text(children, &mut text);
            if !text.trim().is_empty() {
                out.push(text.trim().to_string());
            }
        }
        _ => {}
    }
}

/// The text of a `Tag`'s children — OG's `get-ref-from-ast` `"Tag"` branch takes
/// the first child's `Plain` text (or recurses into a link).
fn tag_plain_text(children: &[lsdoc::ast::Inline], out: &mut String) {
    use lsdoc::ast::{Inline, Url};
    for child in children {
        match child {
            Inline::Plain { text, .. } => out.push_str(text),
            Inline::Link { url, .. } => match url {
                Url::PageRef { v } | Url::Search { v } => out.push_str(v),
                _ => {}
            },
            Inline::NestedLink { content, .. } => out.push_str(content),
            _ => {}
        }
    }
}

/// The plain-text segments of the parsed value — OG `extract-refs-by-commas`
/// (`text.cljs:148-154`) reads exactly the `Plain` nodes.
fn plain_segments(nodes: &[lsdoc::ast::Inline], out: &mut Vec<String>) {
    use lsdoc::ast::Inline;
    for node in nodes {
        if let Inline::Plain { text, .. } = node {
            out.push(text.clone());
        }
    }
}

/// The five counterfactual modes of SPEC §8.1, as parameters of the atomizer
/// and the comparator.
///
/// Gate 1 asks a question no single implementation can answer: when the walk
/// and OG disagree on a corpus query, WHICH decision caused it? The only honest
/// way to answer is to run the same walk with each decision switched off and
/// see which switch closes the gap. These are those switches — production code,
/// not a test scaffold, because gate 1 runs them from an example binary over
/// real graphs.
///
/// [`CompareMode::Both`] is Tine. Every other mode is a deliberate regression
/// towards OG, and nothing in the product ever selects one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CompareMode {
    /// OG's own rule: OG's split (`alias`/`aliases`/`tags` plus the configured
    /// `separated-by-commas` keys, and no other key), case-SENSITIVE property
    /// equality, case-insensitive page identity for refs, and no effective-type
    /// coercion — an atom matching `^\d+$` compares as an integer to a numeric
    /// literal, every other atom as text (v13 §8.1, Y3).
    Og,
    /// OG, plus Q20: atom identity is NFC-lowercased.
    Q20Only,
    /// OG, plus Q21: every key splits on commas.
    Q21Only,
    /// Q20 and Q21, with OG's comparison otherwise — no coercion.
    BothUntyped,
    /// Tine: Q20, Q21 and §6.3 effective-type coercion.
    #[default]
    Both,
}

impl CompareMode {
    /// Whether the comma split applies to EVERY key (Q21) or only to the keys
    /// OG splits.
    pub fn splits_every_key(self) -> bool {
        matches!(
            self,
            CompareMode::Q21Only | CompareMode::BothUntyped | CompareMode::Both
        )
    }

    /// Whether atom identity folds case and normalizes to NFC (Q20).
    pub fn folds_case(self) -> bool {
        matches!(
            self,
            CompareMode::Q20Only | CompareMode::BothUntyped | CompareMode::Both
        )
    }

    /// The same mode with case folding switched ON, for the keys OG resolves
    /// to page names (`tags`, `alias`, `aliases`), whose identity is
    /// case-insensitive in OG too. Folding is the only decision this changes,
    /// so an OG-ward mode stays OG-ward.
    pub fn folding_case(self) -> CompareMode {
        match self {
            CompareMode::Og => CompareMode::Q20Only,
            CompareMode::Q21Only => CompareMode::BothUntyped,
            other => other,
        }
    }

    /// Whether a property atom is coerced by its key's effective type (§6.3).
    /// Only Tine does; every OG-ward mode compares as OG compares.
    pub fn coerces_by_effective_type(self) -> bool {
        matches!(self, CompareMode::Both)
    }

    /// The five modes in the order gate 1 reports them.
    pub fn all() -> [CompareMode; 5] {
        [
            CompareMode::Og,
            CompareMode::Q20Only,
            CompareMode::Q21Only,
            CompareMode::BothUntyped,
            CompareMode::Both,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            CompareMode::Og => "OG",
            CompareMode::Q20Only => "Q20-only",
            CompareMode::Q21Only => "Q21-only",
            CompareMode::BothUntyped => "both-untyped",
            CompareMode::Both => "both",
        }
    }
}

/// Atom identity under one mode: Tine's NFC-lowercased [`atom_key`], or — for
/// the OG-ward modes — the trimmed text as written, because OG's property
/// equality is case-SENSITIVE (measured, `case.cljs`).
pub fn atom_key_in(text: &str, mode: CompareMode) -> String {
    if mode.folds_case() {
        atom_key(text)
    } else {
        text.trim().to_string()
    }
}

/// The ONE atomizer (SPEC §6.2), as Tine runs it. `key` is the raw source key;
/// it is normalized with the existing [`property_key_norm`] before every rule
/// test.
pub fn property_atoms(
    key: &str,
    value: &str,
    format: AtomFormat,
    config: &ParseConfig,
) -> Vec<Atom> {
    property_atoms_in(key, value, format, config, CompareMode::Both)
}

/// [`property_atoms`] under one of the §8.1 modes. Gate 1's only entry point;
/// everything in the product calls [`property_atoms`].
pub fn property_atoms_in(
    key: &str,
    value: &str,
    format: AtomFormat,
    config: &ParseConfig,
    mode: CompareMode,
) -> Vec<Atom> {
    let key_norm = property_key_norm(key);
    let trimmed = value.trim();
    if trimmed.is_empty() {
        // A present key with an empty or whitespace value has presence and zero
        // atoms (§3.3) — the `IsBlank` case.
        return Vec::new();
    }

    // OG's `text/parse-property` decides the SHAPE of the stored value, and its
    // shape is what the `:property` rule's `contains?` branch means: set
    // membership on a set, an index lookup on a string, always false on a
    // number. Steps 1 and 2 below return the raw string; step 3 is OG's set;
    // step 4 is OG's `parse-non-string-property-value` or the string. Recorded
    // per atom as `og_string_len`, and read only by the §8.1 modes.
    let suppressed = reference_parsing_suppressed(&key_norm, config);

    // Step 2 — a quoted value is one atom, quotes included (K19). OG checks this
    // after the unparsed-key branch and so do we; for a step-1 key OG returns the
    // same raw string either way.
    if wrapped_by_quotes(trimmed) {
        return vec![make_atom(
            trimmed.to_string(),
            AtomOrigin::Plain,
            0,
            config,
            mode,
            // Both branches hand `v'` back unparsed, so OG holds the string.
            og_string_len(trimmed),
        )];
    }

    let mut atoms: Vec<Atom> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    // Step 3 — refs, plus comma segments for a comma-configured key. Skipped
    // entirely for a step-1 key (A1).
    if !suppressed {
        let nodes = lsdoc::inline(trimmed, format.lsdoc_name());
        let mut refs: Vec<String> = Vec::new();
        for node in &nodes {
            ref_from_inline(node, &mut refs);
        }
        let mut segments: Vec<String> = Vec::new();
        if separated_by_commas(&key_norm, config) {
            let mut plains: Vec<String> = Vec::new();
            plain_segments(&nodes, &mut plains);
            for plain in &plains {
                for segment in sep_by_comma(plain) {
                    segments.push(segment.to_string());
                }
            }
        }
        // OG's step 3 returns a SET (`(if (seq refs) refs …)`), and `contains?`
        // on a set is real membership — no index lookup — so these atoms carry
        // no `og_string_len`.
        for text in refs {
            push_atom(
                &mut atoms,
                &mut seen,
                text,
                AtomOrigin::Ref,
                config,
                mode,
                None,
            );
        }
        for text in segments {
            push_atom(
                &mut atoms,
                &mut seen,
                text,
                AtomOrigin::Plain,
                config,
                mode,
                None,
            );
        }
        if !atoms.is_empty() {
            return atoms;
        }
    }

    // Step 4 — Q21: split on commas for every key. A value with no comma is one
    // atom, which is exactly OG's single string for the keys OG does not split.
    // Under an OG-ward mode without Q21 the value stays whole, which is what
    // makes `Q21-sufficient` an attributable label rather than a guess.
    let segments: Vec<String> = if mode.splits_every_key() {
        sep_by_comma(trimmed)
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        vec![trimmed.to_string()]
    };
    // OG's own value here is the WHOLE trimmed line — `v'`, never a segment —
    // held as a string unless `parse-non-string-property-value` claimed it, or
    // unconditionally as a string for a step-1 key (that branch returns before
    // the number parse). Q21's split changes which ATOMS exist; it does not
    // change what OG stored, so the same length rides on every segment.
    let stored = (suppressed || !og_parses_as_non_string(trimmed))
        .then(|| og_string_len(trimmed))
        .flatten();
    for segment in segments {
        push_atom(
            &mut atoms,
            &mut seen,
            segment,
            AtomOrigin::Plain,
            config,
            mode,
            stored,
        );
    }
    atoms
}

#[allow(clippy::too_many_arguments)]
fn push_atom(
    atoms: &mut Vec<Atom>,
    seen: &mut Vec<String>,
    text: String,
    origin: AtomOrigin,
    config: &ParseConfig,
    mode: CompareMode,
    og_string_len: Option<u32>,
) {
    let key = atom_key_in(&text, mode);
    if key.is_empty() || seen.iter().any(|existing| existing == &key) {
        return;
    }
    seen.push(key);
    let ordinal = atoms.len() as u32;
    atoms.push(make_atom(
        text,
        origin,
        ordinal,
        config,
        mode,
        og_string_len,
    ));
}

fn make_atom(
    text: String,
    origin: AtomOrigin,
    ordinal: u32,
    config: &ParseConfig,
    mode: CompareMode,
    og_string_len: Option<u32>,
) -> Atom {
    let key = atom_key_in(&text, mode);
    let class = classify_text(&text, origin, config);
    Atom {
        num: (class == ObservedType::Number)
            .then(|| parse_number(&text))
            .flatten(),
        day: (class == ObservedType::Date)
            .then(|| classify_day(&text, config))
            .flatten(),
        text,
        key,
        origin,
        ordinal,
        og_string_len,
    }
}

/// The number rule: `f64::from_str` accepts it and it is finite (§6.2; Tine
/// additionally fills `num` beyond OG's `^\d+$` integer rule).
fn parse_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let parsed: f64 = trimmed.parse().ok()?;
    parsed.is_finite().then_some(parsed)
}

/// The date rule: `yyyy-mm-dd`, or an 8-digit `yyyymmdd` that is a valid
/// calendar date in 1900–2100, or a title in `journal_page_title_format` ONLY
/// (through [`JournalFormat::parse_title`], not `parse`, which also holds the
/// file pattern and the defaults — B6).
fn classify_day(text: &str, config: &ParseConfig) -> Option<i64> {
    let trimmed = text.trim();
    if let Some(day) = iso_day(trimmed) {
        return Some(day);
    }
    if let Some(day) = compact_day(trimmed) {
        return Some(day);
    }
    JournalFormat::new(
        config.journal_file_name_format.as_deref(),
        config.journal_page_title_format.as_deref(),
    )
    .parse_title(trimmed)
    .map(|date| date.ordinal_key())
}

fn iso_day(text: &str) -> Option<i64> {
    let mut parts = text.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || text.len() != 10 {
        return None;
    }
    valid_day(year, month, day)
}

fn compact_day(text: &str) -> Option<i64> {
    if text.len() != 8 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i32 = text[0..4].parse().ok()?;
    if !(1900..=2100).contains(&year) {
        return None;
    }
    valid_day(year, text[4..6].parse().ok()?, text[6..8].parse().ok()?)
}

fn valid_day(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month)
        || day < 1
        || day > u32::from(crate::date::days_in_month(year, month))
    {
        return None;
    }
    Some(JournalDate { year, month, day }.ordinal_key())
}

/// Classification of one atom, first match wins (§6.2): **checkbox** if exactly
/// `true`/`false`; **date**; **number**; **ref** if `origin = Ref`; else
/// **text**.
pub fn classify_text(text: &str, origin: AtomOrigin, config: &ParseConfig) -> ObservedType {
    let trimmed = text.trim();
    if trimmed == "true" || trimmed == "false" {
        return ObservedType::Checkbox;
    }
    if classify_day(trimmed, config).is_some() {
        return ObservedType::Date;
    }
    if parse_number(trimmed).is_some() {
        return ObservedType::Number;
    }
    if origin == AtomOrigin::Ref {
        return ObservedType::Ref;
    }
    ObservedType::Text
}

impl Atom {
    /// The class this atom belongs to in the registry's histogram.
    pub fn class(&self, config: &ParseConfig) -> ObservedType {
        classify_text(&self.text, self.origin, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ParseConfig {
        ParseConfig::default()
    }

    fn texts(atoms: &[Atom]) -> Vec<String> {
        atoms.iter().map(|atom| atom.text.clone()).collect()
    }

    fn md(key: &str, value: &str, config: &ParseConfig) -> Vec<Atom> {
        property_atoms(key, value, AtomFormat::Markdown, config)
    }

    // --- SPEC §6.2 frozen fixture vectors ---------------------------------

    #[test]
    fn plain_value_is_one_plain_atom() {
        let atoms = md("k", "foo", &config());
        assert_eq!(
            atoms,
            vec![Atom {
                text: "foo".into(),
                key: "foo".into(),
                origin: AtomOrigin::Plain,
                num: None,
                day: None,
                ordinal: 0,
                og_string_len: Some(3),
            }]
        );
    }

    #[test]
    fn a_ref_value_is_one_ref_atom() {
        let atoms = md("k", "[[a]]", &config());
        assert_eq!(texts(&atoms), vec!["a"]);
        assert_eq!(atoms[0].origin, AtomOrigin::Ref);
        assert_eq!(atoms[0].key, "a");
    }

    #[test]
    fn mixed_value_drops_the_plain_text_and_keeps_the_ref() {
        assert_eq!(texts(&md("k", "foo [[a]]", &config())), vec!["a"]);
    }

    #[test]
    fn two_refs_keep_document_order_including_a_tag() {
        let atoms = md("k", "[[a]] #b", &config());
        assert_eq!(texts(&atoms), vec!["a", "b"]);
        assert!(atoms.iter().all(|atom| atom.origin == AtomOrigin::Ref));
        assert_eq!(atoms[1].ordinal, 1);
    }

    #[test]
    fn a_comma_configured_key_splits_its_plain_segments() {
        assert_eq!(texts(&md("tags", "a, b", &config())), vec!["a", "b"]);
    }

    #[test]
    fn q21_splits_a_non_configured_key_too() {
        // The intentional divergence: OG keeps `"a, b"` as one string here.
        assert_eq!(texts(&md("k", "a, b", &config())), vec!["a", "b"]);
    }

    #[test]
    fn q21_costs_a_decimal_comma_its_number() {
        let atoms = md("k", "1,5", &config());
        assert_eq!(texts(&atoms), vec!["1", "5"]);
        assert_eq!(atoms[0].num, Some(1.0));
        assert_eq!(atoms[1].num, Some(5.0));
    }

    #[test]
    fn an_empty_value_has_presence_and_zero_atoms() {
        assert!(md("k", "", &config()).is_empty());
        assert!(md("k", "   ", &config()).is_empty());
    }

    #[test]
    fn a_repeated_ref_is_one_atom() {
        assert_eq!(texts(&md("k", "[[a]] [[a]]", &config())), vec!["a"]);
    }

    #[test]
    fn a_quoted_value_is_one_atom_with_its_quotes() {
        let atoms = md("k", "\"x, [[y]]\"", &config());
        assert_eq!(texts(&atoms), vec!["\"x, [[y]]\""]);
        assert_eq!(atoms[0].origin, AtomOrigin::Plain);
    }

    #[test]
    fn an_integer_is_plain_with_a_number() {
        let atoms = md("k", "12", &config());
        assert_eq!(atoms[0].origin, AtomOrigin::Plain);
        assert_eq!(atoms[0].num, Some(12.0));
    }

    #[test]
    fn a_decimal_is_plain_with_a_number_tine_side_only() {
        let atoms = md("k", "1.5", &config());
        assert_eq!(atoms[0].num, Some(1.5));
    }

    #[test]
    fn a_ref_that_collides_with_a_plain_segment_keeps_the_ref_origin() {
        let atoms = md("tags", "[[a]], a", &config());
        assert_eq!(texts(&atoms), vec!["a"]);
        assert_eq!(atoms[0].origin, AtomOrigin::Ref);
    }

    // --- v12 §6.2 step-1 fixtures (VERIFY-11 A1) ---------------------------

    #[test]
    fn an_ignored_reference_key_keeps_its_brackets_literal_and_still_splits() {
        let mut config = ParseConfig::default();
        config.ignored_page_references_keywords = vec!["url".into()];
        let atoms = md("url", "http://a.b/x, [[y]]", &config);
        assert_eq!(texts(&atoms), vec!["http://a.b/x", "[[y]]"]);
        assert!(
            atoms.iter().all(|atom| atom.origin == AtomOrigin::Plain),
            "step 1 suppresses reference parsing, so `[[y]]` is literal text"
        );
    }

    #[test]
    fn an_unparsed_built_in_without_a_comma_is_one_atom() {
        assert_eq!(
            texts(&md("template", "weekly review", &config())),
            vec!["weekly review"]
        );
    }

    #[test]
    fn an_unparsed_built_in_with_a_comma_still_splits_q21() {
        assert_eq!(texts(&md("title", "A, B", &config())), vec!["A", "B"]);
    }

    // --- classification ----------------------------------------------------

    #[test]
    fn classification_order_is_checkbox_date_number_ref_text() {
        let config = config();
        assert_eq!(
            classify_text("true", AtomOrigin::Plain, &config),
            ObservedType::Checkbox
        );
        assert_eq!(
            classify_text("2026-09-04", AtomOrigin::Plain, &config),
            ObservedType::Date
        );
        assert_eq!(
            classify_text("20260904", AtomOrigin::Plain, &config),
            ObservedType::Date
        );
        assert_eq!(
            classify_text("12", AtomOrigin::Plain, &config),
            ObservedType::Number
        );
        assert_eq!(
            classify_text("a", AtomOrigin::Ref, &config),
            ObservedType::Ref
        );
        assert_eq!(
            classify_text("hello", AtomOrigin::Plain, &config),
            ObservedType::Text
        );
    }

    #[test]
    fn a_malformed_calendar_date_is_not_a_date() {
        let config = config();
        for text in ["2026-13-45", "2026-02-30", "2023-02-29", "20261345"] {
            assert_ne!(
                classify_text(text, AtomOrigin::Plain, &config),
                ObservedType::Date,
                "{text}"
            );
        }
        assert_eq!(
            classify_text("2024-02-29", AtomOrigin::Plain, &config),
            ObservedType::Date
        );
    }

    /// B6: date classification reads the TITLE pattern alone — never the file
    /// pattern and never `JournalFormat`'s default fallback list.
    #[test]
    fn a_journal_title_is_a_date_but_the_file_stem_is_text() {
        let mut config = ParseConfig::default();
        config.journal_file_name_format = Some("yyyy_MM_dd".into());
        config.journal_page_title_format = Some("MMM do, yyyy".into());
        assert_eq!(
            classify_text("Sep 4th, 2026", AtomOrigin::Plain, &config),
            ObservedType::Date
        );
        assert_eq!(
            classify_text("2026_09_04", AtomOrigin::Plain, &config),
            ObservedType::Text
        );
    }

    /// The accepted cost of Q21, stated as a test rather than left to be
    /// discovered: the comma split runs BEFORE classification, so a journal
    /// title whose format contains a comma (`MMM do, yyyy` — Logseq's default)
    /// is two atoms in a property value and therefore not a date. Quoting opts
    /// out, exactly as it does for `"Smith, John"`.
    #[test]
    fn q21_splits_a_comma_bearing_journal_title_before_it_can_classify_as_a_date() {
        let mut config = ParseConfig::default();
        config.journal_page_title_format = Some("MMM do, yyyy".into());
        assert_eq!(
            texts(&md("k", "Sep 4th, 2026", &config)),
            vec!["Sep 4th", "2026"]
        );
        let quoted = md("k", "\"Sep 4th, 2026\"", &config);
        assert_eq!(texts(&quoted), vec!["\"Sep 4th, 2026\""]);

        // A comma-free title format classifies straight through.
        let mut iso = ParseConfig::default();
        iso.journal_page_title_format = Some("do MMM yyyy".into());
        let atoms = md("k", "4th Sep 2026", &iso);
        assert_eq!(atoms[0].day, Some(20260904));
    }

    #[test]
    fn atom_key_is_nfc_lowercased_and_trimmed() {
        // U+0065 U+0301 (e + combining acute) composes to U+00E9.
        assert_eq!(atom_key("  E\u{0301}TAT "), "\u{e9}tat");
        assert_eq!(atom_key("Done"), "done");
    }

    #[test]
    fn org_and_markdown_read_the_same_ref_through_their_own_grammar() {
        let config = config();
        let markdown = property_atoms("k", "[[a]]", AtomFormat::Markdown, &config);
        let org = property_atoms("k", "[[a]]", AtomFormat::Org, &config);
        assert_eq!(texts(&markdown), vec!["a"]);
        assert_eq!(texts(&org), vec!["a"]);
    }
}
