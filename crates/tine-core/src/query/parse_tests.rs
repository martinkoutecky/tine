//! The parse entry points and the advanced (datalog) lowering, transcribed
//! from master `query_tests.rs` (5dfc84503): the tests that need no graph.
//! Evaluation tests over a graph belong with execution (og lane Q2).

use super::advanced_patterns::{adv_range, advanced_pred};
use super::ir::{
    Anchor, Attr, CmpOp, DiagnosticKind, ExecutionContext, Filter, Quant, Rel, Source, Value,
};
use super::parse::{parse_query_source, ADVANCED_UNRESOLVED_MESSAGE, ADVANCED_UNSUPPORTED_MESSAGE};
use super::{
    is_advanced, og, query_nesting_within_limit, query_source_within_limit, resolve_for_execution,
    QUERY_NESTING_MAX, QUERY_SOURCE_MAX_BYTES,
};
use crate::date::JournalDate;

// Fixed "today" so relative-date tests are deterministic: 2026-06-16.
const TODAY: JournalDate = JournalDate {
    year: 2026,
    month: 6,
    day: 16,
};

/// The block-anchored evaluable filter of an OG `{{query}}` source.
fn pred(src: &str) -> Filter {
    let (query, _view) = parse_query_source(src, TODAY);
    assert!(
        !query.is_invalid(),
        "{src} parsed with diagnostics: {:?}",
        query.diagnostics
    );
    match query.anchor {
        Anchor::Block => query.evaluable_filter(),
        Anchor::Page => og::rebase_to_block(&query.evaluable_filter()),
    }
}

fn e_page_ref(name: &str) -> Filter {
    Filter::page_ref(name)
}
fn e_task(markers: &[&str]) -> Filter {
    Filter::attr(
        Attr::Task,
        CmpOp::In,
        Value::List {
            items: markers.iter().map(|m| Value::text(*m)).collect(),
        },
    )
}
fn e_property(key: &str, value: Option<&str>) -> Filter {
    og::property_leaf(key.to_string(), value.map(str::to_string))
}
fn e_page_property(key: &str, value: Option<&str>) -> Filter {
    Filter::rel(Rel::Page, Quant::Any, e_property(key, value))
}

fn nested_boolean(head: &str, depth: usize, leaf: &str) -> String {
    format!(
        "{}{}{}",
        format!("({head} ").repeat(depth),
        leaf,
        ")".repeat(depth)
    )
}

#[test]
fn query_parsers_fail_closed_past_the_shared_depth_and_size_limits() {
    let simple_at_limit = nested_boolean("and", QUERY_NESTING_MAX - 1, "(task TODO)");
    assert!(!parse_query_source(&simple_at_limit, TODAY).0.is_invalid());
    let simple_too_deep = nested_boolean("and", QUERY_NESTING_MAX, "(task TODO)");
    assert!(parse_query_source(&simple_too_deep, TODAY).0.is_invalid());

    let advanced_at_limit = format!(
        "[:find (pull ?b [*]) :where {}]",
        nested_boolean("and", QUERY_NESTING_MAX - 1, "(task ?b #{\"TODO\"})")
    );
    let (accepted, _, rejected) = advanced_pred(&advanced_at_limit, None, TODAY);
    assert!(
        accepted.is_some(),
        "unexpected ignored clauses: {rejected:?}"
    );
    let advanced_too_deep = format!(
        "[:find (pull ?b [*]) :where {}]",
        nested_boolean("and", QUERY_NESTING_MAX, "(task ?b #{\"TODO\"})")
    );
    let (rejected, ran, ignored) = advanced_pred(&advanced_too_deep, None, TODAY);
    assert!(rejected.is_none());
    assert!(ran.is_empty());
    assert!(ignored.iter().any(|item| item == "query-nesting-too-deep"));

    let oversized = "x".repeat(QUERY_SOURCE_MAX_BYTES + 1);
    assert!(!query_source_within_limit(&oversized));
    assert!(parse_query_source(&oversized, TODAY).0.is_invalid());

    // The advanced parser must fail closed on size too, at `advanced_pred`
    // itself. `page_affects_advanced_query` calls it directly, so a ceiling
    // enforced only at the `run_advanced_*` entry points is not the shared
    // ceiling this module's doc comment promises.
    let oversized_advanced = format!(
        "[:find (pull ?b [*]) :where (property ?b :note \"{}\")]",
        "y".repeat(QUERY_SOURCE_MAX_BYTES)
    );
    assert!(!query_source_within_limit(&oversized_advanced));
    let (rejected, ran, ignored) = advanced_pred(&oversized_advanced, None, TODAY);
    assert!(rejected.is_none());
    assert!(ran.is_empty());
    assert!(ignored.iter().any(|item| item == "query-too-large"));

    let harmless = format!("(and (content \"{}\"))", "(".repeat(QUERY_NESTING_MAX + 10));
    assert!(query_nesting_within_limit(&harmless));

    let simple_semicolon = format!(";{}", "(".repeat(QUERY_NESTING_MAX + 1));
    assert!(
        !query_nesting_within_limit(&simple_semicolon),
        "semicolon is ordinary text, not a comment, in the simple DSL"
    );
    let advanced_comment = format!(
        "[:find ?b :where ;; {}\n(task ?b #{{\"TODO\"}})]",
        "(".repeat(QUERY_NESTING_MAX + 1)
    );
    assert!(
        query_nesting_within_limit(&advanced_comment),
        "advanced EDN comments must not count delimiter text"
    );
}

