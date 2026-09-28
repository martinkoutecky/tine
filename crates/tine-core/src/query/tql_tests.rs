use super::*;
use crate::query::ir::Leaf;

fn parse(text: &str) -> Query {
    parse_tql(text, crate::query::registry::Registry::none()).0
}

fn ok(text: &str) -> Filter {
    let query = parse(text);
    assert!(
        !query.is_invalid(),
        "{text} did not parse: {:?}",
        query.diagnostics
    );
    query.filter
}

fn rejected(text: &str) -> Vec<Diagnostic> {
    let query = parse(text);
    assert!(
        query.is_invalid(),
        "{text} was accepted as {:?}",
        query.filter
    );
    query.diagnostics
}

fn content(text: &str) -> Filter {
    Filter::attr(Attr::Content, CmpOp::Like, Value::text(text))
}

fn property(key: &str, quant: Quant, op: CmpOp, value: Value) -> Filter {
    Filter::rel(
        Rel::Props,
        quant,
        Filter::and(vec![
            Filter::attr(Attr::Key, CmpOp::Eq, Value::text(key)),
            Filter::attr(Attr::Value, op, value),
        ]),
    )
}

// -- §4.2.2 probe set ---------------------------------------------------

#[test]
fn quantifier_disambiguation_ignores_quoted_text_and_comments() {
    let source = "content = 'any(children, true)' and content = \"every(children, true)\" -- none(children, true)\nor any(children, true)";
    let rewritten = desugar(source, &mut Vec::new());
    assert_eq!(
        rewritten,
        "content = 'any(children, true)' and content = \"every(children, true)\" -- none(children, true)\nor (any(children, true))"
    );
    assert_eq!(
        desugar(
            "content like '%x%' and any /* any in a comment */ (children, true)",
            &mut Vec::new(),
        ),
        "content like '%x%' and (any /* any in a comment */ (children, true))"
    );

    let mut diagnostics = Vec::new();
    let unchanged = pre_pass("content = 'any(children, true)'", &mut diagnostics);
    assert_eq!(unchanged.offset, Some(0));
    let rewritten = pre_pass(
        "content like '%x%' and any(children, true)",
        &mut diagnostics,
    );
    assert_eq!(rewritten.offset, None);
    assert!(diagnostics.is_empty());
}

#[test]
fn probe_anchor_alone_is_every_row_of_the_anchor() {
    let query = parse("@block");
    assert_eq!(query.anchor, Anchor::Block);
    assert_eq!(query.filter, Filter::True);
    let query = parse("@page");
    assert_eq!(query.anchor, Anchor::Page);
    assert_eq!(query.filter, Filter::True);
}

#[test]
fn probe_page_anchor_with_one_condition() {
    let query = parse("@page and #x");
    assert_eq!(query.anchor, Anchor::Page);
    assert_eq!(query.filter, Filter::page_ref("x"));
}

#[test]
fn probe_two_refs_conjoined() {
    assert_eq!(
        ok("#x and #y"),
        Filter::and(vec![Filter::page_ref("x"), Filter::page_ref("y")])
    );
}

#[test]
fn unparenthesized_boolean_chains_are_nary_but_parenthesized_groups_survive() {
    let alpha = || content("%alpha%");
    let beta = || content("%beta%");
    let gamma = || content("%gamma%");
    let delta = || content("%delta%");

    assert_eq!(
        ok("off(content like '%alpha%') and content like '%beta%' and content like '%gamma%' and content like '%delta%'"),
        Filter::and(vec![Filter::off(alpha()), beta(), gamma(), delta()]),
    );
    assert_eq!(
        ok("content like '%alpha%' or content like '%beta%' or content like '%gamma%' or content like '%delta%'"),
        Filter::or(vec![alpha(), beta(), gamma(), delta()]),
    );
    assert_eq!(
        ok("content like '%alpha%' and (content like '%beta%' and content like '%gamma%') and content like '%delta%'"),
        Filter::and(vec![
            alpha(),
            Filter::and(vec![beta(), gamma()]),
            delta(),
        ]),
    );
    assert_eq!(
        ok("(content like '%alpha%' or content like '%beta%') or content like '%gamma%' or content like '%delta%'"),
        Filter::or(vec![
            Filter::or(vec![alpha(), beta()]),
            gamma(),
            delta(),
        ]),
    );
}

