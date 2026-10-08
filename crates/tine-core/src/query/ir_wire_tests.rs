//! The IR's WIRE FORMAT, pinned by golden JSON.
//!
//! SPEC §3.1 fixes the encoding, not just the shapes: internally tagged
//! (`"kind"`), `snake_case` variant names, `snake_case` fields, and `Span`
//! offsets in UTF-16 code units. The TypeScript mirror in
//! `src/editor/queryIr.ts` is written against exactly these bytes, so a
//! rename that Rust happily compiles is a silent break of the frontend — which
//! is why the bytes are checked in rather than re-derived.
//!
//! Each fixture is asserted in both directions: the value serializes to the
//! file, and the file deserializes back to the value.

use crate::model::{BlockDto, PageKind, RefGroup};
use crate::query::ir::{
    AggFn, Anchor, Attr, Bounds, Cardinality, CmpOp, Diagnostic, DiagnosticKind, Field, Filter,
    Leaf, ObservedType, PageRow, Quant, Query, QueryReport, QueryResult, QueryRows, RegistryRow,
    RegistrySnapshot, Rel, SortDir, Source, Span, Value, ViewKind, ViewSettings,
};

/// The checked-in fixture for `name`, compiled in: tine-core source does no
/// file I/O (tests/no_file_io.rs). After a DELIBERATE wire-format change,
/// regenerate the file from the assertion's `encoded` output and move the
/// TypeScript mirror in the same commit.
fn fixture(name: &str) -> &'static str {
    match name {
        "bounds" => include_str!("fixtures/query-ir/bounds.json"),
        "diagnostic" => include_str!("fixtures/query-ir/diagnostic.json"),
        "filter" => include_str!("fixtures/query-ir/filter.json"),
        "leaf" => include_str!("fixtures/query-ir/leaf.json"),
        "query" => include_str!("fixtures/query-ir/query.json"),
        "query_page_anchor" => include_str!("fixtures/query-ir/query_page_anchor.json"),
        "query_result_block" => include_str!("fixtures/query-ir/query_result_block.json"),
        "query_result_page" => include_str!("fixtures/query-ir/query_result_page.json"),
        "registry_snapshot" => include_str!("fixtures/query-ir/registry_snapshot.json"),
        "scoped_display_settings" => include_str!("fixtures/query-ir/scoped_display_settings.json"),
        "source" => include_str!("fixtures/query-ir/source.json"),
        "view_settings" => include_str!("fixtures/query-ir/view_settings.json"),
        other => panic!("no query-ir fixture named {other}"),
    }
}

/// Assert that `value` encodes to `name.json` and decodes back unchanged.
fn golden<T>(name: &str, value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let encoded = serde_json::to_string_pretty(value).expect("serialize") + "\n";
    let expected = fixture(name);
    assert_eq!(
        encoded, expected,
        "the wire format of {name} changed; src/editor/queryIr.ts reads these bytes"
    );
    let decoded: T = serde_json::from_str(expected).expect("deserialize the fixture");
    assert_eq!(
        &decoded, value,
        "{name} does not round-trip through its JSON"
    );
}

/// Assert only the encoding: the type has no `PartialEq` (its rows carry a
/// `RefGroup`, which is compared by the query tests instead).
fn golden_encoding<T: serde::Serialize>(name: &str, value: &T) {
    let encoded = serde_json::to_string_pretty(value).expect("serialize") + "\n";
    assert_eq!(encoded, fixture(name), "the wire format of {name} changed");
}

