//! Canonical, source-addressed page-reference evidence.
//!
//! lsdoc decides which syntax is a reference and which source ranges are plain
//! visible text.  This module maps those parser spans back from Tine's
//! re-bulleted parse input to `DocBlock::raw`; query surfaces then select a
//! canonical page/alias set without reparsing or inventing another matcher.

use crate::model::{ReferenceKind, ReferenceOccurrence, ReferenceSpan};
use crate::refs;
use lsdoc::ast::Block;
use std::ops::Range;
use std::sync::OnceLock;

mod plain_match;
mod signature;
// The walker also compiles into lsdoc-wasm; it reads these through `super::`.
mod walk;
use crate::block_regions;
use crate::doc::property_key_norm;
use crate::query::atom::{reference_parsing_suppressed, separated_by_commas, ParseConfig};
use crate::query::macro_text::is_query_macro_name;
use crate::render::parse_inline_bounded;
pub use signature::{BlockSignature, ReferenceFilter};
#[cfg(test)]
use walk::property_key_eligible;
use walk::{project_into, quoted_linkable_value};
pub use walk::{
    project_with_config, reference_rule, ProjectedPageRef, ReferenceSourceProjection, RewriteTarget,
};

const MAX_OCCURRENCES_PER_BLOCK: usize = 64;

// This small module also compiles into lsdoc-wasm; keep one built-in answer.
mod og_built_in;
pub use og_built_in::{hidden_from_all_pages, is_og_built_in_page, is_og_built_in_property};
#[cfg(test)]
use og_built_in::{OG_EDITABLE_BUILT_IN_PROPERTIES, OG_HIDDEN_BUILT_IN_PROPERTIES};

/// Borrowed view of the reference spans retained with a block projection.
/// The stored form is split (sparse vectors live behind the projection's
/// optional box); every reader works through this one view.
#[deny(missing_docs)]
#[derive(Debug, Clone, Copy)]
pub struct ReferenceSource<'a> {
    /// Explicit page references recognized by the parser.
    pub explicit: &'a [ProjectedPageRef],
    /// Byte ranges in the raw source eligible for plain-text matching.
    pub plain_ranges: &'a [Range<usize>],
    /// Structural source ranges excluded from plain-text matching.
    pub withheld_ranges: &'a [Range<usize>],
}

/// The rewrite targets of one block source (see [`RewriteTarget`]), found by
/// the same walk as its references, over a fresh lsdoc parse. O(raw bytes)
/// plus one inline parse per macro argument.
pub fn rename_targets(raw: &str, is_org: bool) -> Vec<RewriteTarget> {
    let blocks = crate::render::parse_projection(raw, is_org).blocks;
    let regions = crate::block_regions::from_blocks(raw, is_org, &blocks);
    let mut projection = ReferenceSourceProjection {
        rewrite_targets: Some(Vec::new()),
        ..Default::default()
    };
    project_into(
        &mut projection,
        raw,
        is_org,
        &blocks,
        &regions,
        crate::query::atom::ParseConfig::default_ref(),
    );
    projection.rewrite_targets.unwrap_or_default()
}

impl ReferenceSourceProjection {
    /// Borrow this build result as a [`ReferenceSource`].
    #[cfg(any(test, feature = "test-faults"))]
    pub fn as_source(&self) -> ReferenceSource<'_> {
        ReferenceSource {
            explicit: &self.explicit,
            plain_ranges: &self.plain_ranges,
            withheld_ranges: &self.withheld_ranges,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BoundedOccurrences {
    pub occurrences: Vec<ReferenceOccurrence>,
    pub total: usize,
    pub truncated: bool,
}

/// Page candidates in accepted tags/alias/aliases values. Quoted values stay
/// literal; explicit and implicit references share the existing evidence.
/// O(properties + references), zero parses or allocation; inputs must be the
/// memoized regions and evidence of the same raw block.
pub fn linkable_property_names<'a>(
    projection: ReferenceSource<'a>,
    regions: &'a crate::block_regions::BlockRegions,
) -> impl Iterator<Item = &'a str> {
    linkable_property_references(projection, regions).map(|reference| reference.name.as_str())
}