#[test]
fn quantifier_calls_after_boolean_operators_keep_their_exact_ir() {
    for (name, quant) in [
        ("any", Quant::Any),
        ("every", Quant::Every),
        ("none", Quant::None),
    ] {
        let relation = || Filter::rel(Rel::Children, quant, Filter::page_ref("x"));
        assert_eq!(
            ok(&format!(
                "content like '%before%' and {name}(children, ref('x'))"
            )),
            Filter::and(vec![content("%before%"), relation()]),
            "{name} after and",
        );
        assert_eq!(
            ok(&format!(
                "content like '%before%' or {name}(children, ref('x'))"
            )),
            Filter::or(vec![content("%before%"), relation()]),
            "{name} after or",
        );
    }

    let query = parse(
        "@page and any(blocks, content like '%parent owns%' and any(children, ref('ParentOwn')))",
    );
    assert!(
        !query.is_invalid(),
        "valid nested PageBlocks query: {:?}",
        query.diagnostics
    );
    assert_eq!(query.anchor, Anchor::Page);
    assert_eq!(
        query.filter,
        Filter::rel(
            Rel::Blocks,
            Quant::Any,
            Filter::and(vec![
                content("%parent owns%"),
                Filter::rel(Rel::Children, Quant::Any, Filter::page_ref("ParentOwn")),
            ]),
        )
    );
}

#[test]
fn page_hops_follow_the_current_nested_scope() {
    let page_status = || property("status", Quant::Any, CmpOp::Eq, Value::text("active"));
    let page_name = || Filter::attr(Attr::Name, CmpOp::Eq, Value::text("Projects"));

    // At a page row, both the ordinary and explicit spellings name that
    // current page. Neither gains a Page(Page(...)) hop.
    assert_eq!(ok("@page and prop('status') = 'active'"), page_status());
    assert_eq!(
        ok("@page and page_prop('status') = 'active'"),
        page_status()
    );
    assert_eq!(ok("@page and name = 'Projects'"), page_name());
    assert_eq!(
        ok("@block and page_prop('status') = 'active'"),
        Filter::rel(Rel::Page, Quant::Any, page_status()),
    );
    assert_eq!(
        ok("@block and page.name = 'Projects'"),
        Filter::rel(Rel::Page, Quant::Any, page_name()),
    );

    // `blocks` switches the current row to Block even though the outer
    // anchor remains Page. Explicit page targets must hop back from there.
    assert_eq!(
        ok("@page and any(blocks, page_prop('status') = 'active')"),
        Filter::rel(
            Rel::Blocks,
            Quant::Any,
            Filter::rel(Rel::Page, Quant::Any, page_status()),
        ),
    );
    assert_eq!(
        ok("@page and any(blocks, page.name = 'Projects')"),
        Filter::rel(
            Rel::Blocks,
            Quant::Any,
            Filter::rel(Rel::Page, Quant::Any, page_name()),
        ),
    );

    // An ordinary property inside `blocks` stays on the current block.
    assert_eq!(
        ok("@page and any(blocks, prop('status') = 'active')"),
        Filter::rel(Rel::Blocks, Quant::Any, page_status()),
    );
}

#[test]
fn quantified_page_properties_and_page_tags_use_the_same_current_scope_rule() {
    for (name, quant) in [
        ("any", Quant::Any),
        ("every", Quant::Every),
        ("none", Quant::None),
    ] {
        let expected = property("status", quant, CmpOp::Eq, Value::text("active"));
        assert_eq!(
            ok(&format!(
                "@page and {name}(prop('status'), value = 'active')"
            )),
            expected,
        );
        assert_eq!(
            ok(&format!(
                "@page and any(blocks, {name}(page_prop('status'), value = 'active'))"
            )),
            Filter::rel(
                Rel::Blocks,
                Quant::Any,
                Filter::rel(Rel::Page, Quant::Any, expected),
            ),
        );
    }

    let tag = || property("tags", Quant::Any, CmpOp::Eq, Value::text("work"));
    assert_eq!(ok("@page and page_tag('work')"), tag());
    assert_eq!(
        ok("@block and page_tag('work')"),
        Filter::rel(Rel::Page, Quant::Any, tag()),
    );
    assert_eq!(
        ok("@page and any(blocks, page_tag('work'))"),
        Filter::rel(
            Rel::Blocks,
            Quant::Any,
            Filter::rel(Rel::Page, Quant::Any, tag()),
        ),
    );
}

