//! Conformance of the pure query model (master `query/conformance.rs`,
//! 5dfc84503): the frozen atom vectors, the document form of a query (§4.3.1),
//! normalization and print round trips, capsules, the macro-input dispatch and
//! the §4.4 binding boundary.
//!
//! Master's truth-table and case-matrix halves evaluate queries end to end
//! over a graph; they belong with execution (og lane Q2) and are not here.

use crate::date::JournalDate;
use crate::query::ir::Anchor;
use crate::query::{parse_query_text, QueryDialect};

/// SPEC §6.2's frozen atom fixture vectors, in the module gate 3 runs.
///
/// `query::atom`'s own tests pin the MECHANISM (origin, ordinal, de-duplication,
/// classification). This is the frozen TABLE, here because gate 3 is
/// `cargo test -p tine-core conformance::` and the table is part of what that
/// gate asserts. Each row is also an oracle case in
/// `tools/og-query-oracle/atoms.cljs`, which records what OG retains for the
/// same value.
#[test]
fn the_frozen_atom_fixture_vectors_hold() {
    use crate::query::atom::{property_atoms, AtomFormat, AtomOrigin};

    let config = crate::query::atom::ParseConfig::default();
    let texts = |key: &str, value: &str| -> Vec<String> {
        property_atoms(key, value, AtomFormat::Markdown, &config)
            .into_iter()
            .map(|atom| atom.text)
            .collect()
    };

    assert_eq!(texts("k", "foo"), vec!["foo"]);
    assert_eq!(texts("k", "[[a]]"), vec!["a"]);
    assert_eq!(texts("k", "foo [[a]]"), vec!["a"]);
    assert_eq!(texts("k", "[[a]] #b"), vec!["a", "b"]);
    assert_eq!(texts("tags", "a, b"), vec!["a", "b"]);
    assert_eq!(texts("k", "a, b"), vec!["a", "b"]);
    assert_eq!(texts("k", "1,5"), vec!["1", "5"]);
    assert!(texts("k", "").is_empty());
    assert!(texts("k", "   ").is_empty());
    assert_eq!(texts("k", "[[a]] [[a]]"), vec!["a"]);
    assert_eq!(texts("k", "\"x, [[y]]\""), vec!["\"x, [[y]]\""]);
    assert_eq!(texts("k", "12"), vec!["12"]);
    assert_eq!(texts("k", "1.5"), vec!["1.5"]);
    assert_eq!(texts("tags", "[[a]], a"), vec!["a"]);
    assert_eq!(texts("template", "weekly review"), vec!["weekly review"]);
    assert_eq!(texts("title", "A, B"), vec!["A", "B"]);

    // Step 1 (v12, VERIFY-11 A1): reference parsing suppressed, comma split
    // still applied.
    let mut ignored = crate::query::atom::ParseConfig::default();
    ignored.ignored_page_references_keywords = vec!["url".into()];
    let atoms = property_atoms("url", "http://a.b/x, [[y]]", AtomFormat::Markdown, &ignored);
    assert_eq!(
        atoms.iter().map(|a| a.text.clone()).collect::<Vec<_>>(),
        vec!["http://a.b/x", "[[y]]"]
    );
    assert!(atoms.iter().all(|a| a.origin == AtomOrigin::Plain));

    // The G8 repeated-row flattening fixture: `k:: a` twice, `K:: b`, then
    // `k:: a, c` -- one owner, four source rows, first occurrence wins.
    let flattened: Vec<String> = ["a", "a", "b", "a, c"]
        .iter()
        .flat_map(|value| property_atoms("k", value, AtomFormat::Markdown, &config))
        .fold(
            Vec::new(),
            |mut out: Vec<crate::query::atom::Atom>, atom| {
                if !out.iter().any(|existing| existing.key == atom.key) {
                    out.push(atom);
                }
                out
            },
        )
        .into_iter()
        .map(|atom| atom.text)
        .collect();
    assert_eq!(flattened, vec!["a", "b", "c"]);
}

// ---------------------------------------------------------------------------
// §4.3.1 — the document form of a query (R1)
//
// These are the gate-3 half of "macro recognition/refusal on both formats"
// (§8.3). They assert the LITERAL bytes §4.3.1 pins, that the raw reader
// recovers them, and that a refusal writes nothing.
// ---------------------------------------------------------------------------