/// Parser-owned references inside applicable tags/alias/aliases values,
/// including explicit native wrappers. Quoted values stay literal.
/// O(properties + references), zero parses or allocation.
pub fn linkable_property_references<'a>(
    projection: ReferenceSource<'a>,
    regions: &'a crate::block_regions::BlockRegions,
) -> impl Iterator<Item = &'a ProjectedPageRef> {
    let mut properties = regions
        .properties
        .iter()
        .filter(|p| {
            p.applicable
                && (p.key.eq_ignore_ascii_case("tags")
                    || p.key.eq_ignore_ascii_case("alias")
                    || p.key.eq_ignore_ascii_case("aliases"))
                && !quoted_linkable_value(&p.value)
        })
        .peekable();
    projection.explicit.iter().filter_map(move |reference| {
        while properties
            .peek()
            .is_some_and(|p| p.value_range.1 <= reference.range.start)
        {
            properties.next();
        }
        properties
            .peek()
            .filter(|p| {
                p.value_range.0 <= reference.range.start && reference.range.end <= p.value_range.1
            })
            .map(|_| reference)
    })
}

/// Project parser-claimed references and the remaining plain-search spans for
/// one block source. Work and allocation are bounded by that source's spans;
/// malformed ranges are omitted, and no graph state or files are touched.
pub fn project(raw: &str, is_org: bool, blocks: &[Block]) -> ReferenceSourceProjection {
    project_with_config(
        raw,
        is_org,
        blocks,
        &crate::block_regions::from_blocks(raw, is_org, blocks),
        crate::query::atom::ParseConfig::default_ref(),
    )
}

fn byte_to_utf16(raw: &str, byte: usize) -> usize {
    utf16_len(raw.get(..byte).unwrap_or(raw))
}

/// UTF-16 length of `text`: one unit per scalar value, two for those outside
/// the BMP (4 UTF-8 bytes). Counting lead bytes vectorizes where
/// `encode_utf16().count()` decodes every character (GH #623: this runs twice
/// per occurrence, over the block prefix).
fn utf16_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    let scalars = bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count();
    let astral = bytes.iter().filter(|&&b| b >= 0xF0).count();
    scalars + astral
}

fn is_og_edge_alphanumeric(ch: Option<char>) -> bool {
    ch.is_some_and(|ch| ch.is_ascii_alphanumeric())
}

fn og_prefix_allows(raw: &str, start: usize) -> bool {
    let mut preceding = raw.get(..start).unwrap_or_default().chars().rev();
    match preceding.next() {
        None => true,
        Some('[') => preceding.next() != Some('['),
        Some('#') => false,
        Some(_) => true,
    }
}

fn overlaps(range: &Range<usize>, other: &Range<usize>) -> bool {
    range.start < other.end && other.start < range.end
}

fn push_unique_bounded(
    out: &mut Vec<ReferenceOccurrence>,
    matched_name: &str,
    canonical: &str,
    kind: ReferenceKind,
    span: ReferenceSpan,
    rule: &str,
) -> bool {
    if out.iter().any(|existing| {
        existing.span == span
            && existing.kind == kind
            && refs::same_page(&existing.matched_name, matched_name)
    }) {
        return true;
    }
    if out.len() >= MAX_OCCURRENCES_PER_BLOCK {
        return false;
    }
    #[cfg(any(test, feature = "test-counters"))]
    OCCURRENCE_CONSTRUCTIONS.with(|count| count.set(count.get().saturating_add(1)));
    out.push(ReferenceOccurrence {
        matched_name: matched_name.to_string(),
        canonical: canonical.to_string(),
        kind,
        span,
        rule: rule.to_string(),
    });
    true
}