#[test]
fn p6_flat_boolean_chain_and_nested_current_scope_are_structural() {
    let flat = ok("off(content like '%alpha%') and content like '%beta%' and content like '%gamma%' and content like '%delta%'");
    let Filter::And { items } = flat else {
        panic!("expected And")
    };
    assert_eq!(items.len(), 4, "an unparenthesized chain is four siblings");
    assert!(matches!(items.first(), Some(Filter::Off { .. })));

    let nested = ok("@page and any(blocks, page_prop('status') = 'active')");
    let Filter::Leaf {
        leaf: Leaf::Rel {
            rel: Rel::Blocks,
            pred,
            ..
        },
    } = nested
    else {
        panic!("expected page Blocks relation");
    };
    assert!(matches!(
        pred.as_ref(),
        Filter::Leaf {
            leaf: Leaf::Rel { rel: Rel::Page, .. }
        }
    ));
}

#[test]
fn probe_bracket_ref_keeps_its_spaces() {
    assert_eq!(ok("[[a b]]"), Filter::page_ref("a b"));
}

#[test]
fn probe_bracket_inside_a_literal_is_not_sugar() {
    assert_eq!(
        ok("content like '[[not sugar]]'"),
        Filter::attr(Attr::Content, CmpOp::Like, Value::text("[[not sugar]]"))
    );
}

#[test]
fn probe_ref_in_value_position_compares_by_page_name() {
    assert_eq!(
        ok("prop('k') = [[x]]"),
        Filter::rel(
            Rel::Props,
            Quant::Any,
            Filter::and(vec![
                Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k")),
                Filter::attr(Attr::Value, CmpOp::Eq, Value::text("x")),
            ])
        )
    );
}

#[test]
fn probe_ref_list_in_value_position_compares_by_page_name() {
    assert_eq!(
        ok("prop('k') in (#a, #b)"),
        Filter::rel(
            Rel::Props,
            Quant::Any,
            Filter::and(vec![
                Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k")),
                Filter::attr(
                    Attr::Value,
                    CmpOp::In,
                    Value::List {
                        items: vec![Value::text("a"), Value::text("b")]
                    }
                ),
            ])
        )
    );
}

#[test]
fn probe_between_on_a_date_attribute() {
    assert_eq!(
        ok("scheduled between today and '+7d'"),
        Filter::attr(
            Attr::Scheduled,
            CmpOp::Between,
            Value::List {
                items: vec![Value::date("today"), Value::date("+7d")]
            }
        )
    );
}

#[test]
fn probe_unquoted_relative_date_is_rejected_with_the_quoting_suggestion() {
    let diagnostics = rejected("scheduled between today and -7d");
    assert!(diagnostics.iter().any(|d| d
        .suggestions
        .contains(&"quote relative dates: '-7d'".to_string())));
}

#[test]
fn probe_a_disabled_line_becomes_an_off_operand() {
    assert_eq!(ok("-- #x"), Filter::off(Filter::page_ref("x")));
}

#[test]
fn probe_two_runs_separated_by_a_bare_dash_line_stay_two_nodes() {
    let filter = ok("-- #x\n--\n-- and #y");
    assert_eq!(
        filter,
        Filter::and(vec![
            Filter::off(Filter::page_ref("x")),
            Filter::off(Filter::page_ref("y")),
        ])
    );
}

#[test]
fn probe_a_run_inside_a_group_is_an_operand_of_that_group() {
    let filter = ok("#a and (\n#b\n-- or #c\n)");
    assert_eq!(
        filter,
        Filter::and(vec![
            Filter::page_ref("a"),
            Filter::or(vec![
                Filter::page_ref("b"),
                Filter::off(Filter::page_ref("c")),
            ]),
        ])
    );
}