use crate::query::ir::{
    Attr, CmpOp, DiagnosticKind, Filter, Quant, Query, Rel, Source, ViewSettings,
};
use crate::query::macro_text::{macro_safe, query_macro_extent, recognizable_macro, FormFamily};
use crate::query::print::{query_print, PrintDialect};
use crate::query::registry::Registry;
use crate::query::tql::parse_tql;

/// A block-anchored, empty-view, source-free query over `filter` — the shape
/// §4.3.1's literal output table is stated for.
fn builder(filter: Filter) -> Query {
    Query::new(Anchor::Block, filter, Source::Builder)
}

fn saved_macro(query: &Query) -> String {
    let argument = query_print(
        query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .unwrap_or_else(|d| panic!("printable: {d:?}"));
    format!("{{{{tine-query {argument}}}}}")
}

fn ref_a() -> Filter {
    Filter::page_ref("a")
}
fn ref_b() -> Filter {
    Filter::page_ref("b")
}

/// §4.3.1's three literal output fixtures, byte for byte.
#[test]
fn the_three_pinned_macro_outputs_are_exact() {
    assert_eq!(
        saved_macro(&builder(Filter::and(vec![
            Filter::off(ref_a()),
            Filter::off(ref_b())
        ]))),
        "{{tine-query @block and off([[a]]) and off([[b]])}}"
    );
    assert_eq!(
        saved_macro(&builder(Filter::off(Filter::off(ref_a())))),
        "{{tine-query @block and off([[a]])}}",
        "the printer normalizes nested Off before either layout"
    );
    assert_eq!(
        saved_macro(&builder(Filter::not(Filter::off(ref_a())))),
        "{{tine-query @block and not off([[a]])}}"
    );
}

/// The title/options variant of each: the map is appended once, verbatim, and
/// the raw reader recovers it exactly — which the AST argument cannot, because
/// the document parser's macro ends before the final `}`.
#[test]
fn each_pinned_output_carries_its_options_map_and_the_raw_reader_recovers_it() {
    let options = "{:title \"T\" :collapsed? true}";
    for filter in [
        Filter::and(vec![Filter::off(ref_a()), Filter::off(ref_b())]),
        Filter::off(Filter::off(ref_a())),
        Filter::not(Filter::off(ref_a())),
    ] {
        let query = Query::new(
            Anchor::Block,
            filter,
            Source::Tql {
                original: String::new(),
                og_options: options.to_string(),
            },
        );
        let argument = query_print(
            &query,
            &ViewSettings::default(),
            PrintDialect::TqlMacro,
            false,
        )
        .expect("printable");
        assert!(
            argument.ends_with(options),
            "the options map is appended once, verbatim: {argument}"
        );
        let raw = format!("{{{{tine-query {argument}}}}}");
        let extent = query_macro_extent(&raw).expect("a macro");
        assert_eq!(
            extent.argument, argument,
            "the raw reader recovers it whole"
        );
        assert_eq!(extent.end, raw.len(), "including the options closing brace");
    }
}

/// The persisted form ALWAYS carries an explicit anchor. Without it the document
/// parser takes its leading-page-reference argument alternative and the macro
/// stops being a macro — measured in both Markdown and Org.
#[test]
fn the_persisted_form_is_anchored_and_the_unanchored_one_is_not_a_macro() {
    assert_eq!(
        saved_macro(&builder(ref_a())),
        "{{tine-query @block and [[a]]}}"
    );
    // Measured in both readers: a page reference ALONE is an accepted macro
    // argument, but one followed by anything else takes the leading-reference
    // alternative and the macro stops being a macro. The explicit anchor is
    // what makes every ordinary conjunction safe.
    assert!(recognizable_macro("tine-query", "[[a]]").is_ok());
    assert!(recognizable_macro("tine-query", "[[a]] and task = 'TODO'").is_err());
    assert!(recognizable_macro("tine-query", "@block and [[a]] and task = 'TODO'").is_ok());
}

/// A bare anchor when the filter is exactly `True`; no trailing ` and true`.
#[test]
fn a_true_filter_prints_the_anchor_alone() {
    assert_eq!(saved_macro(&builder(Filter::True)), "{{tine-query @block}}");
    let page = Query::new(Anchor::Page, Filter::True, Source::Builder);
    assert_eq!(saved_macro(&page), "{{tine-query @page}}");
}

/// §4.3.1's focused acceptance: the comma survives with no inserted space, in
/// both readers, even though the AST argument arrives split in two.
#[test]
fn a_comma_in_a_literal_survives_the_document_round_trip_in_both_formats() {
    let query = parse_tql("@block and content = 'a,b'", Registry::none()).0;
    let argument = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect("printable");
    assert_eq!(argument, "@block and content = 'a,b'");
    for format in ["md", "org"] {
        let raw = format!("{{{{tine-query {argument}}}}}");
        let nodes = lsdoc::inline(&raw, format);
        assert!(
            matches!(nodes.first(), Some(lsdoc::ast::Inline::Macro { name, .. }) if name == "tine-query"),
            "{format} must read it back as a macro"
        );
        assert_eq!(
            query_macro_extent(&raw).expect("a macro").argument,
            argument,
            "{format}: the raw reader keeps the comma with no inserted space"
        );
    }
}

/// The lone-brace / semicolon / escaped-quote fixtures produce identical split
/// AND extent results — the two readers are one scan (I-12).
#[test]
fn split_and_extent_agree_on_the_lone_brace_semicolon_and_escaped_quote_cases() {
    for form in [
        "content = '{'",
        "content = 'a;b'",
        "content = 'a''b'",
        "content = 'a,b'",
    ] {
        let options = "{:title \"T\"}";
        let argument = format!("{form} {options}");
        let (split_form, split_options) =
            crate::query::macro_text::split_trailing_map(&argument, FormFamily::Tql);
        assert_eq!(
            (split_form.as_str(), split_options.as_str()),
            (form, options)
        );
        let raw = format!("{{{{tine-query {argument}}}}}");
        let extent = query_macro_extent(&raw).expect("a macro");
        assert_eq!(extent.argument, argument);
        let (again_form, again_options) =
            crate::query::macro_text::split_trailing_map(&extent.argument, FormFamily::Tql);
        assert_eq!(
            (again_form, again_options),
            (split_form, split_options),
            "split and extent must agree on {form}"
        );
    }
}

/// An unsafe save is a refusal that writes nothing: the printer returns the
/// located diagnostic and the caller still holds its original bytes (I-4).
#[test]
fn an_unsafe_argument_is_refused_and_the_original_bytes_are_untouched() {
    let original = "{{tine-query @block and [[a]]}}";
    let query = parse_tql("@block and content = 'x}'", Registry::none()).0;
    let refusal = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect_err("a `}` in the form cannot be saved as a macro");
    assert_eq!(refusal.kind, DiagnosticKind::Syntax);
    assert!(refusal.span.is_some(), "the refusal is located");
    assert_eq!(original, "{{tine-query @block and [[a]]}}");
    // The lexical rule and the parser agree here; the parser is the authority.
    assert!(macro_safe("@block and content = 'x}'", FormFamily::Tql).is_err());
}

/// A title-only edit preserves the authored form, including a form the filter
/// printer could never regenerate.
#[test]
fn a_title_only_edit_preserves_an_unsupported_authored_form() {
    let advanced = "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]]";
    let query = Query::new(
        Anchor::Block,
        Filter::True,
        Source::Advanced {
            original: advanced.to_string(),
            og_options: "{:title \"New\"}".to_string(),
        },
    );
    let printed = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::AdvancedMacro,
        false,
    )
    .expect("source-preserving");
    assert_eq!(printed, format!("{advanced} {{:title \"New\"}}"));
    // A non-Advanced source gets `NotApplicable`, never a regenerated datalog.
    let tql = parse_tql("@block and [[a]]", Registry::none()).0;
    let refused = query_print(
        &tql,
        &ViewSettings::default(),
        PrintDialect::AdvancedMacro,
        false,
    )
    .expect_err("not an advanced source");
    assert_eq!(refused.kind, DiagnosticKind::NotApplicable);
    // `Builder` is refused on every source-preserving path.
    assert_eq!(
        query_print(
            &builder(Filter::True),
            &ViewSettings::default(),
            PrintDialect::TqlMacro,
            true,
        )
        .expect_err("a builder query has no authored form")
        .kind,
        DiagnosticKind::NotApplicable
    );
}

