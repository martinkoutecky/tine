//! **The one fixture set both grouping resolvers must agree on** (P5B).
//!
//! `tine.group-by::` carried two different meanings on one bare token — the
//! task marker to the board, an ordinary property to the list grouper — so
//! grouping moved to the query-owned `tine.group-field::`, whose value is a
//! canonical sheet `FieldId`. Three consumers read that precedence: the §4.1
//! property merge (`query::view`), the query-backed static publisher
//! (`publish.rs`, which CALLS the same resolver rather than copying it), and
//! the app, whose renderers are a synchronous walk over blocks already in
//! memory and cannot take an IPC round-trip per block.
//!
//! What makes the Rust/TypeScript pair legitimate rather than a twin is THIS
//! FILE: one fixture set, read by the test below and by
//! `src/editor/queryViewProperties.test.ts`. If the two ever disagree about
//! which field a note groups by, one of the two tests goes red — and a
//! published board silently grouping differently from the app is exactly the
//! failure the pinning exists to prevent.

use crate::query::ir::{Field, ViewSettings};
use crate::query::view::{resolve_query_grouping, QueryGrouping};

#[derive(serde::Deserialize)]
struct Case {
    /// Why this case is in the corpus — read it before deleting a row.
    #[allow(dead_code)]
    why: String,
    /// Block properties in document order, keys unnormalized.
    properties: Vec<(String, String)>,
    /// What the query TEXT carried, if anything.
    #[serde(default)]
    parsed: Parsed,
    resolution: Expected,
}

#[derive(serde::Deserialize, Default)]
struct Parsed {
    #[serde(default)]
    group_by: Option<String>,
    #[serde(default)]
    view: Option<String>,
}

impl Parsed {
    fn settings(&self) -> ViewSettings {
        let mut view = ViewSettings::default();
        view.group_by = self.group_by.as_deref().map(Field::new);
        view.view = self.view.as_deref().and_then(|kind| {
            use crate::query::ir::ViewKind;
            match kind {
                "search" => Some(ViewKind::Search),
                "list" => Some(ViewKind::List),
                "table" => Some(ViewKind::Table),
                "board" => Some(ViewKind::Board),
                other => panic!("unknown parsed view {other:?} in the grouping corpus"),
            }
        });
        view
    }
}

#[derive(serde::Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Expected {
    Field { field: String },
    Cleared,
    Unset,
}

fn cases() -> Vec<Case> {
    // Compiled in: tine-core source does no file I/O (tests/no_file_io.rs).
    serde_json::from_str(include_str!("fixtures/query-grouping/resolution.json"))
        .expect("query-grouping/resolution.json is valid JSON")
}

#[test]
fn the_rust_grouping_resolver_matches_the_shared_fixtures() {
    for case in cases() {
        let found = resolve_query_grouping(&case.properties, &case.parsed.settings());
        let actual = match &found {
            QueryGrouping::Field(field) => Expected::Field {
                field: field.as_str().to_string(),
            },
            QueryGrouping::Cleared => Expected::Cleared,
            QueryGrouping::Unset => Expected::Unset,
        };
        assert_eq!(
            actual, case.resolution,
            "grouping for {:?} ({})",
            case.properties, case.why
        );
    }
}

/// The corpus is only worth anything if it exercises the branches the packet
/// froze. A future edit that quietly trimmed it to the easy cases would leave
/// the two readers free to drift on exactly the inputs that matter.
#[test]
fn the_grouping_corpus_covers_every_frozen_branch() {
    let cases = cases();
    let value_for = |key: &str, case: &Case| {
        case.properties
            .iter()
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
    };
    for wanted in [Expected::Cleared, Expected::Unset] {
        assert!(
            cases.iter().any(|c| c.resolution == wanted),
            "the corpus lost every {wanted:?} case"
        );
    }
    // The identity pair the whole key exists for.
    for (spelling, expected) in [("state", "state"), ("prop:state", "prop:state")] {
        assert!(
            cases.iter().any(
                |c| value_for("tine.group-field", c).as_deref() == Some(spelling)
                    && c.resolution
                        == Expected::Field {
                            field: expected.into()
                        }
            ),
            "no case pins `tine.group-field:: {spelling}`"
        );
    }
    // Both legacy readings of the SAME bare token, at the two view families.
    assert!(
        cases.iter().any(
            |c| value_for("tine.group-by", c).as_deref() == Some("state")
                && value_for("tine.view", c).as_deref() == Some("board")
                && c.resolution
                    == Expected::Field {
                        field: "state".into()
                    }
        ),
        "no case proves a legacy board `state` stays the task marker"
    );
    assert!(
        cases.iter().any(
            |c| value_for("tine.group-by", c).as_deref() == Some("state")
                && value_for("tine.view", c).is_none()
                && c.resolution
                    == Expected::Field {
                        field: "prop:state".into()
                    }
        ),
        "no case proves a legacy list `state` stays the ordinary property"
    );
    assert!(
        cases.iter().any(
            |c| value_for("tine.group-by", c).as_deref() == Some("status")
                && value_for("tine.view", c).as_deref() == Some("board")
                && c.resolution
                    == Expected::Field {
                        field: "prop:status".into()
                    }
        ),
        "no case proves a legacy board `status` is an ordinary property, not the marker"
    );
    assert!(
        cases.iter().any(
            |c| value_for("tine.group-by", c).as_deref() == Some("prop:state")
                && value_for("tine.view", c).is_none()
                && c.resolution
                    == Expected::Field {
                        field: "prop:prop:state".into()
                    }
        ),
        "no case proves a legacy list literal `prop:` prefix stays part of the key"
    );
    assert!(
        cases.iter().any(|c| c.parsed.group_by.is_some()
            && value_for("tine.group-by", c).is_none()
            && value_for("tine.group-field", c).is_none()),
        "no case reaches the DSL directive"
    );
    for forbidden in ['\0', '\r', '\n'] {
        assert!(
            cases
                .iter()
                .any(|c| value_for("tine.group-field", c).is_some_and(|v| v.contains(forbidden))),
            "no case proves that {forbidden:?} makes the new key unreadable"
        );
    }
}