#[test]
fn probe_a_dash_line_inside_a_multiline_literal_is_not_a_run() {
    let filter = ok("content like 'a\n-- b\nc'");
    assert_eq!(
        filter,
        Filter::attr(Attr::Content, CmpOp::Like, Value::text("a\n-- b\nc"))
    );
}

#[test]
fn probe_off_written_by_hand() {
    assert_eq!(ok("off(#x)"), Filter::off(Filter::page_ref("x")));
}

#[test]
fn probe_property_presence_and_blankness() {
    assert_eq!(
        ok("prop('k') is null"),
        Filter::rel(
            Rel::Props,
            Quant::None,
            Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k"))
        )
    );
    assert_eq!(
        ok("prop('k') is not null"),
        Filter::rel(
            Rel::Props,
            Quant::Any,
            Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k"))
        )
    );
    assert_eq!(
        ok("prop('k') = ''"),
        Filter::rel(
            Rel::Props,
            Quant::Any,
            Filter::and(vec![
                Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k")),
                Filter::attr(Attr::AtomCount, CmpOp::Eq, Value::Number { number: 0.0 }),
            ])
        )
    );
}

#[test]
fn probe_every_over_property_atoms() {
    assert_eq!(
        ok("every(prop('k'), value > 3)"),
        Filter::rel(
            Rel::Props,
            Quant::Every,
            Filter::and(vec![
                Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k")),
                Filter::attr(Attr::Value, CmpOp::Gt, Value::Number { number: 3.0 }),
            ])
        )
    );
}

#[test]
fn probe_content_like_is_a_like_leaf() {
    assert_eq!(
        ok("content like '%foo%'"),
        Filter::attr(Attr::Content, CmpOp::Like, Value::text("%foo%"))
    );
}

#[test]
fn probe_trailing_percent_is_starts_with() {
    assert_eq!(
        ok("page.name like 'proj/%'"),
        Filter::rel(
            Rel::Page,
            Quant::Any,
            Filter::attr(Attr::Name, CmpOp::StartsWith, Value::text("proj/"))
        )
    );
}

#[test]
fn probe_trailing_statement_is_rejected() {
    rejected("content = 'x' DROP TABLE blocks");
}

#[test]
fn probe_subquery_is_rejected() {
    rejected("prop('k') in (select 1)");
}

// -- vocabulary ---------------------------------------------------------

/// A registry holding exactly these property keys and nothing else, so a
/// suggestion test asserts on the keys it named rather than on a fixture
/// graph's incidental vocabulary.
fn registry_with(keys: &[&str]) -> crate::query::registry::Registry {
    use crate::query::atom::AtomFormat;
    use crate::query::registry::{build_registry, OwnerRow, OwnerType, PageMeta};
    let config = crate::query::atom::ParseConfig::default();
    let rows = keys.iter().enumerate().map(|(index, key)| OwnerRow {
        owner_type: OwnerType::Block,
        owner_id: format!("block-{index}"),
        page_id: "page".to_string(),
        source_name: (*key).to_string(),
        normalized_name: crate::doc::property_key_norm(key),
        ordinal: 0,
        value: "v".to_string(),
    });
    build_registry(
        rows,
        &|_| {
            Some(PageMeta {
                format: AtomFormat::Markdown,
                name: "page".to_string(),
            })
        },
        &config,
    )
    .expect("every row names the one page")
}

#[test]
fn an_unknown_identifier_is_named_and_never_rewritten() {
    let diagnostics = rejected("frobnicate = 'x'");
    assert!(diagnostics
        .iter()
        .any(|d| d.kind == DiagnosticKind::UnknownIdent));
    // Against no registry there is nothing to suggest, and a guess is not
    // a suggestion: the list is empty rather than invented.
    assert!(diagnostics.iter().all(|d| d.suggestions.is_empty()));
    // The filter is refused either way -- a suggestion never rewrites.
    assert_eq!(parse("frobnicate = 'x'").filter, Filter::False);
}