// ---------------------------------------------------------------------------
// §3.5 — omission-safe normalization (R2)
// ---------------------------------------------------------------------------

/// Round-trip is now **structural AND semantic**: normalizing may not change
/// what the query answers, and neither may printing and re-parsing it.
///
/// `eval` here is the executable tree — the filter after `Off` omission — which
/// is exactly what the walk and the lowering consume (§3.5).
fn evaluable(filter: &Filter) -> Filter {
    Query::new(Anchor::Block, filter.clone(), Source::Builder).evaluable_filter()
}

#[test]
fn normalization_preserves_truth_over_constants_off_not_and_relations() {
    let cases: Vec<Filter> = vec![
        Filter::True,
        Filter::False,
        Filter::and(vec![]),
        Filter::or(vec![]),
        // R2's two named regressions: identity removal turned each of these
        // into a fully disabled root, which evaluates as `True`.
        Filter::or(vec![Filter::False, Filter::off(Filter::True)]),
        Filter::not(Filter::and(vec![Filter::True, Filter::off(Filter::True)])),
        Filter::and(vec![Filter::True, Filter::off(ref_a())]),
        Filter::or(vec![Filter::off(ref_a()), Filter::off(ref_b())]),
        Filter::not(Filter::off(ref_a())),
        // A relation predicate with an empty and a nonempty related set.
        Filter::rel(Rel::Children, Quant::Any, Filter::or(vec![])),
        Filter::rel(Rel::Children, Quant::Any, Filter::and(vec![])),
        Filter::rel(
            Rel::Children,
            Quant::Any,
            Filter::and(vec![ref_a(), Filter::off(ref_b())]),
        ),
        Filter::rel(Rel::Children, Quant::Any, Filter::True),
    ];
    for filter in cases {
        let query = builder(filter.clone());
        assert_eq!(
            evaluable(&query.normalized().filter),
            evaluable(&filter),
            "normalizing changed what {filter:?} evaluates to"
        );
        // Idempotent: normalizing twice is normalizing once.
        assert_eq!(
            query.normalized().normalized().filter,
            query.normalized().filter
        );
    }
}