fn every_filter_variant() -> Filter {
    Filter::and(vec![
        Filter::True,
        Filter::False,
        Filter::not(Filter::page_ref("alpha")),
        Filter::off(Filter::attr(
            Attr::Task,
            CmpOp::In,
            Value::List {
                items: vec![Value::text("TODO"), Value::text("DOING")],
            },
        )),
        Filter::or(vec![
            Filter::attr(Attr::Content, CmpOp::Like, Value::text("%foo%")),
            Filter::attr(Attr::Content, CmpOp::Match, Value::text("foo")),
            Filter::attr(Attr::Content, CmpOp::Regex, Value::text("^foo")),
        ]),
        Filter::attr(Attr::Priority, CmpOp::IsSet, Value::None),
        Filter::attr(Attr::Scheduled, CmpOp::IsNotSet, Value::None),
        Filter::attr(
            Attr::Deadline,
            CmpOp::Between,
            Value::List {
                items: vec![Value::date("today"), Value::date("+7d")],
            },
        ),
        Filter::rel(
            Rel::Page,
            Quant::Any,
            Filter::and(vec![
                Filter::attr(Attr::Name, CmpOp::StartsWith, Value::text("proj/")),
                Filter::attr(Attr::Namespace, CmpOp::NotEq, Value::text("archive")),
                Filter::attr(Attr::Journal, CmpOp::Eq, Value::Bool { value: true }),
                Filter::attr(Attr::UsedAsTag, CmpOp::Eq, Value::Bool { value: true }),
                Filter::attr(Attr::Day, CmpOp::Ge, Value::date("2026-01-01")),
            ]),
        ),
        Filter::rel(
            Rel::Tags,
            Quant::None,
            Filter::attr(Attr::Name, CmpOp::NotIn, Value::List { items: vec![] }),
        ),
        Filter::rel(
            Rel::Props,
            Quant::Every,
            Filter::and(vec![
                Filter::attr(Attr::Key, CmpOp::Eq, Value::text("size")),
                Filter::attr(Attr::Value, CmpOp::Gt, Value::Number { number: 3.0 }),
                // `IsBlank` — a property atom that IS set but empty. Added by
                // P0-ts: the TypeScript mirror's coverage test
                // (`src/editor/queryIr.test.ts`) is only as strong as this
                // corpus, and it found that the one function called
                // `every_filter_variant` was missing an operator.
                Filter::attr(Attr::Value, CmpOp::IsBlank, Value::None),
                Filter::attr(Attr::AtomCount, CmpOp::Lt, Value::Number { number: 5.0 }),
            ]),
        ),
        Filter::rel(Rel::Parent, Quant::Any, Filter::True),
        Filter::rel(Rel::Ancestors, Quant::None, Filter::False),
        Filter::rel(Rel::Descendants, Quant::Every, Filter::True),
        Filter::rel(
            Rel::Children,
            Quant::Any,
            Filter::attr(Attr::Task, CmpOp::Le, Value::text("z")),
        ),
        Filter::rel(
            Rel::Blocks,
            Quant::Every,
            Filter::attr(Attr::Content, CmpOp::Eq, Value::text("x")),
        ),
        Filter::attr(
            Attr::CreatedAt,
            CmpOp::Between,
            Value::List {
                items: vec![Value::date("-7d"), Value::date("now")],
            },
        ),
        Filter::attr(
            Attr::LastModifiedAt,
            CmpOp::Between,
            Value::List {
                items: vec![Value::date("-7d"), Value::date("now")],
            },
        ),
        Filter::Raw {
            text: "(frobnicate x)".to_string(),
            kind: DiagnosticKind::UnknownHead,
            span: Some(Span { start: 16, end: 30 }),
        },
    ])
}

#[test]
fn the_filter_wire_format_covers_every_variant() {
    golden("filter", &every_filter_variant());
}

/// The corpus must EARN its name. `every_filter_variant` is what pins both the
/// wire bytes and the TypeScript mirror's completeness, so an operator it never
/// spells is an operator neither side is checked on — which is how `IsBlank` sat
/// unpinned until P0-ts's mirror test noticed. Enumerating here rather than
/// trusting the name means the next added operator fails this test on the spot.
#[test]
fn the_corpus_spells_every_comparison_operator() {
    use std::collections::BTreeSet;
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    every_filter_variant().for_each_leaf(&mut |leaf| {
        if let Leaf::Attr { op, .. } = leaf {
            seen.insert(match op {
                CmpOp::Eq => "eq",
                CmpOp::NotEq => "not_eq",
                CmpOp::Lt => "lt",
                CmpOp::Le => "le",
                CmpOp::Gt => "gt",
                CmpOp::Ge => "ge",
                CmpOp::Between => "between",
                CmpOp::In => "in",
                CmpOp::NotIn => "not_in",
                CmpOp::Like => "like",
                CmpOp::StartsWith => "starts_with",
                CmpOp::Match => "match",
                CmpOp::Regex => "regex",
                CmpOp::IsSet => "is_set",
                CmpOp::IsNotSet => "is_not_set",
                CmpOp::IsBlank => "is_blank",
            });
        }
    });
    let expected: BTreeSet<&'static str> = [
        "eq",
        "not_eq",
        "lt",
        "le",
        "gt",
        "ge",
        "between",
        "in",
        "not_in",
        "like",
        "starts_with",
        "match",
        "regex",
        "is_set",
        "is_not_set",
        "is_blank",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        seen, expected,
        "every_filter_variant() does not spell every CmpOp; add the missing one \
         so both the wire fixture and the TypeScript mirror are pinned on it"
    );
}