#[test]
fn an_unknown_identifier_suggests_the_registrys_nearest_keys() {
    let registry = registry_with(&["status", "statuses", "author", "unrelated"]);
    let (query, _) = parse_tql("statuss = 'done'", &registry);
    let diagnostic = query
        .diagnostics
        .iter()
        .find(|d| d.kind == DiagnosticKind::UnknownIdent)
        .expect("an unknown identifier is named");
    // Best first: `statuses` shares one more leading character with
    // `statuss` than `status` does, so Jaro-Winkler ranks it above.
    assert_eq!(
        diagnostic.suggestions,
        vec!["prop('statuses')".to_string(), "prop('status')".to_string()],
    );
    // Suggesting is not rewriting (I-22): the query is still refused.
    assert_eq!(query.filter, Filter::False);
}

#[test]
fn a_registry_key_that_is_nothing_like_the_identifier_is_not_suggested() {
    let registry = registry_with(&["author", "unrelated"]);
    let (query, _) = parse_tql("statuss = 'done'", &registry);
    assert!(query.diagnostics.iter().all(|d| d.suggestions.is_empty()));
}

#[test]
fn an_unknown_function_and_an_unknown_relation_suggest_keys_too() {
    let registry = registry_with(&["status"]);
    let (function, _) = parse_tql("statuss('done')", &registry);
    assert_eq!(
        function
            .diagnostics
            .iter()
            .find(|d| d.kind == DiagnosticKind::UnknownIdent)
            .map(|d| d.suggestions.clone()),
        Some(vec!["prop('status')".to_string()]),
    );
    let (relation, _) = parse_tql("any(statuss, content = 'x')", &registry);
    assert_eq!(
        relation
            .diagnostics
            .iter()
            .find(|d| d.kind == DiagnosticKind::UnknownIdent)
            .map(|d| d.suggestions.clone()),
        Some(vec!["prop('status')".to_string()]),
    );
}

#[test]
fn an_unknown_function_is_not_a_condition() {
    assert!(rejected("frobnicate('x')")
        .iter()
        .any(|d| d.kind == DiagnosticKind::UnknownIdent));
}

#[test]
fn an_anchor_in_the_middle_says_where_it_belongs() {
    let diagnostics = rejected("#x and @page");
    assert!(diagnostics
        .iter()
        .any(|d| d.message == "the anchor goes first"));
}

// -- §7.4 / §3.5: the anchor mismatch is its own diagnostic, and the
// author's leaf is RETAINED rather than dropped ------------------------

/// The one enabled `NotApplicable` diagnostic of a query, or a panic.
fn not_applicable_of(query: &Query) -> &Diagnostic {
    query
        .diagnostics
        .iter()
        .find(|d| d.kind == DiagnosticKind::NotApplicable)
        .unwrap_or_else(|| {
            panic!(
                "expected a NotApplicable diagnostic, got {:?}",
                query.diagnostics
            )
        })
}

fn retained_raw(filter: &Filter) -> (&str, DiagnosticKind, Option<Span>) {
    match filter {
        Filter::Raw { text, kind, span } => (text.as_str(), *kind, *span),
        other => panic!("expected a retained Raw leaf, got {other:?}"),
    }
}

/// K4: `like` is checked against the ATTRIBUTE's type (§4.2.3 marks it ✗
/// for dates and checkboxes), not the pattern's. Before K4 the parser
/// passed the pattern's Text type, so `scheduled like '2026%'` was
/// accepted and then silently matched nothing.
#[test]
fn like_on_a_date_or_checkbox_attribute_is_a_syntax_diagnostic() {
    for (source, what) in [
        ("scheduled like '2026%'", "`scheduled`"),
        ("deadline like '%09%'", "`deadline`"),
        ("page.day like '2026%'", "`day`"),
        ("page.journal like 't%'", "`journal`"),
    ] {
        let query = parse(source);
        assert!(
            query.diagnostics.iter().any(|diagnostic| {
                diagnostic.kind == DiagnosticKind::Syntax
                    && diagnostic.message == format!("`like` does not apply to {what}")
            }),
            "{source}: {:?}",
            query.diagnostics
        );
    }
    // The text attributes keep `like`.
    for source in [
        "task like 'DO%'",
        "page.namespace like '%x%'",
        "content like '%x%'",
    ] {
        assert!(parse(source).diagnostics.is_empty(), "{source}");
    }
}