#[cfg(any(test, feature = "test-counters"))]
thread_local! {
    static OCCURRENCE_CONSTRUCTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-counters"))]
pub fn reset_occurrence_constructions() {
    OCCURRENCE_CONSTRUCTIONS.with(|count| count.set(0));
}

#[cfg(any(test, feature = "test-counters"))]
pub fn occurrence_constructions() -> usize {
    OCCURRENCE_CONSTRUCTIONS.with(std::cell::Cell::get)
}

/// What an explicit reference reaches: page keys (its own and its namespace
/// prefixes), or, for an enabled property-key page, every page one of whose
/// names folds to the same property key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceTarget<'a> {
    /// A normalized page key (`refs::page_key`).
    PageKey(&'a str),
    /// A property-key fold (`doc::property_key_norm`) of a page name.
    PropertyKey(&'a str),
}

/// Visit every target `reference` reaches until `visit` returns true; returns
/// whether it did. The ONE rule behind the Linked References panel's match
/// test and the `links` / `linked_by` link table (I-12).
pub fn visit_reference_targets(
    reference: &ProjectedPageRef,
    config: &crate::config::Config,
    mut visit: impl FnMut(ReferenceTarget<'_>) -> bool,
) -> bool {
    if reference.rule == "explicit_property_key" {
        return config.property_page_key_enabled(&reference.name)
            && visit(ReferenceTarget::PropertyKey(
                &crate::doc::property_key_norm(&reference.name),
            ));
    }
    let key = refs::page_key(&reference.name);
    let found = refs::namespace_refs(&key).any(|key| visit(ReferenceTarget::PageKey(key)));
    found
}

fn projected_reference_matches(
    reference: &ProjectedPageRef,
    names_norm: &[String],
    config: &crate::config::Config,
) -> bool {
    visit_reference_targets(reference, config, |target| match target {
        ReferenceTarget::PropertyKey(folded) => names_norm
            .iter()
            .any(|name| crate::doc::property_key_norm(name) == folded),
        ReferenceTarget::PageKey(key) => names_norm.iter().any(|name| name == key),
    })
}

/// The page names of one query, with the plain-text matcher's per-name folding
/// done once (lazily: explicit-kind callers never pay for it) instead of once
/// per block (GH #623, I-25: no per-block work that depends only on the query).
pub struct ReferenceNeedles<'a> {
    names: &'a [String],
    needles: OnceLock<Vec<Option<plain_match::Needle>>>,
}

impl<'a> ReferenceNeedles<'a> {
    pub fn new(names: &'a [String]) -> Self {
        Self {
            names,
            needles: OnceLock::new(),
        }
    }

    pub fn names(&self) -> &'a [String] {
        self.names
    }

    fn needles(&self) -> &[Option<plain_match::Needle>] {
        self.needles.get_or_init(|| {
            self.names
                .iter()
                .map(|name| plain_match::Needle::new(name))
                .collect()
        })
    }
}

#[cfg(any(test, feature = "test-faults"))]
pub fn occurrences_of_kind_bounded(
    raw: &str,
    projection: ReferenceSource<'_>,
    canonical: &str,
    names_norm: &[String],
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> BoundedOccurrences {
    occurrences_of_kind_prepared(
        raw,
        projection,
        canonical,
        &ReferenceNeedles::new(names_norm),
        kind,
        config,
    )
}

/// `occurrences_of_kind_bounded` with the query's folded names shared across
/// blocks.
pub fn occurrences_of_kind_prepared(
    raw: &str,
    projection: ReferenceSource<'_>,
    canonical: &str,
    names: &ReferenceNeedles<'_>,
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> BoundedOccurrences {
    let mut out = Vec::new();
    let mut total = 0usize;
    if kind == ReferenceKind::Explicit {
        for reference in projection.explicit {
            if !projected_reference_matches(reference, names.names, config) {
                continue;
            }
            total = total.saturating_add(1);
            if out.len() < MAX_OCCURRENCES_PER_BLOCK {
                let _ = push_unique_bounded(
                    &mut out,
                    reference.name.trim(),
                    canonical,
                    kind,
                    ReferenceSpan {
                        start: byte_to_utf16(raw, reference.range.start),
                        end: byte_to_utf16(raw, reference.range.end),
                    },
                    reference.rule,
                );
            }
        }
        return BoundedOccurrences {
            truncated: total > out.len(),
            occurrences: out,
            total,
        };
    }

    for (name, needle) in names.names.iter().zip(names.needles()) {
        let Some(needle) = needle else {
            continue;
        };
        for eligible in projection.plain_ranges {
            plain_match::visit_plain_matches(raw, eligible, needle, |range| {
                if projection
                    .explicit
                    .iter()
                    .any(|reference| overlaps(&range, &reference.range))
                {
                    return true;
                }
                total = total.saturating_add(1);
                if out.len() < MAX_OCCURRENCES_PER_BLOCK {
                    let _ = push_unique_bounded(
                        &mut out,
                        raw.get(range.clone()).unwrap_or(name),
                        canonical,
                        kind,
                        ReferenceSpan {
                            start: byte_to_utf16(raw, range.start),
                            end: byte_to_utf16(raw, range.end),
                        },
                        "plain_og_boundary",
                    );
                }
                true
            });
        }
    }
    out.sort_by_key(|occurrence| (occurrence.span.start, occurrence.span.end));
    BoundedOccurrences {
        truncated: total > out.len(),
        occurrences: out,
        total,
    }
}

#[cfg(any(test, feature = "test-faults"))]
pub fn occurrences_of_kind(
    raw: &str,
    projection: ReferenceSource<'_>,
    canonical: &str,
    names_norm: &[String],
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> Vec<ReferenceOccurrence> {
    occurrences_of_kind_bounded(raw, projection, canonical, names_norm, kind, config).occurrences
}

/// Cheap membership path used once a result construction budget is closed.
/// It performs no occurrence/string construction and stops at the first hit.
#[cfg(any(test, feature = "test-faults"))]
pub fn has_occurrence_kind(
    raw: &str,
    projection: ReferenceSource<'_>,
    names_norm: &[String],
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> bool {
    has_occurrence_prepared(
        raw,
        projection,
        &ReferenceNeedles::new(names_norm),
        kind,
        config,
    )
}

/// `has_occurrence_kind` with the query's folded names shared across blocks.
pub fn has_occurrence_prepared(
    raw: &str,
    projection: ReferenceSource<'_>,
    names: &ReferenceNeedles<'_>,
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> bool {
    if kind == ReferenceKind::Explicit {
        return projection
            .explicit
            .iter()
            .any(|reference| projected_reference_matches(reference, names.names, config));
    }
    for needle in names.needles().iter().flatten() {
        for eligible in projection.plain_ranges {
            let mut found = false;
            plain_match::visit_plain_matches(raw, eligible, needle, |range| {
                found = !projection
                    .explicit
                    .iter()
                    .any(|reference| overlaps(&range, &reference.range));
                !found
            });
            if found {
                return true;
            }
        }
    }
    false
}

pub fn occurrences(
    raw: &str,
    projection: ReferenceSource<'_>,
    canonical: &str,
    names_norm: &[String],
    config: &crate::config::Config,
) -> Vec<ReferenceOccurrence> {
    let names = ReferenceNeedles::new(names_norm);
    let mut out = occurrences_of_kind_prepared(
        raw,
        projection,
        canonical,
        &names,
        ReferenceKind::Explicit,
        config,
    )
    .occurrences;
    out.extend(
        occurrences_of_kind_prepared(
            raw,
            projection,
            canonical,
            &names,
            ReferenceKind::Plain,
            config,
        )
        .occurrences,
    );
    out.sort_by(|a, b| {
        a.span
            .start
            .cmp(&b.span.start)
            .then_with(|| a.span.end.cmp(&b.span.end))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.matched_name.cmp(&b.matched_name))
    });
    out.truncate(MAX_OCCURRENCES_PER_BLOCK);
    out
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn utf16_len_agrees_with_encode_utf16() {
        for text in [
            "",
            "plain ascii",
            "caf\u{e9} \u{4e2d}\u{6587}",
            "astral \u{1f600}\u{10348} end",
            "e\u{301}\u{1f1e8}\u{1f1ff}",
        ] {
            assert_eq!(utf16_len(text), text.encode_utf16().count(), "{text:?}");
            for (cut, _) in text.char_indices() {
                assert_eq!(byte_to_utf16(text, cut), text[..cut].encode_utf16().count());
            }
        }
        // A cut that is not a char boundary falls back to the whole text.
        assert_eq!(byte_to_utf16("\u{e9}", 1), 1);
    }

    fn evidence(raw: &str, names: &[&str]) -> Vec<ReferenceOccurrence> {
        let parsed = crate::render::parse_projection(raw, false);
        let projected = project(raw, false, &parsed.blocks);
        occurrences(
            raw,
            projected.as_source(),
            "Target",
            &names
                .iter()
                .map(|name| refs::normalize(name))
                .collect::<Vec<_>>(),
            &crate::config::Config::default(),
        )
    }

    #[test]
    fn mixed_explicit_and_plain_occurrences_stay_independent() {
        let raw = "[[Target]] then Target and `Target`";
        let got = evidence(raw, &["Target"]);
        assert_eq!(
            got.iter()
                .filter(|hit| hit.kind == ReferenceKind::Explicit)
                .count(),
            1
        );
        let plains = got
            .iter()
            .filter(|hit| hit.kind == ReferenceKind::Plain)
            .collect::<Vec<_>>();
        assert_eq!(plains.len(), 2);
        assert!(plains
            .iter()
            .all(|hit| &raw[hit.span.start..hit.span.end] == "Target"));
    }

    #[test]
    fn task_reference_belongs_to_the_blocks_own_header() {
        let own = evidence("TODO call Alice", &["TODO"]);
        assert!(own.iter().any(|hit| hit.kind == ReferenceKind::Explicit));
        let body = evidence("ordinary body\n# TODO a secondary heading", &["TODO"]);
        assert!(body.iter().all(|hit| hit.kind != ReferenceKind::Explicit));
    }

    #[test]
    fn priority_reference_belongs_to_the_blocks_own_header() {
        // OG graph-parser `with-page-refs`: `[marker priority]` join the refs.
        for raw in ["[#A] call Alice", "TODO [#A] call Alice", "## [#A] heading"] {
            let own = evidence(raw, &["A"]);
            let hit = own
                .iter()
                .find(|hit| hit.kind == ReferenceKind::Explicit)
                .unwrap_or_else(|| panic!("{raw}: no explicit hit"));
            assert_eq!(hit.rule, "explicit_priority");
            let start = raw.find("[#A]").unwrap();
            assert_eq!((hit.span.start, hit.span.end), (start, start + 4));
        }
        // Page identity folds case: a lowercase `[#a]` (accepted by the parser,
        // like OG's mldoc) names the same page "A".
        assert!(evidence("TODO [#a] x", &["A"])
            .iter()
            .any(|hit| hit.kind == ReferenceKind::Explicit));
        // Not a priority: mid-text, inline code, a nested heading, after the title.
        for raw in [
            "see [#A] later",
            "`[#A] code`",
            "ordinary body\n# [#A] secondary heading",
            "TODO task [#A] later",
        ] {
            let got = evidence(raw, &["A"]);
            assert!(
                got.iter().all(|hit| hit.kind != ReferenceKind::Explicit),
                "{raw}: {got:?}"
            );
        }
    }

    #[test]
    fn unicode_boundaries_and_properties_use_parser_ranges() {
        let got = evidence("note:: Target\nŽTargetX Target", &["Target"]);
        let plains = got
            .iter()
            .filter(|hit| hit.kind == ReferenceKind::Plain)
            .collect::<Vec<_>>();
        assert_eq!(plains.len(), 2);
    }

    #[test]
    fn escaped_link_is_not_plain_but_code_mentions_are() {
        let got = evidence("\\[[Target]] and `Target`\n```\nTarget\n```", &["Target"]);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.iter().all(|hit| hit.kind == ReferenceKind::Plain));
    }

    #[test]
    fn bare_linkable_property_values_project_exact_explicit_evidence() {
        for raw in [
            "tags:: Target",
            "alias:: Target",
            "aliases:: Other, Target, Third",
            "tags:: Other，Target，Third",
        ] {
            let got = evidence(raw, &["Target"]);
            let explicit = got
                .iter()
                .filter(|hit| hit.kind == ReferenceKind::Explicit)
                .collect::<Vec<_>>();
            assert_eq!(explicit.len(), 1, "{raw}: {got:?}");
            assert_eq!(explicit[0].rule, "implicit_linkable_property");
            let start = raw.find("Target").unwrap();
            assert_eq!(explicit[0].span.start, byte_to_utf16(raw, start));
            assert_eq!(explicit[0].span.end, byte_to_utf16(raw, start + 6));
        }
    }

    #[test]
    fn property_key_projection_uses_canonical_span_and_exact_og_built_ins() {
        let raw = "  Done_At:: today";
        let parsed = crate::render::parse_projection(raw, false);
        let projected = project(raw, false, &parsed.blocks);
        let key = projected
            .explicit
            .iter()
            .find(|reference| reference.rule == "explicit_property_key")
            .unwrap();
        assert_eq!(key.name, "done-at");
        assert_eq!(&raw[key.range.clone()], "Done_At");

        for built_in in OG_EDITABLE_BUILT_IN_PROPERTIES
            .iter()
            .chain(OG_HIDDEN_BUILT_IN_PROPERTIES)
        {
            let raw = format!("{built_in}:: value");
            let parsed = crate::render::parse_projection(&raw, false);
            let projected = project(&raw, false, &parsed.blocks);
            assert!(
                projected
                    .explicit
                    .iter()
                    .all(|reference| reference.rule != "explicit_property_key"),
                "built-in key projected: {built_in}: {:?}",
                projected.explicit
            );
        }
    }

    #[test]
    fn og_built_in_predicate_normalizes_and_includes_registered_srs_keys() {
        for key in OG_EDITABLE_BUILT_IN_PROPERTIES
            .iter()
            .chain(OG_HIDDEN_BUILT_IN_PROPERTIES)
        {
            for spelling in [
                key.to_string(),
                format!(" {} ", key.to_uppercase()),
                key.replace('-', "_"),
            ] {
                assert!(is_og_built_in_property(&spelling), "{spelling:?}");
                assert_eq!(
                    is_og_built_in_property(&spelling),
                    !property_key_eligible(&crate::doc::property_key_norm(&spelling)),
                );
            }
        }
        assert!(is_og_built_in_property(" Card Ease_Factor "));
        for key in ["", " ", "owner", "card-ease-factor-custom", "tine.custom"] {
            assert!(!is_og_built_in_property(key), "{key:?}");
        }
    }

    #[test]
    fn property_key_eligibility_matches_normalizing_every_built_in_per_call() {
        // GJ5 S2: the once-normalized built-in set answers exactly what the
        // former per-call normalization of every built-in answered.
        let reference = |key: &str| {
            let key = crate::doc::property_key_norm(key);
            !key.is_empty()
                && !OG_EDITABLE_BUILT_IN_PROPERTIES
                    .iter()
                    .chain(OG_HIDDEN_BUILT_IN_PROPERTIES)
                    .any(|built_in| crate::doc::property_key_norm(built_in) == key)
        };
        let mut keys: Vec<String> = [
            "",
            " ",
            "status",
            "Status",
            "done_at",
            "TITLE",
            " Template_Including_Parent ",
            "background color",
            "Background_Color",
            "logseq.table.version",
            "card_last_score",
            "custom",
            "ÜBER",
            "a b_c",
            "id",
            "ID",
            "created_at",
            "heading ",
        ]
        .iter()
        .map(|key| key.to_string())
        .collect();
        for built_in in OG_EDITABLE_BUILT_IN_PROPERTIES
            .iter()
            .chain(OG_HIDDEN_BUILT_IN_PROPERTIES)
        {
            keys.push(built_in.to_string());
            keys.push(built_in.to_uppercase());
            keys.push(built_in.replace('-', "_"));
            keys.push(format!("{built_in}x"));
        }
        for key in keys {
            assert_eq!(
                property_key_eligible(&crate::doc::property_key_norm(&key)),
                reference(&key),
                "{key:?}"
            );
        }
    }

    #[test]
    fn explicit_property_syntax_is_not_duplicated_or_promoted_from_custom_values() {
        let raw = "tags:: [[Target]]\nalias:: #Target\ncustom:: Target\naliases:: \"Target\"";
        let parsed = crate::render::parse_projection(raw, false);
        let projected = project(raw, false, &parsed.blocks);
        let target = projected
            .explicit
            .iter()
            .filter(|reference| refs::same_page(&reference.name, "Target"))
            .collect::<Vec<_>>();
        assert_eq!(target.len(), 2, "{target:?}");
        assert!(target
            .iter()
            .all(|reference| reference.rule != "implicit_linkable_property"));
    }

    #[test]
    fn occurrence_construction_is_capped_while_scanning_many_matches() {
        let raw = "Target ".repeat(50_000);
        let parsed = crate::render::parse_projection(&raw, false);
        let projected = project(&raw, false, &parsed.blocks);
        reset_occurrence_constructions();
        let got = occurrences_of_kind(
            &raw,
            projected.as_source(),
            "Target",
            &[refs::normalize("Target")],
            ReferenceKind::Plain,
            &crate::config::Config::default(),
        );
        assert_eq!(got.len(), MAX_OCCURRENCES_PER_BLOCK);
        assert_eq!(occurrence_constructions(), MAX_OCCURRENCES_PER_BLOCK);
        assert!(got.capacity() <= MAX_OCCURRENCES_PER_BLOCK);
    }

    #[test]
    fn occurrence_cap_reports_total_and_truncation() {
        let raw = "Target ".repeat(70);
        let parsed = crate::render::parse_projection(&raw, false);
        let projected = project(&raw, false, &parsed.blocks);
        let got = occurrences_of_kind_bounded(
            &raw,
            projected.as_source(),
            "Target",
            &[refs::normalize("Target")],
            ReferenceKind::Plain,
            &crate::config::Config::default(),
        );
        assert_eq!(got.occurrences.len(), MAX_OCCURRENCES_PER_BLOCK);
        assert_eq!(got.total, 70);
        assert!(got.truncated);
    }

    #[test]
    fn structural_id_property_is_not_plain_reference_text() {
        let got = evidence("id:: 6a55b643-1234-5678-9abc-def012345678", &["6a55b643"]);
        assert!(
            got.is_empty(),
            "structural id leaked into evidence: {got:?}"
        );
    }

    #[test]
    fn block_reference_uuid_is_not_a_plain_page_mention() {
        let got = evidence(
            "((11111111-1111-4111-8111-111111111111))",
            &["11111111-1111-4111-8111-111111111111"],
        );
        assert!(
            got.is_empty(),
            "block link leaked into page evidence: {got:?}"
        );
    }

    #[test]
    fn unlinked_mentions_include_literal_regions_without_counting_link_syntax() {
        for raw in [
            "`Target`",
            "```\nTarget\n```",
            "$$\nTarget\n$$",
            "<div>Target</div>",
        ] {
            let got = evidence(raw, &["Target"]);
            assert_eq!(
                got.iter()
                    .filter(|hit| hit.kind == ReferenceKind::Plain)
                    .count(),
                1,
                "{raw}: {got:?}"
            );
        }
        for raw in ["```\n[[Target]]\n```", "`#Target`", "\\[[Target]]"] {
            assert!(evidence(raw, &["Target"]).is_empty(), "{raw}");
        }
    }

    #[test]
    fn plain_boundaries_match_logseq_ascii_edge_rules() {
        let got = evidence("北京Target北京 foo_Target_bar aTargetz", &["Target"]);
        let plains = got
            .iter()
            .filter(|hit| hit.kind == ReferenceKind::Plain)
            .collect::<Vec<_>>();
        assert_eq!(plains.len(), 2);
        assert!(plains.iter().all(|hit| hit.matched_name == "Target"));
    }
}