#[test]
fn the_leaf_wire_format_is_internally_tagged() {
    golden(
        "leaf",
        &vec![
            Leaf::Attr {
                attr: Attr::Content,
                op: CmpOp::Like,
                value: Value::text("%foo%"),
            },
            Leaf::Rel {
                rel: Rel::Refs,
                quant: Quant::Any,
                pred: Box::new(Filter::attr(Attr::Name, CmpOp::Eq, Value::text("alpha"))),
            },
        ],
    );
}

#[test]
fn the_source_wire_format_names_all_four_origins() {
    golden(
        "source",
        &vec![
            Source::Og {
                original: "(task TODO) {:title \"T\"}".to_string(),
                og_options: "{:title \"T\"}".to_string(),
            },
            Source::Tql {
                og_options: String::new(),
                original: "task = 'TODO'".to_string(),
            },
            Source::Advanced {
                og_options: String::new(),
                original: "[:find (pull ?b [*]) :where (task ?b \"TODO\")]".to_string(),
            },
            Source::Builder,
        ],
    );
}

#[test]
fn the_diagnostic_wire_format_names_every_kind() {
    let kinds = [
        DiagnosticKind::UnknownHead,
        DiagnosticKind::Syntax,
        DiagnosticKind::UnknownIdent,
        DiagnosticKind::NotApplicable,
        DiagnosticKind::Depth,
        DiagnosticKind::Size,
    ];
    let diagnostics: Vec<Diagnostic> = kinds
        .iter()
        .enumerate()
        .map(|(index, kind)| Diagnostic {
            span: Some(Span {
                start: index as u32,
                end: index as u32 + 4,
            }),
            message: format!("diagnostic {index}"),
            suggestions: vec!["prop('status')".to_string()],
            disabled: index % 2 == 1,
            kind: *kind,
        })
        .collect();
    golden("diagnostic", &diagnostics);
}

#[test]
fn the_view_settings_wire_format_is_stable() {
    golden(
        "view_settings",
        &ViewSettings {
            view: Some(ViewKind::Table),
            sort: vec![
                (Field::new("status"), SortDir::Asc),
                (Field::new("modified"), SortDir::Desc),
            ],
            group_by: Some(Field::new("page")),
            columns: vec![Field::new("page"), Field::new("status")],
            aggregates: vec![
                (Field::new(""), AggFn::Count),
                (Field::new("size"), AggFn::Sum),
                (Field::new("size"), AggFn::Avg),
            ],
            sample: Some(25),
        },
    );
}

#[test]
fn the_query_wire_format_is_stable() {
    golden(
        "query",
        &Query {
            anchor: Anchor::Block,
            filter: every_filter_variant(),
            diagnostics: vec![Diagnostic {
                span: Some(Span { start: 16, end: 30 }),
                message: "`frobnicate` is not a query filter".to_string(),
                suggestions: vec![],
                disabled: false,
                kind: DiagnosticKind::UnknownHead,
            }],
            source: Source::Og {
                original: "(and (task TODO) (frobnicate x))".to_string(),
                og_options: String::new(),
            },
        },
    );
    golden(
        "query_page_anchor",
        &Query {
            anchor: Anchor::Page,
            filter: Filter::attr(Attr::Journal, CmpOp::Eq, Value::Bool { value: true }),
            diagnostics: vec![],
            source: Source::Tql {
                og_options: String::new(),
                original: "@page and journal = true".to_string(),
            },
        },
    );
}

