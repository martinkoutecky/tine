//! **The one fixture set both raw-macro readers must agree on** (SPEC §4.3.1, §7.9).
//!
//! P0-ts deleted the frontend's query parser, its options-map splitter and its
//! advanced-vs-OG discriminator, because each was a second answer to a question
//! Rust already answers (I-12). The raw EXTENT reader is deliberately not one of
//! them: §4.3.1 keeps it on both sides — *"Extend the existing
//! `queryMacroExtent(s)` boundary helper … The Rust publishing boundary uses the
//! same fixtures and a transcription of this helper"* — because rendering,
//! in-place macro rewriting and Export collection are synchronous walks over
//! blocks already in memory, and an async IPC round-trip per macro is not
//! available to them.
//!
//! What makes that pair legitimate rather than a twin is THIS FILE: one fixture
//! set, read by `src/editor/queryMacro.test.ts` and by the test below, asserting
//! the same recovered `{text, name, argument}` for every case. If the two readers
//! ever disagree, one of these two tests goes red.
//!
//! Offsets are deliberately NOT compared: Rust reports byte offsets and
//! JavaScript UTF-16 code units, so the integers cannot agree on non-ASCII input
//! while the recovered TEXT still does. The text is the contract.

use crate::query::macro_text::query_macro_extents;

#[derive(serde::Deserialize)]
struct Case {
    /// Why this case is in the corpus — read it before deleting a row.
    #[allow(dead_code)]
    why: String,
    raw: String,
    extents: Vec<Expected>,
}

#[derive(serde::Deserialize)]
struct Expected {
    /// The exact source slice `[start, end)` must select.
    text: String,
    name: String,
    argument: String,
}

fn cases() -> Vec<Case> {
    // Compiled in: tine-core source does no file I/O (tests/no_file_io.rs).
    serde_json::from_str(include_str!("fixtures/query-macro/extents.json"))
        .expect("query-macro/extents.json is valid JSON")
}

#[test]
fn the_raw_extent_reader_matches_the_shared_fixtures() {
    for case in cases() {
        let found = query_macro_extents(&case.raw);
        assert_eq!(
            found.len(),
            case.extents.len(),
            "extent COUNT for {:?} ({})",
            case.raw,
            case.why
        );
        for (found, expected) in found.iter().zip(&case.extents) {
            assert_eq!(
                &case.raw[found.start..found.end],
                expected.text,
                "extent SLICE for {:?} ({})",
                case.raw,
                case.why
            );
            assert_eq!(found.name, expected.name, "macro NAME for {:?}", case.raw);
            assert_eq!(
                found.argument, expected.argument,
                "raw ARGUMENT for {:?} ({}) — this is the byte contract the AST cannot meet",
                case.raw, case.why
            );
        }
    }
}

/// The corpus is only worth anything if it exercises both macro names and the
/// hazards §4.3.1 names. A future edit that quietly trimmed it to the easy cases
/// would leave both readers unpinned exactly where they diverge.
#[test]
fn the_shared_corpus_covers_both_names_and_the_named_hazards() {
    let cases = cases();
    let all: String = cases.iter().map(|c| c.raw.as_str()).collect();
    assert!(all.contains("{{tine-query"), "no TQL macro in the corpus");
    assert!(
        all.contains("{{query-foo"),
        "no lookalike-name rejection case"
    );
    assert!(all.contains("'a,b'"), "no literal-comma case (§4.3.1)");
    assert!(all.contains("[[A }} B]]"), "no page-ref-opacity case");
    assert!(all.contains(";"), "no options-map comment case");
    assert!(
        cases.iter().any(|c| c.extents.is_empty()),
        "no case that must find NOTHING"
    );
    assert!(
        cases.iter().any(|c| c.extents.len() > 1),
        "no multi-macro case (X2)"
    );
}