/// The same property through both print forms: parse(print(q)) must answer what
/// `q` answers, in the pane layout and in the persisted macro form.
#[test]
fn both_print_forms_round_trip_semantically() {
    let sources = [
        "@block and [[a]] and off([[b]])",
        "@block and (false or off(true))",
        "@block and not (true and off(true))",
        "@block and off([[a]]) and off([[b]])",
        "@block and not off([[a]])",
    ];
    for source in sources {
        let query = parse_tql(source, Registry::none()).0;
        assert!(!query.is_invalid(), "{source}: {:?}", query.diagnostics);
        let normalized = query.normalized();
        for dialect in [PrintDialect::Tql, PrintDialect::TqlMacro] {
            let printed = query_print(&query, &ViewSettings::default(), dialect, false)
                .unwrap_or_else(|d| panic!("{source} as {dialect:?}: {d:?}"));
            let again = parse_tql(&printed, Registry::none()).0;
            assert!(
                !again.is_invalid(),
                "{source} printed {printed:?} as {dialect:?}, which does not parse: {:?}",
                again.diagnostics
            );
            assert_eq!(
                again.normalized().filter,
                normalized.filter,
                "{source} as {dialect:?} printed {printed:?}: structural round trip"
            );
            assert_eq!(
                evaluable(&again.filter),
                evaluable(&query.filter),
                "{source} as {dialect:?} printed {printed:?}: semantic round trip"
            );
        }
    }
}