#[test]
fn the_query_result_wire_format_is_tagged_by_anchor() {
    golden_encoding(
        "query_result_block",
        &QueryResult {
            statistics: Some(statistics_wire_example()),
            rows: QueryRows::Block {
                groups: vec![RefGroup {
                    page: "Home".to_string(),
                    kind: PageKind::Page,
                    blocks: vec![BlockDto {
                        id: "11111111-1111-4111-8111-111111111111".to_string(),
                        raw: "TODO a task".to_string(),
                        ..Default::default()
                    }],
                    evidence: vec![],
                }],
            },
            diagnostics: vec![],
            report: QueryReport {
                ran: vec![],
                ignored: vec![],
                supported: true,
            },
            total: 1,
            matched_total: None,
            exceeded: false,
        },
    );
    golden_encoding(
        "query_result_page",
        &QueryResult {
            statistics: Some(statistics_wire_example()),
            rows: QueryRows::Page {
                pages: vec![
                    PageRow {
                        path: "pages/home.md".to_string(),
                        name: "Home".to_string(),
                        kind: PageKind::Page,
                        journal_day: None,
                        properties: vec![("status".to_string(), "active".to_string())],
                    },
                    PageRow {
                        path: "journals/2026_07_29.md".to_string(),
                        name: "Jul 29th, 2026".to_string(),
                        kind: PageKind::Journal,
                        journal_day: Some(20260729),
                        properties: vec![],
                    },
                ],
            },
            diagnostics: vec![Diagnostic {
                span: None,
                message: "`(all-page-tags)` takes no arguments".to_string(),
                suggestions: vec![],
                disabled: false,
                kind: DiagnosticKind::Syntax,
            }],
            report: QueryReport {
                ran: vec!["page-tags".to_string()],
                ignored: vec!["sample".to_string()],
                supported: true,
            },
            total: 2,
            matched_total: Some(7),
            exceeded: true,
        },
    );
}

fn statistics_wire_example() -> crate::query::ir::QueryStatistics {
    use crate::query::ir::{
        QueryStatistics, QueryStatisticsCell as Cell, QueryStatisticsGroup,
        QueryStatisticsGroupingStatus as Status, QueryStatisticsMarker as Marker,
    };
    let cells = vec![
        Cell::Number {
            value: 3.125,
            skipped: 1,
        },
        Cell::Number {
            value: 0.0,
            skipped: 0,
        },
        Cell::Marker {
            reason: Marker::NonFinite,
            skipped: 0,
        },
        Cell::Marker {
            reason: Marker::DivisionByZero,
            skipped: 0,
        },
        Cell::Marker {
            reason: Marker::EmptyGroup,
            skipped: 0,
        },
        Cell::Marker {
            reason: Marker::NonNumeric,
            skipped: 2,
        },
    ];
    QueryStatistics {
        count: 2,
        aggregates: vec![(Field::new("cost"), AggFn::Sum); 6],
        group_by: Some(Field::new("prop:group")),
        overall: cells.clone(),
        groups: Some(vec![QueryStatisticsGroup {
            key: None,
            count: 2,
            cells,
        }]),
        grouping_status: Status::Exact,
    }
}

#[test]
fn statistics_wire_rejects_non_finite_numbers_and_marker_values() {
    use crate::query::ir::QueryStatisticsCell;
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(serde_json::to_string(&QueryStatisticsCell::Number { value, skipped: 0 }).is_err());
    }
    for value in [
        serde_json::json!({"kind":"number","value":null,"skipped":0}),
        serde_json::json!({"kind":"number","value":"3.125","skipped":0}),
        serde_json::json!({"kind":"marker","reason":"non_finite","value":null,"skipped":0}),
    ] {
        assert!(serde_json::from_value::<QueryStatisticsCell>(value).is_err());
    }
    let absent = serde_json::json!({"anchor":"block","groups":[],"report":{"supported":true},"total":0,"exceeded":false});
    let result: QueryResult = serde_json::from_value(absent).unwrap();
    assert!(result.statistics.is_none());
    assert!(serde_json::to_value(result)
        .unwrap()
        .get("statistics")
        .is_none());
}

#[test]
fn the_registry_snapshot_wire_format_is_stable() {
    golden(
        "registry_snapshot",
        &RegistrySnapshot {
            generation: 7,
            rows: vec![
                RegistryRow {
                    normalized_name: "status".to_string(),
                    cardinality: Cardinality::One,
                    observed_type: ObservedType::Text,
                    count_blocks: 42,
                    count_pages: 3,
                    histogram: vec![(ObservedType::Text, 42), (ObservedType::Number, 0)],
                    mismatch_count: 0,
                    declared: None,
                    top_values: vec![("active".to_string(), 30), ("done".to_string(), 12)],
                },
                RegistryRow {
                    normalized_name: "size".to_string(),
                    cardinality: Cardinality::Many,
                    observed_type: ObservedType::Number,
                    count_blocks: 9,
                    count_pages: 0,
                    histogram: vec![(ObservedType::Number, 8), (ObservedType::Text, 1)],
                    mismatch_count: 1,
                    declared: Some((ObservedType::Number, Cardinality::Many)),
                    top_values: vec![],
                },
            ],
        },
    );
}