#[test]
fn parse_pageref_and_tag() {
    assert_eq!(pred("[[Foo]]"), e_page_ref("Foo"));
    assert_eq!(pred("#bar"), e_page_ref("bar"));
    assert_eq!(pred("(tag Foo)"), e_page_ref("Foo"));
}

#[test]
fn parse_boolean() {
    assert_eq!(
        pred("(and [[A]] [[B]])"),
        Filter::and(vec![e_page_ref("A"), e_page_ref("B")])
    );
    assert_eq!(pred("(not [[A]])"), Filter::not(e_page_ref("A")));
}

#[test]
fn parse_task_and_property() {
    assert_eq!(pred("(task TODO DOING)"), e_task(&["TODO", "DOING"]));
    assert_eq!(
        pred("(property type book)"),
        e_property("type", Some("book"))
    );
    assert_eq!(pred("(property public)"), e_property("public", None));
}

#[test]
fn property_key_and_ref_value_match_logseq() {
    // Leading `:` on the key is stripped (keyword form == symbol form).
    assert_eq!(
        pred("(property :type book)"),
        e_property("type", Some("book"))
    );
    // `_` → `-` (Logseq stores `my_key` as `my-key`).
    assert_eq!(pred("(property my_key v)"), e_property("my-key", Some("v")));
    // A `[[page]]` value is captured (was dropped, leaking a stray page-ref).
    assert_eq!(
        pred("(property :fach [[Foo Bar]])"),
        e_property("fach", Some("Foo Bar"))
    );
    // A `#tag` value is captured.
    assert_eq!(
        pred("(property :type #assignment)"),
        e_property("type", Some("assignment"))
    );
    // page-property mirrors the same normalization + value capture.
    assert_eq!(
        pred("(page-property :fach [[Foo]])"),
        e_page_property("fach", Some("Foo"))
    );
}

#[test]
fn reported_and_of_colon_properties_parses_both_clauses() {
    // GH: `(and (property :fach [[X]]) (property :type "#assignment"))` used to
    // parse to And[Property(":fach", None), PageRef(X)] — the colon key never
    // matched, the ref leaked, and the second clause was dropped → "No results".
    let p = pred(
        r##"(and (property :fach [[Management der digitalen Transformation]]) (property :type "#assignment"))"##,
    );
    assert_eq!(
        p,
        Filter::and(vec![
            e_property("fach", Some("Management der digitalen Transformation")),
            // OG's `parse-property-value` strips the leading `#` of a tag
            // spelling, quoted or not.
            e_property("type", Some("assignment")),
        ])
    );
}

#[test]
fn advanced_datalog_is_unsupported() {
    assert!(is_advanced(
        "[:find (pull ?b [*]) :where [?b :block/marker]]"
    ));
    assert!(parse_query_source("[:find ?b :where ...]", TODAY)
        .0
        .is_invalid());
}

#[test]
fn advanced_exact_page_property_pair_matches_page_property_predicate() {
    let source = r#"[:find (pull ?p [*])
                         :where
                         [?p :block/properties ?props]
                         [(get ?props :class)]]"#;
    let (lowered, ran, ignored) = advanced_pred(source, None, TODAY);

    assert_eq!(
        lowered.map(|query| query.filter),
        Some(pred("(page-property :class)"))
    );
    assert_eq!(ran, vec!["page-property"]);
    assert!(ignored.is_empty());
}