/// A quantifier's comma exposes a later-argument page reference: the document
/// parser reads `, [[a]])` as a malformed argument. The pane holds the plain
/// spelling; the macro printer spells the operand in parentheses so the query
/// still saves (og C3T follow-up; master refused this shape). Only text inside
/// a string literal — the user's value, never re-spelled — stays a refusal.
#[test]
fn a_quantifier_over_a_page_ref_round_trips_in_the_pane_and_in_the_macro() {
    let source = "@block and any(children, [[a]] and off([[b]]))";
    let query = parse_tql(source, Registry::none()).0;
    assert!(!query.is_invalid(), "{:?}", query.diagnostics);
    let pane = query_print(&query, &ViewSettings::default(), PrintDialect::Tql, false)
        .expect("the pane never checks macro safety");
    let again = parse_tql(&pane, Registry::none()).0;
    assert_eq!(again.normalized().filter, query.normalized().filter);
    assert_eq!(evaluable(&again.filter), evaluable(&query.filter));
    let persisted = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect("a page-reference operand is spelled so the parser reads the macro back");
    let reread = parse_tql(&persisted, Registry::none()).0;
    assert_eq!(reread.normalized().filter, query.normalized().filter);
    assert_eq!(evaluable(&reread.filter), evaluable(&query.filter));
    let literal = parse_tql("@block and content like '%a, [[b]]%'", Registry::none()).0;
    let refusal = query_print(
        &literal,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect_err("a `, [[` inside a string literal is not a macro argument");
    assert_eq!(refusal.kind, DiagnosticKind::Syntax);
}

/// Cache keys use this normalization, so two trees that differ in truth must not
/// normalize to the same key (§3.5, §5.9).
#[test]
fn normalization_never_conflates_two_trees_that_differ_in_truth() {
    let false_tree = Filter::or(vec![Filter::False, Filter::off(Filter::True)]);
    let true_tree = Filter::off(Filter::True);
    assert_ne!(
        builder(false_tree.clone()).normalized().filter,
        builder(true_tree.clone()).normalized().filter
    );
    assert_eq!(evaluable(&false_tree), Filter::or(vec![Filter::False]));
    assert_eq!(evaluable(&true_tree), Filter::True);
}

// ---------------------------------------------------------------------------
// §4.3.2 — lossless invalid and legacy leaves (R4)
// ---------------------------------------------------------------------------

/// A malformed disabled operand is CAPTURED, not replaced by `off(false)`: the
/// payload survives a save and reopen next to an untouched active row.
#[test]
fn a_broken_disabled_row_survives_a_save_and_reopen_beside_an_active_row() {
    let authored = "-- task = '\nand [[a]]";
    let query = parse_tql(authored, Registry::none()).0;
    assert!(
        !query.is_invalid(),
        "a disabled broken row does not invalidate the query: {:?}",
        query.diagnostics
    );
    let Filter::And { items } = &query.normalized().filter else {
        panic!("{:?}", query.filter);
    };
    assert_eq!(
        items[0],
        Filter::off(Filter::raw("task = '", DiagnosticKind::Syntax)),
        "the exact payload is retained, never `off(false)`"
    );
    assert_eq!(
        items[1],
        ref_a(),
        "the neighbouring active row is untouched"
    );

    // Save, reopen: payload and kind both survive, and so does the neighbour.
    let saved = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect("printable");
    let reopened = parse_tql(&saved, Registry::none()).0;
    assert_eq!(reopened.normalized().filter, query.normalized().filter);
    assert!(!reopened.is_invalid());
}

/// Payloads with line breaks, quotes, braces and Unicode survive the capsule.
#[test]
fn a_capsule_preserves_line_breaks_quotes_braces_and_unicode() {
    for payload in [
        "a\nb",
        "quote ' and \"double\"",
        "braces { } and #{",
        "unicode: héllo 😀 ключ",
        "",
    ] {
        let query = builder(Filter::raw(payload, DiagnosticKind::Syntax));
        let printed = crate::query::print::print_tql(&query);
        let again = parse_tql(&printed, Registry::none()).0;
        assert_eq!(
            again.normalized().filter,
            Filter::raw(payload, DiagnosticKind::Syntax),
            "{payload:?} did not survive {printed}"
        );
    }
}

/// Every one of the six kinds round-trips exactly; prose may be regenerated,
/// the kind may not be lost.
#[test]
fn every_diagnostic_kind_round_trips_through_its_capsule() {
    for kind in [
        DiagnosticKind::UnknownHead,
        DiagnosticKind::Syntax,
        DiagnosticKind::UnknownIdent,
        DiagnosticKind::NotApplicable,
        DiagnosticKind::Depth,
        DiagnosticKind::Size,
    ] {
        let query = builder(Filter::raw("(frobnicate x)", kind));
        let printed = crate::query::print::print_tql(&query);
        let again = parse_tql(&printed, Registry::none()).0;
        assert_eq!(
            again.normalized().filter,
            Filter::raw("(frobnicate x)", kind)
        );
        assert_eq!(
            again.diagnostics.first().map(|d| d.kind),
            Some(kind),
            "an enabled capsule yields its retained kind and invalidates the query"
        );
        assert!(
            again.is_invalid(),
            "no auto-execution of previously invalid text"
        );
    }
}

/// A capsule that will not decode degrades to `Syntax` with its bytes retained —
/// never to an executable predicate, and never to silence.
#[test]
fn a_bad_capsule_is_syntax_and_never_an_executable_predicate() {
    for bad in [
        "raw_hex('nonsense', '61')",
        "raw_hex('syntax', '6')",
        "raw_hex('syntax', 'zz')",
        "raw_hex('syntax', 'ff')",
    ] {
        let query = parse_tql(&format!("@block and {bad}"), Registry::none()).0;
        assert!(
            matches!(
                query.normalized().filter,
                Filter::Raw {
                    kind: DiagnosticKind::Syntax,
                    ..
                }
            ),
            "{bad} became {:?}",
            query.filter
        );
        assert!(query.is_invalid(), "{bad} must report its problem");
    }
}

/// Re-enabling a capsule shows the retained error and returns no results.
#[test]
fn re_enabling_a_capsule_restores_an_enabled_error_rather_than_running_it() {
    let disabled = parse_tql(
        "@block and off(raw_hex('syntax', '2d2d20'))",
        Registry::none(),
    )
    .0;
    assert!(!disabled.is_invalid(), "disabled: greyed, not invalid");
    assert!(disabled.diagnostics.iter().all(|d| d.disabled));
    let enabled = parse_tql("@block and raw_hex('syntax', '2d2d20')", Registry::none()).0;
    assert!(
        enabled.is_invalid(),
        "re-enabled: the retained error is back"
    );
    assert_eq!(
        enabled.diagnostics[0].message, "`-- ` does not parse",
        "the renderer's input is the decoded original text, not hexadecimal"
    );
}

/// The legacy `(content-regex …)` head and its TQL spelling are one leaf.
#[test]
fn the_legacy_regex_head_and_the_tql_regexp_spelling_are_the_same_leaf() {
    let (og, _) = parse_query_text(
        "(content-regex \"^a.*b$\")",
        QueryDialect::Og,
        JournalDate::from_ordinal(20260904),
    );
    let tql = parse_tql("@block and content regexp '^a.*b$'", Registry::none()).0;
    assert_eq!(tql.normalized().filter, og.normalized().filter);
    // The printer emits the TQL spelling, which parses back to the same leaf.
    // The pane layout omits the default block anchor (§4.3); the persisted
    // macro form never does.
    let printed = crate::query::print::print_tql(&og);
    assert_eq!(printed, "content regexp '^a.*b$'");
    assert_eq!(
        query_print(&og, &ViewSettings::default(), PrintDialect::TqlMacro, false)
            .expect("printable"),
        "@block and content regexp '^a.*b$'"
    );
    assert_eq!(
        parse_tql(&printed, Registry::none()).0.normalized().filter,
        og.normalized().filter
    );
}

/// `regexp` is content-only, and the `RLIKE` alias is deliberately not admitted.
#[test]
fn regexp_is_content_only_and_rlike_is_not_a_second_spelling() {
    for refused in [
        "@block and task regexp 'a'",
        "@page and name regexp 'a'",
        "@block and prop('k') regexp 'a'",
        "@block and content rlike 'a'",
    ] {
        let query = parse_tql(refused, Registry::none()).0;
        assert!(query.is_invalid(), "{refused} must be refused");
    }
    // Negation lowers through ordinary `Not`, not a second operator.
    let negated = parse_tql("@block and not content regexp 'a'", Registry::none()).0;
    assert!(!negated.is_invalid(), "{:?}", negated.diagnostics);
    assert_eq!(
        negated.normalized().filter,
        Filter::not(Filter::attr(
            Attr::Content,
            CmpOp::Regex,
            crate::query::ir::Value::text("a")
        ))
    );
}

// ---------------------------------------------------------------------------
// §7.1 — the macro-input dispatch (C3)
// ---------------------------------------------------------------------------

use crate::query::{parse_query_input, QueryInput};

fn parsed(text: &str, input: QueryInput) -> Query {
    parse_query_input(
        text,
        input,
        JournalDate::from_ordinal(20260904),
        Registry::none(),
    )
    .0
}

/// The macro inputs split their argument ONCE and record the grammar in the
/// source variant. `macro_query` picks advanced or OG by token, never by a
/// speculative parse.
#[test]
fn the_macro_inputs_split_once_and_record_the_grammar() {
    let og = parsed("(task TODO) {:title \"T\"}", QueryInput::MacroQuery);
    assert!(matches!(
        &og.source,
        Source::Og { original, og_options }
            if original == "(task TODO)" && og_options == "{:title \"T\"}"
    ));

    let tql = parsed("@block and [[a]] {:title \"T\"}", QueryInput::MacroTql);
    assert!(matches!(
        &tql.source,
        Source::Tql { original, og_options }
            if original == "@block and [[a]]" && og_options == "{:title \"T\"}"
    ));
    assert_eq!(
        tql.normalized().filter,
        ref_a(),
        "the map never reaches the grammar"
    );

    let advanced = parsed(
        "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]] {:title \"T\"}",
        QueryInput::MacroQuery,
    );
    assert!(matches!(
        &advanced.source,
        Source::Advanced { original, og_options }
            if original == "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]]"
                && og_options == "{:title \"T\"}"
    ));
}

/// A whole `{:query … :inputs …}` map is the FORM, not options (X4, §4.4).
#[test]
fn a_whole_advanced_map_is_the_form_not_the_options() {
    let form = "{:query [:find ?b :where [?b :block/marker \"TODO\"]] :inputs [:current-page]}";
    let query = parsed(form, QueryInput::MacroQuery);
    assert!(matches!(
        &query.source,
        Source::Advanced { original, og_options } if original == form && og_options.is_empty()
    ));
    // A map that FOLLOWS it still splits.
    let with_options = parsed(&format!("{form} {{:title \"T\"}}"), QueryInput::MacroQuery);
    assert!(matches!(
        &with_options.source,
        Source::Advanced { original, og_options }
            if original == form && og_options == "{:title \"T\"}"
    ));
}

/// The one discriminator protects strings and page refs: `:find` inside a
/// literal is text, and Macro and Export therefore pick the same variant.
#[test]
fn a_find_token_inside_a_literal_picks_the_same_source_variant_in_every_caller() {
    for form in [
        "(property note \"see the :find clause\")",
        "(property note \"a :where clause\")",
        "[[a :find b]]",
    ] {
        let query = parsed(form, QueryInput::MacroQuery);
        assert!(
            matches!(&query.source, Source::Og { .. }),
            "{form} is OG text, not datalog: {:?}",
            query.source
        );
    }
    assert!(matches!(
        parsed(
            "[:find ?b :where [?b :block/marker \"TODO\"]]",
            QueryInput::MacroQuery
        )
        .source,
        Source::Advanced { .. }
    ));
}

/// An options map appended to a TQL macro survives a title edit unchanged,
/// through the source-preserving path.
#[test]
fn a_tql_macro_title_edit_preserves_the_form_and_writes_the_new_map_once() {
    let mut query = parsed("@block and [[a]] {:title \"Old\"}", QueryInput::MacroTql);
    let Source::Tql { og_options, .. } = &mut query.source else {
        panic!("{:?}", query.source);
    };
    *og_options = "{:title \"New\"}".to_string();
    let printed = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        true,
    )
    .expect("source-preserving");
    assert_eq!(printed, "@block and [[a]] {:title \"New\"}");
    let raw = format!("{{{{tine-query {printed}}}}}");
    assert_eq!(query_macro_extent(&raw).expect("a macro").argument, printed);
}