#[test]
fn the_bounds_wire_format_is_stable() {
    golden(
        "bounds",
        &Bounds {
            max_rows: 100,
            max_bytes: 1_000_000,
        },
    );
}

/// The mirror is only a mirror if the Rust side says where it is. A reader who
/// changes `Query` must be told, at the definition, that TypeScript reads it.
///
/// The named path is checked against the FILE SYSTEM as well as the prose.
/// Naming a mirror that is not there is the failure this guard exists to catch,
/// and prose alone cannot catch it: P0-ts found this pin still pointing at
/// `queryBuilder.ts` — which by then held the DSL parser, not the mirror — after
/// the mirror moved to its own module.
/// **T1's binding half, driven through the real registry producer.**
///
/// The picker declares a type by writing `tine.type::` on the page whose name
/// is the row's `normalized_name` **verbatim** — no key normalizer on the
/// TypeScript side (D-14). That contract is only sound if the registry binds
/// declarations exactly the way the UI spells them, so this pins both
/// directions of the one interesting case: a property authored with a SPACE.
///
/// `property_key_norm` folds `due date` to `due-date`, and `build_registry`
/// looks the declaration up under `refs::page_key(normalized_name)`. So the
/// page that binds is `due-date`; a page literally named `due date` is a
/// different page and must not bind, which is exactly why the UI is forbidden
/// to "helpfully" write the declaration on the key as the source spelled it.
#[test]
fn a_declaration_binds_on_the_normalized_key_page_not_the_authored_spelling() {
    use crate::query::atom::AtomFormat;
    use crate::query::atom::ParseConfig;
    use crate::query::registry::{build_registry, OwnerRow, OwnerType, PageMeta};

    // The block spells the key `due date`; the registry row is `due-date`.
    let authored = OwnerRow {
        owner_type: OwnerType::Block,
        owner_id: "b1".into(),
        page_id: "notes".into(),
        source_name: "due date".into(),
        normalized_name: "due-date".into(),
        ordinal: 0,
        value: "not a number".into(),
    };
    let declaration = |page_id: &str| OwnerRow {
        owner_type: OwnerType::Page,
        owner_id: page_id.into(),
        page_id: page_id.into(),
        source_name: "tine.type".into(),
        normalized_name: "tine.type".into(),
        ordinal: 0,
        value: "number".into(),
    };
    let pages = |page_id: &str| {
        Some(PageMeta {
            format: AtomFormat::Markdown,
            // The page id IS the page name here, so the fixture says out loud
            // which spelling each declaration page carries.
            name: page_id.to_string(),
        })
    };

    // The key page — `due-date` — binds.
    let bound = build_registry(
        vec![authored.clone(), declaration("due-date")].into_iter(),
        &pages,
        &ParseConfig::default(),
    )
    .expect("registry builds");
    let snapshot: RegistrySnapshot = bound.snapshot();
    let row = snapshot
        .rows
        .iter()
        .find(|row| row.normalized_name == "due-date")
        .expect("the authored `due date` key is registered as `due-date`");
    assert_eq!(
        row.declared,
        Some((ObservedType::Number, Cardinality::One)),
        "a declaration on the page named `due-date` binds to the row the picker \
         would have written it from"
    );
    assert_eq!(
        row.observed_type,
        ObservedType::Text,
        "the observed type is untouched by the declaration"
    );
    assert_eq!(
        row.mismatch_count, 1,
        "the one text value now mismatches the declared number"
    );

    // The authored spelling — `due date` — does not.
    let unbound = build_registry(
        vec![authored, declaration("due date")].into_iter(),
        &pages,
        &ParseConfig::default(),
    )
    .expect("registry builds");
    let row = unbound
        .snapshot()
        .rows
        .iter()
        .find(|row| row.normalized_name == "due-date")
        .cloned()
        .expect("the row is present either way");
    assert_eq!(
        row.declared, None,
        "a page named with the authored spelling is a different page and must \
         not bind — writing the declaration there would silently do nothing"
    );
    assert_eq!(
        row.mismatch_count, 0,
        "with no declaration there is nothing to mismatch"
    );
}