#[test]
fn advanced_current_page_input_lowers_the_standard_page_relationship() {
    // Logseq graph-parser revision 6e7afa8eb040686ff057156ee877193b581dd369
    // resolves the typed :current-page keyword positionally through
    // current-page-fn and lowercases it before DataScript execution.
    let refs = r#"{:query [:find (pull ?b [*])
                              :in $ ?current-page
                              :where
                              [?p :block/name ?current-page]
                              [?b :block/refs ?p]]
                      :inputs [:current-page]}"#;
    let (lowered, ran, ignored) = advanced_pred(refs, Some("Focus A"), TODAY);

    assert_eq!(
        lowered.map(|query| query.filter),
        Some(e_page_ref("focus a"))
    );
    assert_eq!(ran, vec!["current-page-ref"]);
    assert!(ignored.is_empty());

    let physical = refs.replace(":block/refs", ":block/page");
    let (lowered, ran, ignored) = advanced_pred(&physical, Some("Focus A"), TODAY);
    assert_eq!(
        lowered.map(|query| query.filter),
        Some(Filter::rel(
            Rel::Page,
            Quant::Any,
            Filter::attr(Attr::Name, CmpOp::Eq, Value::text("focus a"))
        ))
    );
    assert_eq!(ran, vec!["current-page"]);
    assert!(ignored.is_empty());
}

#[test]
fn advanced_typed_inputs_keep_date_bounds_numeric() {
    let source = r#"[:find (pull ?b [*])
                         :in $ ?start ?end
                         :where (between ?b ?start ?end)]
                        :inputs [2026-06-01 2026-06-30]"#;
    let (lowered, ran, ignored) = advanced_pred(source, Some("Not a date"), TODAY);

    assert_eq!(
        lowered.map(|query| query.filter),
        Some(Filter::rel(
            Rel::Page,
            Quant::Any,
            adv_range(Attr::Day, Some(20260601), Some(20260630))
        ))
    );
    assert_eq!(ran, vec!["between"]);
    assert!(ignored.is_empty());
}

#[test]
fn advanced_unrelated_bracket_pattern_stays_unsupported() {
    let source = r#"[:find (pull ?p [*])
                         :where
                         [?p :block/name ?name]
                         [(get ?name :class)]]"#;
    let (lowered, ran, ignored) = advanced_pred(source, None, TODAY);

    assert!(lowered.is_none());
    assert!(ran.is_empty());
    assert_eq!(ignored, vec!["pattern", "pattern"]);
}

/// GH #542: an attribute pattern lowers only when it is a filter on the
/// returned block alone.
#[test]
fn gh542_attribute_patterns_lower_only_block_local_meaning() {
    let lower = |src: &str| advanced_pred(src, None, TODAY);

    // A flipped comparison is the same bound.
    let (a, _, ignored_a) =
        lower("[:find (pull ?b [*]) :where [?b :block/scheduled ?d] [(<= ?d 20260630)]]");
    let (b, _, ignored_b) =
        lower("[:find (pull ?b [*]) :where [?b :block/scheduled ?d] [(>= 20260630 ?d)]]");
    assert!(ignored_a.is_empty() && ignored_b.is_empty());
    assert_eq!(a.unwrap().filter, b.unwrap().filter);

    // A value variable shared by two patterns is a join (scheduled == deadline):
    // neither pattern may lower to "has a schedule".
    let (_, ran, ignored) =
        lower("[:find (pull ?b [*]) :where [?b :block/scheduled ?d] [?b :block/deadline ?d]]");
    assert!(ran.is_empty(), "{ran:?}");
    assert_eq!(ignored, vec!["pattern", "pattern"]);

    // A `not` correlated with an outer binding is not "no deadline".
    let (_, ran, ignored) = lower(
        "[:find (pull ?b [*]) :where (task ?b #{\"TODO\"}) [?b :block/scheduled ?d] (not [?b :block/deadline ?d])]",
    );
    assert_eq!(ran, vec!["task"]);
    assert!(ignored.contains(&"not".to_string()), "{ignored:?}");

    // A literal of the wrong type never matches in Logseq; it is not guessed.
    let (lowered, _, ignored) = lower("[:find (pull ?b [*]) :where [?b :block/marker 3]]");
    assert!(lowered.is_none());
    assert_eq!(ignored, vec!["pattern"]);

    // The pulled variable must be the one the clauses constrain.
    let (lowered, _, _) = lower("[:find (pull ?x [*]) :where [?b :block/marker \"TODO\"]]");
    assert!(lowered.is_none());
}