// ---------------------------------------------------------------------------
// SPEC §4.4 (R5): execution-time binding
// ---------------------------------------------------------------------------

/// The `?current-page` advanced form: every block referencing the page the
/// query is rendered on. `:inputs [:current-page]` is exactly the shape
/// Logseq's own "linked references" template uses.
const CURRENT_PAGE_QUERY: &str = "{:query [:find (pull ?b [*]) \
:in $ ?p :where [?page :block/name ?p] [?b :block/refs ?page]] \
:inputs [:current-page]}";

fn advanced_query(source: &str) -> crate::query::ir::Query {
    crate::query::parse_query_input(
        source,
        QueryInput::Advanced,
        JournalDate::today(),
        crate::query::registry::Registry::none(),
    )
    .0
}

/// §4.4: the parse's provisional inspection diagnostic is REPLACED by the bound
/// lowering's verdict — a successfully bound query carries no "this is datalog"
/// syntax error, and therefore is not treated as invalid.
#[test]
fn resolution_replaces_the_provisional_inspection_diagnostic() {
    let query = advanced_query(CURRENT_PAGE_QUERY);
    assert!(
        query.is_invalid(),
        "the unresolved preview is not executable IR (§4.4)"
    );
    let resolved = crate::query::resolve_for_execution(
        &query,
        &crate::query::ir::ExecutionContext::on_page("Beta"),
        JournalDate::today(),
    );
    assert!(
        resolved.query().diagnostics.is_empty(),
        "a bound query carries the lowering's verdict, not the inspection's: {:?}",
        resolved.query().diagnostics
    );
    assert!(resolved.is_executable());
    // The immutable source survives the binding, so printing still round-trips.
    assert_eq!(
        resolved.query().source.original(),
        Some(CURRENT_PAGE_QUERY),
        "the authored form stays available for printing (§4.4)"
    );
}