#[test]
fn a_block_attribute_at_the_page_anchor_is_retained_as_not_applicable() {
    let query = parse("@page and task = 'TODO'");
    let diagnostic = not_applicable_of(&query);
    assert_eq!(diagnostic.message, "`task` does not apply to pages");
    assert!(!diagnostic.disabled);
    // These four plain inputs are pre-pass-unchanged, so the span points
    // into the text the author actually typed.
    assert!(
        diagnostic.span.is_some(),
        "a pre-pass-unchanged input is spanned"
    );
    let (text, kind, span) = retained_raw(&query.filter);
    assert_eq!(text, "task = 'TODO'");
    assert_eq!(kind, DiagnosticKind::NotApplicable);
    assert!(span.is_some());
}

#[test]
fn a_block_relation_at_the_page_anchor_is_retained_as_not_applicable() {
    let query = parse("@page and any(children, true)");
    assert_eq!(
        not_applicable_of(&query).message,
        "`children` does not apply to pages"
    );
    let (text, kind, span) = retained_raw(&query.filter);
    assert_eq!(text, "any(children, true)");
    assert_eq!(kind, DiagnosticKind::NotApplicable);
    assert!(span.is_some());
}

#[test]
fn a_page_relation_at_the_block_anchor_is_retained_as_not_applicable() {
    let query = parse("@block and any(blocks, true)");
    assert_eq!(
        not_applicable_of(&query).message,
        "`blocks` does not apply to blocks"
    );
    let (text, kind, _) = retained_raw(&query.filter);
    assert_eq!(text, "any(blocks, true)");
    assert_eq!(kind, DiagnosticKind::NotApplicable);
}

#[test]
fn a_bare_page_attribute_at_the_block_anchor_suggests_the_page_hop() {
    let query = parse("@block and journal = true");
    let diagnostic = not_applicable_of(&query);
    assert_eq!(diagnostic.message, "`journal` does not apply to blocks");
    assert_eq!(diagnostic.suggestions, vec!["page.journal".to_string()]);
    let (text, kind, _) = retained_raw(&query.filter);
    assert_eq!(text, "journal = true");
    assert_eq!(kind, DiagnosticKind::NotApplicable);
}

#[test]
fn a_not_applicable_leaf_keeps_its_place_among_its_siblings() {
    let query = parse("@page and name = 'x' and task = 'TODO'");
    let Filter::And { items } = &query.filter else {
        panic!("expected the surrounding and, got {:?}", query.filter);
    };
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0],
        Filter::attr(Attr::Name, CmpOp::Eq, Value::text("x"))
    );
    assert_eq!(retained_raw(&items[1]).0, "task = 'TODO'");
}

#[test]
fn a_genuinely_unknown_name_is_still_an_unknown_identifier() {
    let query = parse("@page and nonsense = 1");
    assert!(query
        .diagnostics
        .iter()
        .all(|d| d.kind != DiagnosticKind::NotApplicable));
    assert!(query
        .diagnostics
        .iter()
        .any(|d| d.kind == DiagnosticKind::UnknownIdent));
    assert_eq!(query.filter, Filter::False);
}

#[test]
fn a_disabled_wrong_row_leaf_carries_a_disabled_diagnostic() {
    let query = parse("@page and off(task = 'TODO')");
    let diagnostic = not_applicable_of(&query);
    assert!(diagnostic.disabled);
    assert!(!query.is_invalid());
    let Filter::Off { inner } = &query.filter else {
        panic!("expected an off wrapper, got {:?}", query.filter);
    };
    assert_eq!(retained_raw(inner).0, "task = 'TODO'");
}