/// GH #542 (og lane Q1): a marker variable narrowed by `contains?` lowers to a
/// task leaf, and the Clojure `:result-transform` is reported, not run. og's
/// pre-port engine answered this query with an empty result.
#[test]
fn gh542_contains_narrowed_marker_lowers_to_a_task_leaf() {
    let source = r#"{:title "NOW"
        :query [:find (pull ?h [*])
                :where
                [?h :block/marker ?marker]
                [(contains? #{"NOW" "DOING"} ?marker)]]
        :result-transform (fn [result] (sort-by (fn [h] (get h :block/priority "Z")) result))}"#;
    let (lowered, ran, ignored) = advanced_pred(source, None, TODAY);
    let filter = lowered
        .expect("the contains? pattern is in the subset")
        .filter;
    // Master's lowering: the pattern binds a marker, the predicate narrows it.
    assert_eq!(
        filter,
        Filter::and(vec![
            Filter::attr(Attr::Task, CmpOp::IsSet, Value::None),
            Filter::attr(
                Attr::Task,
                CmpOp::In,
                Value::List {
                    items: vec![Value::text("NOW"), Value::text("DOING")],
                },
            ),
        ])
    );
    assert!(!ran.is_empty());
    assert_eq!(ignored, vec!["result-transform"]);
}

/// §4.4: the binding boundary lowers an advanced source, replaces the
/// provisional inspection diagnostic, and carries the clause report verbatim;
/// an unsupported source resolves to `False` with the unsupported diagnostic.
#[test]
fn resolve_for_execution_binds_advanced_sources_once() {
    let (parsed, _) = super::parse_query_input(
        "[:find (pull ?b [*]) :where (task ?b #{\"TODO\"})]",
        super::QueryInput::Advanced,
        TODAY,
        super::registry::Registry::none(),
    );
    assert!(matches!(parsed.source, Source::Advanced { .. }));
    assert!(parsed
        .diagnostics
        .iter()
        .any(|d| d.message == ADVANCED_UNRESOLVED_MESSAGE));
    let resolved = resolve_for_execution(&parsed, &ExecutionContext::default(), TODAY);
    assert!(resolved.is_executable());
    assert_eq!(resolved.report().ran, vec!["task"]);
    assert!(resolved.query().diagnostics.is_empty());
    assert_eq!(resolved.today(), TODAY);

    let (unsupported, _) = super::parse_query_input(
        "[:find (pull ?b [*]) :where [?b :block/name ?n] [(get ?n :x)]]",
        super::QueryInput::Advanced,
        TODAY,
        super::registry::Registry::none(),
    );
    let resolved = resolve_for_execution(&unsupported, &ExecutionContext::default(), TODAY);
    assert!(!resolved.is_executable());
    assert_eq!(resolved.query().filter, Filter::False);
    assert!(resolved
        .query()
        .diagnostics
        .iter()
        .any(|d| d.kind == DiagnosticKind::Syntax && d.message == ADVANCED_UNSUPPORTED_MESSAGE));
}

/// I-22 benign extreme paired with the TQL size refusal: sources right AT the
/// 64 KiB ceiling parse cleanly (a long literal, and a wide flat boolean
/// chain), and one byte more is refused with a size diagnostic.
#[test]
fn tql_at_the_source_ceiling_is_accepted_and_one_byte_over_is_refused() {
    use super::{parse_query_text, QueryDialect};

    let frame = "content like '%%'";
    let literal = format!(
        "content like '%{}%'",
        "y".repeat(QUERY_SOURCE_MAX_BYTES - frame.len())
    );
    assert_eq!(literal.len(), QUERY_SOURCE_MAX_BYTES);
    let (query, _) = parse_query_text(&literal, QueryDialect::Tql, TODAY);
    assert!(!query.is_invalid(), "{:?}", query.diagnostics);
    assert!(
        matches!(
            &query.filter,
            Filter::Leaf {
                leaf: super::ir::Leaf::Attr {
                    attr: Attr::Content,
                    ..
                }
            }
        ),
        "expected one content leaf, got {:?}",
        query.filter
    );

    let term = "content like '%alpha%'";
    let joint = " and ";
    let count = (QUERY_SOURCE_MAX_BYTES + joint.len()) / (term.len() + joint.len());
    let chain = vec![term; count].join(joint);
    assert!(chain.len() <= QUERY_SOURCE_MAX_BYTES);
    assert!(chain.len() + term.len() + joint.len() > QUERY_SOURCE_MAX_BYTES);
    let (query, _) = parse_query_text(&chain, QueryDialect::Tql, TODAY);
    assert!(!query.is_invalid(), "{:?}", query.diagnostics.first());
    let Filter::And { items } = &query.filter else {
        panic!("expected a flat conjunction");
    };
    assert_eq!(items.len(), count);

    let over = format!("{literal} ");
    let (query, _) = parse_query_text(&over, QueryDialect::Tql, TODAY);
    assert!(query.is_invalid());
    assert!(query
        .diagnostics
        .iter()
        .any(|d| d.kind == DiagnosticKind::Size));
}

#[test]
fn macro_argument_limit_applies_before_splitting_an_options_map() {
    use super::{parse_query_input, QueryInput};
    use crate::query::registry::Registry;

    let oversized = format!(
        "(task TODO) {{:title \"{}\"}}",
        "x".repeat(QUERY_SOURCE_MAX_BYTES)
    );
    for input in [QueryInput::MacroQuery, QueryInput::MacroTql] {
        let (query, _) = parse_query_input(&oversized, input, TODAY, Registry::none());
        assert!(query
            .diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::Size));
    }
}