/// §4.4: an OG or TQL query resolves to itself with an empty `ignored` and
/// `supported = true` — the resolver is one boundary for every source, not an
/// advanced-only detour.
#[test]
fn og_and_tql_resolve_to_themselves_with_an_empty_report() {
    for (text, dialect) in [
        ("(task TODO)", QueryDialect::Og),
        ("@block and task = 'TODO'", QueryDialect::Tql),
    ] {
        let (query, _) = parse_query_text(text, dialect, JournalDate::today());
        let resolved = crate::query::resolve_for_execution(
            &query,
            &crate::query::ir::ExecutionContext::on_page("Anywhere"),
            JournalDate::today(),
        );
        assert_eq!(resolved.query().filter, query.filter, "{text}");
        assert!(resolved.report().ignored.is_empty(), "{text}");
        assert!(resolved.report().ran.is_empty(), "{text}");
        assert!(resolved.report().supported, "{text}");
    }
}

// ---------------------------------------------------------------------------
// SPEC §5.10 (R3): `content match` is ONE semantic contract
// ---------------------------------------------------------------------------

/// §5.10, §3.3, §4.2.3: `Match` is legal on `content` and on nothing else, and
/// the IR's own recognizer says so.
#[test]
fn match_is_legal_only_on_content() {
    use crate::query::ir::{Attr, CmpOp, Filter, Value};
    for (tql, ok) in [
        ("@block and content match 'x'", true),
        ("@block and task match 'x'", false),
        ("@block and priority match 'x'", false),
        (
            "@block and any(props, key = 'k' and value match 'x')",
            false,
        ),
        ("@page and name match 'x'", false),
    ] {
        let (query, _) = parse_query_text(tql, QueryDialect::Tql, JournalDate::today());
        assert_eq!(!query.is_invalid(), ok, "{tql}: {:?}", query.diagnostics);
    }
    // The recognizer is exact: a Match on another attribute is not a search
    // query with a different subject.
    assert_eq!(
        Filter::attr(Attr::Content, CmpOp::Match, Value::text("foo")).match_sources(),
        vec!["foo"]
    );
    assert!(Filter::attr(Attr::Task, CmpOp::Match, Value::text("foo"))
        .match_sources()
        .is_empty());
    assert!(Filter::attr(Attr::Content, CmpOp::Like, Value::text("foo"))
        .match_sources()
        .is_empty());
}

// --- SPEC §8/§8.1: OG's own `:property` rule, and the line production does not
// cross -------------------------------------------------------------------
