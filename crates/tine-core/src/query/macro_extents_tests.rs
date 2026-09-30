//! Shared fixtures for the native macro reader and its compiled wasm entry.
//! `queryMacro.guard.test.ts` pins both hosts to macro_extent.rs (I-12).
//! Rust exposes bytes; the frontend converts to UTF-16. Both boundaries must
//! recover the same text, name and verbatim argument, including parser hazards.

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

/// I-22 (og C3 L02): multibyte text where the scanner probes a byte offset
/// is ordinary block text. It is not a query macro and must not panic.
#[test]
fn multibyte_text_at_a_probed_offset_is_not_a_panic() {
    for raw in [
        "{{中文}}",
        "{{ 名前 }}",
        "{{ščř",
        "{{query a}é}",
        "{{中文}} {{tine-quer",
    ] {
        assert!(query_macro_extents(raw).is_empty(), "{raw}");
    }
    let found = query_macro_extents("{{中文}} {{query (task TODO)}}");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].argument, "(task TODO)");
}