#[test]
fn a_retained_wrong_row_leaf_survives_print_and_reparse() {
    let query = parse("@page and task = 'TODO'");
    let printed = crate::query::print::print_tql(&query);
    let reparsed = parse(&printed);
    let (text, kind, _) = retained_raw(&reparsed.filter);
    assert_eq!(text, "task = 'TODO'");
    assert_eq!(kind, DiagnosticKind::NotApplicable);
    assert_eq!(
        not_applicable_of(&reparsed).message,
        "`task = 'TODO'` does not apply to this row"
    );
}

#[test]
fn ordering_operators_do_not_apply_to_text() {
    assert!(rejected("priority < 'B'")
        .iter()
        .any(|d| d.message.contains("does not apply")));
}

#[test]
fn match_applies_only_to_content() {
    assert_eq!(
        ok("content match 'foo'"),
        Filter::attr(Attr::Content, CmpOp::Match, Value::text("foo"))
    );
    assert!(rejected("task match 'foo'")
        .iter()
        .any(|d| d.message.contains("does not apply")));
}

#[test]
fn planning_presence_is_a_presence_leaf() {
    assert_eq!(
        ok("scheduled is not null"),
        Filter::attr(Attr::Scheduled, CmpOp::IsSet, Value::None)
    );
}

#[test]
fn children_quantifiers_carry_the_element_scope() {
    assert_eq!(
        ok("any(children, task = 'TODO')"),
        Filter::rel(
            Rel::Children,
            Quant::Any,
            Filter::attr(Attr::Task, CmpOp::Eq, Value::text("TODO"))
        )
    );
    assert_eq!(
        ok("@page and any(blocks, task = 'TODO')"),
        Filter::rel(
            Rel::Blocks,
            Quant::Any,
            Filter::attr(Attr::Task, CmpOp::Eq, Value::text("TODO"))
        )
    );
}

#[test]
fn page_attributes_need_no_hop_at_the_page_anchor() {
    assert_eq!(
        ok("@page and journal = true"),
        Filter::attr(Attr::Journal, CmpOp::Eq, Value::Bool { value: true })
    );
}

#[test]
fn a_tag_leaf_is_the_blocks_own_inline_tags() {
    assert_eq!(
        ok("tag('x')"),
        Filter::rel(
            Rel::Tags,
            Quant::Any,
            Filter::attr(Attr::Name, CmpOp::Eq, Value::text("x"))
        )
    );
}

#[test]
fn a_page_tag_leaf_reads_the_pages_tags_property() {
    let filter = ok("page_tag('work')");
    let Filter::Leaf {
        leaf: Leaf::Rel { rel: Rel::Page, .. },
    } = &filter
    else {
        panic!("expected a page hop, got {filter:?}");
    };
}

#[test]
fn the_recursion_limit_refuses_a_pathological_nesting() {
    let deep = format!("{}#x{}", "(".repeat(200), ")".repeat(200));
    rejected(&deep);
}

// -- pre-pass units -----------------------------------------------------

#[test]
fn literal_spans_honour_doubled_quotes() {
    let text = "a 'b''c' d";
    assert_eq!(literal_spans(text), vec![(2, 8)]);
}

#[test]
fn a_malformed_disabled_span_is_a_disabled_diagnostic_and_does_not_invalidate() {
    let query = parse("#a\n-- and )(");
    assert!(
        !query.is_invalid(),
        "a disabled diagnostic must not invalidate: {:?}",
        query.diagnostics
    );
    assert!(query.diagnostics.iter().any(|d| d.disabled));
}

#[test]
fn starts_with_recognises_only_a_single_trailing_wildcard() {
    assert_eq!(starts_with_prefix("proj/%"), Some("proj/".to_string()));
    assert_eq!(starts_with_prefix("%proj%"), None);
    assert_eq!(starts_with_prefix("pro_j%"), None);
    assert_eq!(starts_with_prefix("proj"), None);
    assert_eq!(starts_with_prefix("50\\%%"), Some("50%".to_string()));
}

/// Reader B (og 14 Q2): an out-of-range relative date is a diagnostic, not a
/// query that silently matches a garbage day.
#[test]
fn an_out_of_range_relative_date_is_a_diagnostic() {
    let diagnostics = rejected("scheduled between '-9223372036854775807d' and today");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("out of range")),
        "{diagnostics:?}"
    );
}
