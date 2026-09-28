//! **The one fixture set both visible-column resolvers must agree on** (P5A).
//!
//! `tine.columns::` owns the ordered list of columns a query block SHOWS;
//! `tine.fields::` keeps the typed sheet schema. Three consumers read that
//! precedence — the §4.1 property merge (`query::view`), the static publisher
//! (`publish.rs`), and the app's query table — and two of them are in Rust
//! while the third is in TypeScript. Rust has exactly one implementation
//! (`query::view::resolve_query_columns`, which `publish.rs` calls rather than
//! copying); TypeScript has an adapter at the existing cross-language boundary
//! (`src/editor/queryViewProperties.ts`), because rendering a table is a
//! synchronous walk over blocks already in memory and cannot take an IPC
//! round-trip per block.
//!
//! What makes that pair legitimate rather than a twin is THIS FILE: one fixture
//! set, read by `src/editor/queryViewProperties.test.ts` and by the test below.
//! If the two readers ever disagree about which columns a note selects, one of
//! these two tests goes red — and a published page silently showing different
//! columns from the app is exactly the failure the pinning exists to prevent.

use crate::query::view::{resolve_query_columns, QueryColumns};

#[derive(serde::Deserialize)]
struct Case {
    /// Why this case is in the corpus — read it before deleting a row.
    #[allow(dead_code)]
    why: String,
    /// Block properties in document order, keys unnormalized.
    properties: Vec<(String, String)>,
    resolution: Expected,
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Expected {
    Named { columns: Vec<String> },
    Cleared,
    Unset,
}

fn cases() -> Vec<Case> {
    // Compiled in: tine-core source does no file I/O (tests/no_file_io.rs).
    serde_json::from_str(include_str!("fixtures/query-columns/resolution.json"))
        .expect("query-columns/resolution.json is valid JSON")
}

#[test]
fn the_rust_column_resolver_matches_the_shared_fixtures() {
    for case in cases() {
        let found = resolve_query_columns(&case.properties);
        let actual = match &found {
            QueryColumns::Named(columns) => Expected::Named {
                columns: columns.iter().map(|f| f.as_str().to_string()).collect(),
            },
            QueryColumns::Cleared => Expected::Cleared,
            QueryColumns::Unset => Expected::Unset,
        };
        match (&actual, &case.resolution) {
            (Expected::Named { columns: got }, Expected::Named { columns: want }) => {
                assert_eq!(
                    got, want,
                    "columns for {:?} ({})",
                    case.properties, case.why
                );
            }
            (Expected::Cleared, Expected::Cleared) | (Expected::Unset, Expected::Unset) => {}
            _ => panic!(
                "resolution KIND for {:?} ({}): got {:?}",
                case.properties, case.why, found
            ),
        }
    }
}

/// The corpus is only worth anything if it exercises the branches the packet
/// froze. A future edit that quietly trimmed it to the easy cases would leave
/// the two readers free to drift on exactly the inputs that matter.
#[test]
fn the_corpus_covers_every_frozen_branch() {
    let cases = cases();
    let kind = |c: &Case| match c.resolution {
        Expected::Named { .. } => "named",
        Expected::Cleared => "cleared",
        Expected::Unset => "unset",
    };
    for wanted in ["named", "cleared", "unset"] {
        assert!(
            cases.iter().any(|c| kind(c) == wanted),
            "the corpus lost every `{wanted}` case"
        );
    }
    let value_for = |key: &str, case: &Case| {
        case.properties
            .iter()
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
    };
    assert!(
        cases
            .iter()
            .any(|c| value_for("tine.columns", c).is_some_and(|v| v.contains('='))),
        "no case proves that `=` invalidates a whole tine.columns list"
    );
    assert!(
        cases.iter().any(
            |c| value_for("tine.columns", c).is_some() && value_for("tine.fields", c).is_some()
        ),
        "no case proves the precedence between the new key and the legacy one"
    );
    for forbidden in ['\0', '\r', '\n'] {
        assert!(
            cases
                .iter()
                .any(|c| value_for("tine.columns", c).is_some_and(|v| v.contains(forbidden))),
            "no case proves that {forbidden:?} invalidates a tine.columns list"
        );
    }
}

/// **Nothing behind an absent `tine.columns` can resurrect a cleared list.**
///
/// The one writer removes the key rather than emptying it, so a cleared
/// selection resolves as `Unset` rather than `Cleared` — and those two differ in
/// exactly one thing: `Unset` leaves `ViewSettings::columns` as the query TEXT
/// left it. That is only a difference if the text can carry columns.
///
/// It cannot. No dialect parses a column set and no printer emits one
/// (`query::print::og_view`), so every reading arrives with an empty list and
/// the two states draw the same table. This pins the fact rather than the
/// comment: if a future dialect ever lifts columns out of the text, the
/// writer's removal becomes a resurrection and this test says so first.
#[test]
fn no_dialect_lifts_a_column_set_out_of_the_query_text() {
    let registry = crate::query::registry::Registry::none();
    let today = crate::date::JournalDate::today();
    let cases: [(crate::query::QueryInput, &str); 5] = [
        (
            crate::query::QueryInput::Og,
            "(and (todo TODO) (sort-by cost desc) (sample 5) (group-by status) (aggregate sum cost))",
        ),
        (
            crate::query::QueryInput::Advanced,
            "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]]",
        ),
        (
            crate::query::QueryInput::Tql,
            "from blocks where todo = TODO order by cost desc limit 5",
        ),
        (
            crate::query::QueryInput::MacroTql,
            "from blocks where todo = TODO",
        ),
        (
            crate::query::QueryInput::MacroQuery,
            "(and (todo TODO) (sort-by cost desc) (group-by status)) {:title \"T\"}",
        ),
    ];
    for (input, text) in cases {
        let (_, view) = crate::query::parse_query_input(text, input, today, &registry);
        assert!(
            view.columns.is_empty(),
            "{input:?} lifted columns out of {text:?}: {:?}",
            view.columns
        );
    }
}
