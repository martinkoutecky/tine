use super::*;
use crate::date::JournalDate;
use crate::query::ir::{AggFn, Field};
use crate::query::{parse_query_text, tql::parse_tql};

fn tql(source: &str) -> Query {
    parse_tql(source, crate::query::registry::Registry::none()).0
}

fn og(source: &str) -> (Query, ViewSettings) {
    parse_query_text(
        source,
        crate::query::QueryDialect::Og,
        JournalDate::from_ordinal(20260904),
    )
}

/// The complete argument of a `{{query …}}` macro, so the trailing options
/// map lands in `source.og_options()` the way a real document supplies it.
fn macro_query(source: &str) -> (Query, ViewSettings) {
    crate::query::parse_query_input(
        source,
        crate::query::QueryInput::MacroQuery,
        JournalDate::from_ordinal(20260904),
        &crate::query::registry::Registry::none(),
    )
}

/// The round-trip property §4.3 defines: printing then re-parsing is the
/// identity on normalized queries.
fn round_trips(source: &str) {
    let query = tql(source);
    assert!(
        !query.is_invalid(),
        "{source} did not parse: {:?}",
        query.diagnostics
    );
    let printed = print_tql(&query);
    let again = tql(&printed);
    assert!(
        !again.is_invalid(),
        "printing {source} produced {printed:?}, which does not parse: {:?}",
        again.diagnostics
    );
    assert_eq!(
        again.normalized(),
        query.normalized(),
        "{source} printed as {printed:?}"
    );
}

/// The sheet edits group boundaries, so semantic normalization is too weak
/// for its persisted TQL. This checks the actual filter returned to the UI.
fn round_trips_exactly(source: &str) {
    let query = tql(source);
    assert!(
        !query.is_invalid(),
        "{source} did not parse: {:?}",
        query.diagnostics
    );

    let pane = print_tql(&query);
    let pane_again = tql(&pane);
    assert_eq!(
        pane_again.filter, query.filter,
        "{source} printed in the pane as {pane:?}"
    );

    let persisted = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .unwrap_or_else(|diagnostic| panic!("{source} was refused: {diagnostic:?}"));
    let macro_again = tql(&persisted);
    assert_eq!(
        macro_again.filter, query.filter,
        "{source} printed in the macro as {persisted:?}",
    );
}

#[test]
fn tql_preserves_flat_chains_and_explicit_group_boundaries_exactly() {
    for source in [
        "#a and #b and #c",
        "#a and #b and #c and #d",
        "#a or #b or #c",
        "#a or #b or #c or #d",
        "#a and (#b and #c) and #d",
        "#a or (#b or #c) or #d",
        "#a and (#b or #c) and #d",
        "off(#a) and (#b and #c) and not (#d or #e)",
    ] {
        round_trips_exactly(source);
    }
}

#[test]
fn repeated_off_collapses_without_flattening_its_authored_group() {
    let query = tql("off(off(#a and (#b and #c)))");
    let expected = tql("off(#a and (#b and #c))");
    assert_eq!(tql(&print_tql(&query)).filter, expected.filter);
    assert_eq!(tql(&print_tql_macro(&query)).filter, expected.filter);
}

#[test]
fn layered_tql_keeps_a_same_kind_group_beside_an_off_sibling() {
    let query = tql("off(#a) and (#b and #c) and #d");
    let printed = print_tql(&query);
    assert!(
        printed.contains("and ([[b]] and [[c]])"),
        "printed as {printed:?}"
    );
    assert_eq!(tql(&printed).filter, query.filter);
}

#[test]
fn reopened_structural_tql_accepts_an_edit_and_round_trips_again() {
    let original = tql("off(#a) and (#b or #c) and (#d and #e)");
    let first = query_print(
        &original,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect("the first save is representable");
    let mut reopened = tql(&first);
    assert_eq!(reopened.filter, original.filter);

    let Filter::And { items } = &mut reopened.filter else {
        panic!("the reopened root is the authored all-of group");
    };
    items.push(Filter::page_ref("f"));
    let edited = reopened.filter.clone();
    let second = query_print(
        &reopened,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect("the edited save is representable");
    assert_eq!(tql(&second).filter, edited);
}

#[test]
fn p6_tql_printer_preserves_an_explicit_same_kind_group() {
    let query = tql("#a and (#b and #c) and #d");
    let printed = print_tql(&query);
    let again = tql(&printed);
    assert_eq!(again.filter, query.filter, "printed as {printed:?}");
}

#[test]
fn every_tql_shape_round_trips() {
    for source in [
        "@block",
        "@page",
        "#x",
        "[[a b]]",
        "#x and #y",
        "#x or #y",
        "not #x",
        "#x and (#y or #z)",
        "(#x or #y) and not #z",
        "task = 'TODO'",
        "task in ('TODO', 'DOING')",
        "priority = 'A'",
        "content like '%foo%'",
        "content match 'foo'",
        "scheduled is not null",
        "deadline is null",
        "scheduled between today and '+7d'",
        "page.name = 'Home'",
        "page.name like 'proj/%'",
        "page.journal = true",
        "page.day between '2026-01-01' and '2026-12-31'",
        "prop('k') = 'v'",
        "prop('k') != 'v'",
        "prop('k') is null",
        "prop('k') is not null",
        "prop('k') = ''",
        "prop('k') in ('a', 'b')",
        "prop('k') > 3",
        "every(prop('k'), value > 3)",
        "none(prop('k'), value = 'x')",
        "page_prop('status') = 'public'",
        "page_tag('work')",
        "tag('x')",
        "any(children, task = 'TODO')",
        "every(children, task = 'DONE')",
        "none(children, task = 'TODO')",
        "@page and any(blocks, task = 'TODO')",
        "@page and name = 'Home'",
        "@page and journal = true",
        "off(#x)",
        "not off([[a]])",
        "any(children, off(task = 'TODO'))",
        "([[a]] or off([[b]]))",
        "#a and (#b or off(#c))",
    ] {
        round_trips(source);
    }
}

/// Every `{{query …}}` shipped in the repository — the templates the app
/// installs and the parser fixture — parsed as OG, printed as TQL, and
/// re-parsed. This is the corpus round-trip §4.3 asks for; the private
/// anonymized graph contains no `{{query}}` at all, so it cannot supply one.
#[test]
fn the_shipped_query_corpus_round_trips_through_the_tql_printer() {
    const CORPUS: &[&str] = &[
        include_str!("../templates/showcase.md"),
        include_str!("../templates/sheets.md"),
    ];
    let mut seen = 0usize;
    for text in CORPUS {
        for source in macro_arguments(text) {
            let (query, _) = og(&source);
            assert!(
                !query.is_invalid(),
                "shipped query {source:?} does not parse: {:?}",
                query.diagnostics
            );
            let printed = print_tql(&query);
            let again = tql(&printed);
            assert!(
                !again.is_invalid(),
                "{source:?} printed as {printed:?}, which does not parse: {:?}",
                again.diagnostics
            );
            assert_eq!(
                again.normalized().filter,
                query.normalized().filter,
                "{source:?} printed as {printed:?}"
            );
            seen += 1;
        }
    }
    // og ships five `{{query}}` macros (showcase + sheets); master's larger
    // Guide corpus pinned ten. A scan that finds fewer lost a template.
    assert!(seen >= 5, "the corpus scan found only {seen} queries");
}

/// Every `{{query …}}` argument in one document, brace-balanced so a
/// trailing options map stays inside the macro.
fn macro_arguments(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("{{query") {
        let after = &rest[at + "{{query".len()..];
        let Some(end) = after.find("}}") else { break };
        let argument = after[..end].trim();
        if !argument.is_empty() {
            out.push(argument.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

#[test]
fn adjacent_root_off_siblings_print_as_two_dash_runs() {
    let query = tql("-- #x\n--\n-- and #y");
    assert_eq!(print_tql(&query), "-- [[x]]\n--\n-- and [[y]]");
    round_trips("-- #x\n--\n-- and #y");
}

#[test]
fn an_off_below_the_root_prints_inline() {
    assert_eq!(print_tql(&tql("not off([[a]])")), "not off([[a]])");
    assert_eq!(
        print_tql(&tql("any(children, off([[a]]))")),
        "any(children, off([[a]]))"
    );
    // At the ROOT the `Off` operand takes the `-- ` line form, and the
    // connector sits inside the run exactly where the pre-pass lifts it out.
    assert_eq!(print_tql(&tql("[[a]] or off([[b]])")), "[[a]]\n-- or [[b]]");
}

#[test]
fn an_off_inside_a_group_stays_inline() {
    // The `or` group is not the root, so its `Off` operand prints inline.
    let query = tql("[[z]] and ([[a]] or off([[b]]))");
    assert_eq!(print_tql(&query), "[[z]] and ([[a]] or off([[b]]))");
}

#[test]
fn a_root_off_operand_prefixes_its_line() {
    let query = tql("#a\n-- and #b");
    assert_eq!(print_tql(&query), "[[a]]\n-- and [[b]]");
}

#[test]
fn nested_off_collapses_under_normalization() {
    let query = tql("off(off([[a]]))");
    assert_eq!(
        query.normalized().filter,
        Filter::off(Filter::page_ref("a"))
    );
}

// -- OG printer ---------------------------------------------------------

fn og_round_trips(source: &str) {
    let (query, view) = og(source);
    assert!(
        !query.is_invalid(),
        "{source} did not parse: {:?}",
        query.diagnostics
    );
    let printed = query_print(&query, &view, PrintDialect::Og, false)
        .unwrap_or_else(|d| panic!("{source} is not OG-printable: {d:?}"));
    let (again, again_view) = og(&printed);
    assert_eq!(
        again.normalized(),
        query.normalized(),
        "{source} printed as {printed:?}"
    );
    assert_eq!(again_view, view, "{source} printed as {printed:?}");
}

/// Group boundaries are authored state, so normalized equality is not a
/// sufficient save/reopen oracle for the builder's OG output.
fn og_round_trips_exactly(source: &str, expected: &str) {
    let (query, view) = og(source);
    assert!(
        !query.is_invalid(),
        "{source} did not parse: {:?}",
        query.diagnostics
    );
    let printed = query_print(&query, &view, PrintDialect::Og, false)
        .unwrap_or_else(|d| panic!("{source} is not OG-printable: {d:?}"));
    assert_eq!(printed, expected);
    let (again, again_view) = og(&printed);
    assert_eq!(
        again.anchor, query.anchor,
        "{source} printed as {printed:?}"
    );
    assert_eq!(
        again.filter, query.filter,
        "{source} printed as {printed:?}"
    );
    assert_eq!(again_view, view, "{source} printed as {printed:?}");
}

#[test]
fn og_save_reopen_preserves_authored_boolean_groups_exactly() {
    for source in [
        "(and (and \"beta\" \"alpha\") (and \"gamma\" \"delta\"))",
        "(or (or [[beta]] [[alpha]]) (or [[gamma]] [[delta]]))",
        "(not (and (and [[beta]] [[alpha]]) [[gamma]]))",
        "(and (and (page Alpha) (page Beta)) \"needle\")",
        "(and (and (namespace Alpha) (namespace Beta)) (namespace Gamma))",
    ] {
        og_round_trips_exactly(source, source);
    }
}

#[test]
fn og_group_roundtrip_keeps_directives_and_opaque_options_once() {
    let source = "(and (and (page Alpha) (page Beta)) (page Gamma)) (sort-by page asc) {:title \"Grouped\" :collapsed? true}";
    og_round_trips_exactly(source, source);
}

#[test]
fn og_expressible_queries_round_trip_through_the_og_printer() {
    for source in [
        "[[Alpha]]",
        "(and [[Alpha]] [[Beta]])",
        "(or [[Alpha]] [[Beta]])",
        "(not [[Alpha]])",
        "(task TODO DOING)",
        "(priority A)",
        "(property status active)",
        "(page-property category work)",
        "(page Home)",
        "(namespace Project)",
        "(journal)",
        "(page-tags public private)",
        "(between scheduled today +7d)",
        "(and (task TODO) (page Home))",
    ] {
        og_round_trips(source);
    }
}

#[test]
fn the_og_printer_refuses_what_og_cannot_say() {
    for source in [
        "off(#x)",
        "tag('x')",
        "any(children, task = 'TODO')",
        "every(prop('k'), value > 3)",
        "prop('k') > 3",
        // Tine-only heads the OG-syntax parser reads but the printer must
        // never write back (§3.3 B4).
        "content match 'foo'",
        "scheduled is not null",
    ] {
        let query = tql(source);
        let view = ViewSettings::default();
        assert!(
            !og_expressible(&query, &view),
            "{source} must not be OG-expressible"
        );
        let printed = query_print(&query, &view, PrintDialect::Og, false);
        assert!(
            matches!(&printed, Err(d) if d.kind == DiagnosticKind::NotApplicable),
            "{source} printed as {printed:?}"
        );
    }
}

/// mldoc's macro grammar has a leading-page-reference argument alternative:
/// an argument that STARTS with `[[` and carries anything after it is read
/// back as plain text, not a macro. Measured on mldoc 1.5.7 and lsdoc, in
/// Markdown and Org: `{{query [[a]]}}` is a macro, `{{query [[a]] X}}` is
/// not, for a view directive and an options map alike.
///
/// So the printer wraps such a form in a single-child `and`, which OG's own
/// `simplify-query` collapses — the same query, and readable. Refusing here
/// instead would take away a form the author can legitimately write.
#[test]
fn a_page_ref_form_stays_readable_when_anything_follows_it() {
    for (source, expected) in [
        ("[[a]] (sort-by page asc)", "(and [[a]]) (sort-by page asc)"),
        ("[[a]] {:title \"T\"}", "(and [[a]]) {:title \"T\"}"),
        (
            "[[a]] (sort-by page asc) {:title \"T\"}",
            "(and [[a]]) (sort-by page asc) {:title \"T\"}",
        ),
    ] {
        let (query, view) = macro_query(source);
        let printed = query_print(&query, &view, PrintDialect::Og, false)
            .unwrap_or_else(|d| panic!("{source} refused: {d:?}"));
        assert_eq!(printed, expected);
        let (again, _) = macro_query(&printed);
        assert_eq!(again.normalized(), query.normalized(), "{source}");
    }
}

/// The wrap is NOT applied when nothing follows the reference: a bare
/// `{{query [[a]]}}` is already a macro, and widening the rewrite would
/// churn every such block on its next save.
#[test]
fn a_bare_page_ref_form_is_left_exactly_as_og_writes_it() {
    let (query, view) = macro_query("[[a]]");
    let printed = query_print(&query, &view, PrintDialect::Og, false).expect("printable");
    assert_eq!(printed, "[[a]]");
}

#[test]
fn the_og_printer_re_emits_the_view_and_the_options_map_verbatim() {
    let (query, view) = og("(task TODO) (sort-by page asc) {:title \"T\"}");
    let printed = query_print(&query, &view, PrintDialect::Og, false).expect("printable");
    assert_eq!(printed, "(task TODO) (sort-by page asc) {:title \"T\"}");
}

/// **Directive migration (§4.3, Q15, P2).** `aggregate` and `group-by` have
/// no OG reader worth preserving — Logseq ignores both — and Tine now keeps
/// them in `tine.col-aggregates::` / `tine.group-by::`, which the reader's
/// precedence merge already consumes. So the OG printer must NOT re-emit
/// them: a block that stays `{{query}}` would otherwise carry the same view
/// twice, and the copy in the text would silently outlive a removal made in
/// the builder. `sort-by` and `sample` stay in the text (Q15).
#[test]
fn the_og_printer_leaves_aggregates_and_group_by_out_of_the_text() {
    let (query, mut view) = og("(task TODO) (sort-by page asc) (sample 5)");
    view.aggregates = vec![
        (Field::new(""), AggFn::Count),
        (Field::new("hours"), AggFn::Sum),
    ];
    view.group_by = Some(Field::new("status"));
    assert!(
        og_expressible(&query, &view),
        "aggregates and a group-by do not make a query inexpressible"
    );
    let printed = query_print(&query, &view, PrintDialect::Og, false).expect("printable");
    assert_eq!(printed, "(task TODO) (sort-by page asc) (sample 5)");
    assert!(!printed.contains("aggregate"), "printed as {printed}");
    assert!(!printed.contains("group-by"), "printed as {printed}");
}

/// **The builder's typed operators are IR the engine already reads back
/// (SPEC §7.4, §9 P2 "typed operators"; T2).**
///
/// `operatorsFor` in `src/editor/queryBuilder.ts` maps a registry type to a
/// comparison family and constructs the matching `Value`. It chooses among
/// `CmpOp`s that already exist — but "already exists in the enum" is not the
/// same as "prints and parses back". This pins the second: every
/// `(op, operand)` pair that helper can emit, on the property-value shape it
/// emits it in, survives `query_print(tql_macro)` + a re-parse unchanged.
/// A pair that did not would be a builder writing chips the next open
/// silently rereads as something else.
#[test]
fn every_typed_property_operator_the_builder_offers_round_trips() {
    let key = || {
        Filter::leaf(Leaf::Attr {
            attr: Attr::Key,
            op: CmpOp::Eq,
            value: Value::text("cost"),
        })
    };
    let cases: Vec<(CmpOp, Value)> = vec![
        // number
        (CmpOp::Eq, Value::Number { number: 100.0 }),
        (CmpOp::NotEq, Value::Number { number: 100.0 }),
        (CmpOp::Lt, Value::Number { number: 100.0 }),
        (CmpOp::Le, Value::Number { number: 100.0 }),
        (CmpOp::Gt, Value::Number { number: 100.0 }),
        (CmpOp::Ge, Value::Number { number: 100.0 }),
        // date (the operand is the unresolved literal, A6)
        (
            CmpOp::Eq,
            Value::Date {
                literal: "today".to_string(),
            },
        ),
        (
            CmpOp::Lt,
            Value::Date {
                literal: "2026-01-01".to_string(),
            },
        ),
        (
            CmpOp::Gt,
            Value::Date {
                literal: "-30d".to_string(),
            },
        ),
        (
            CmpOp::Between,
            Value::List {
                items: vec![
                    Value::Date {
                        literal: "today".to_string(),
                    },
                    Value::Date {
                        literal: "+7d".to_string(),
                    },
                ],
            },
        ),
        // checkbox
        (CmpOp::Eq, Value::Bool { value: true }),
        (CmpOp::NotEq, Value::Bool { value: false }),
        // text and ref
        (CmpOp::Eq, Value::text("book")),
        (CmpOp::NotEq, Value::text("book")),
        (CmpOp::Like, Value::text("%50\\%%")),
        (CmpOp::StartsWith, Value::text("Proj")),
    ];
    for (op, value) in cases {
        let filter = Filter::leaf(Leaf::Rel {
            rel: Rel::Props,
            quant: Quant::Any,
            pred: Box::new(Filter::And {
                items: vec![
                    key(),
                    Filter::leaf(Leaf::Attr {
                        attr: Attr::Value,
                        op,
                        value: value.clone(),
                    }),
                ],
            }),
        });
        let query = Query {
            anchor: Anchor::Block,
            filter,
            diagnostics: Vec::new(),
            source: Source::Builder,
        };
        let view = ViewSettings::default();
        let printed = query_print(&query, &view, PrintDialect::TqlMacro, false)
            .unwrap_or_else(|d| panic!("{op:?} {value:?} refused: {d:?}"));
        let (again, _) = crate::query::parse_query_input(
            &printed,
            crate::query::QueryInput::MacroTql,
            JournalDate::from_ordinal(20260904),
            &crate::query::registry::Registry::none(),
        );
        assert!(
            !again.is_invalid(),
            "{printed} did not parse: {:?}",
            again.diagnostics
        );
        assert_eq!(
            again.normalized().filter,
            query.normalized().filter,
            "{op:?} {value:?} printed as {printed}"
        );
    }
}

#[test]
fn a_quoted_value_survives_the_og_escaping() {
    let (query, view) = og("(property note \"a \\\"b\\\" c\")");
    let printed = query_print(&query, &view, PrintDialect::Og, false).expect("printable");
    let (again, _) = og(&printed);
    assert_eq!(again.normalized(), query.normalized());
}

/// Rule 3 (og 14 Q2 Reader B): the printer's bare-or-`[[ ]]` choice is the
/// resolver's grammar. `-7D` (uppercase unit) does not resolve, so it is not
/// printed as a bare date; a `yyyy_MM_dd` stem resolves, so it is.
#[test]
fn the_og_printer_writes_bare_exactly_the_tokens_the_resolver_reads() {
    let today = JournalDate::from_ordinal(20260904);
    for (source, expected) in [
        (
            "(between scheduled 2026_01_05 today)",
            "(between scheduled 2026_01_05 today)",
        ),
        (
            "(between scheduled [[-7D]] today)",
            "(between scheduled [[-7D]] today)",
        ),
        ("(between scheduled -7d NOW)", "(between scheduled -7d NOW)"),
    ] {
        let (query, view) = og(source);
        assert!(!query.is_invalid(), "{source}: {:?}", query.diagnostics);
        let printed = print_og(&query, &view, false).expect(source);
        assert_eq!(printed, expected, "{source}");
        let Some(literal) = printed.split_whitespace().nth(2) else {
            panic!("{printed}")
        };
        let bare = !literal.starts_with("[[");
        assert_eq!(
            bare,
            crate::query::resolve_date_token(literal.trim_matches(['[', ']']), today).is_some(),
            "{literal}: printed bare iff it resolves"
        );
    }
}

/// A small deterministic generator over the TQL grammar (xorshift, fixed
/// seed), so the round-trip property is checked over COMPOSITIONS — compound
/// atom tests under every quantifier, `not`/`and`/`or` at every depth, page
/// hops — not over a hand-picked list. A printer that drops a quantifier or a
/// parenthesis rewrites a saved query to a different meaning when the sheet
/// reprints it (I-4); this is the class check for that.
struct TqlGen(u64);

impl TqlGen {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn pick<'a>(&mut self, options: &[&'a str]) -> &'a str {
        options[(self.next() % options.len() as u64) as usize]
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn combine(&mut self, depth: u32, leaf: &mut dyn FnMut(&mut Self, u32) -> String) -> String {
        if depth == 0 || self.chance(35) {
            return leaf(self, depth);
        }
        match self.next() % 5 {
            0 => format!("not {}", self.combine(depth - 1, leaf)),
            1 => format!(
                "{} and {}",
                self.combine(depth - 1, leaf),
                self.combine(depth - 1, leaf)
            ),
            2 => format!(
                "{} or {}",
                self.combine(depth - 1, leaf),
                self.combine(depth - 1, leaf)
            ),
            3 => format!("({})", self.combine(depth - 1, leaf)),
            _ => format!(
                "not ({} and {})",
                self.combine(depth - 1, leaf),
                self.combine(depth - 1, leaf)
            ),
        }
    }
    fn atom_test(&mut self, depth: u32) -> String {
        self.combine(depth, &mut |g, _| {
            g.pick(&[
                "value > 1",
                "value < 5",
                "value = 'a'",
                "value != 'b'",
                "value in ('a', 'b')",
                "value not in ('c')",
                "value between 1 and 5",
                "value like 'a%'",
                "value = ''",
                "true",
            ])
            .to_string()
        })
    }
    fn block(&mut self, depth: u32) -> String {
        self.combine(depth, &mut |g, depth| match g.next() % 6 {
            0 | 1 => g
                .pick(&[
                    "#x",
                    "[[a]]",
                    "task = 'TODO'",
                    "priority = 'A'",
                    "content like '%foo%'",
                    "scheduled is not null",
                    "page.name = 'Home'",
                    "page.journal = true",
                    "tag('t')",
                    "page_tag('w')",
                    "prop('k') = 'v'",
                    "prop('k') > 3",
                    "prop('k') in ('a', 'b')",
                    "prop('k') is null",
                    "prop('k') is not null",
                    "prop('k') = ''",
                    "prop('k') between 1 and 5",
                    "page_prop('s') = 'p'",
                    "page_prop('s') is null",
                ])
                .to_string(),
            2 | 3 => {
                let quant = g.pick(&["any", "every", "none"]);
                let over = g.pick(&["prop('k')", "page_prop('s')"]);
                let test = g.atom_test(depth.min(3));
                format!("{quant}({over}, {test})")
            }
            4 => {
                let quant = g.pick(&["any", "every", "none"]);
                format!(
                    "{quant}(children, {})",
                    g.block(depth.saturating_sub(1).min(2))
                )
            }
            _ => format!("off({})", g.block(depth.saturating_sub(1).min(2))),
        })
    }
}

/// Repeated disabling has one persisted representation (see
/// `repeated_off_collapses_without_flattening_its_authored_group`); every
/// other difference between the parsed and the reprinted tree is a defect.
fn collapse_repeated_off(filter: &Filter) -> Filter {
    match filter {
        Filter::Off { inner } => {
            let mut inner = inner.as_ref();
            while let Filter::Off { inner: deeper } = inner {
                inner = deeper;
            }
            Filter::off(collapse_repeated_off(inner))
        }
        Filter::Not { inner } => Filter::not(collapse_repeated_off(inner)),
        Filter::And { items } => Filter::and(items.iter().map(collapse_repeated_off).collect()),
        Filter::Or { items } => Filter::or(items.iter().map(collapse_repeated_off).collect()),
        Filter::Leaf {
            leaf: Leaf::Rel { rel, quant, pred },
        } => Filter::rel(*rel, *quant, collapse_repeated_off(pred)),
        other => other.clone(),
    }
}

#[test]
fn generated_tql_round_trips_exactly_through_both_printers() {
    let mut generator = TqlGen(0x9E37_79B9_7F4A_7C15);
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for _ in 0..4000 {
        let source = generator.block(4);
        let query = tql(&source);
        if query.is_invalid() {
            continue;
        }
        checked += 1;
        let expected = collapse_repeated_off(&query.filter);
        let pane = print_tql(&query);
        let again = tql(&pane);
        if again.filter != expected {
            failures.push(format!("pane: {source:?} → {pane:?}"));
        }
        match query_print(
            &query,
            &ViewSettings::default(),
            PrintDialect::TqlMacro,
            false,
        ) {
            Ok(persisted) => {
                if tql(&persisted).filter != expected {
                    failures.push(format!("macro: {source:?} → {persisted:?}"));
                }
            }
            // The macro form is re-spelled until the document parser reads it
            // back (`guard_page_ref_arguments`), so no generated query may be
            // refused: a refusal here is a shape the printer cannot say.
            Err(refusal) => {
                failures.push(format!("refused: {source:?} → {}", refusal.message));
            }
        }
    }
    assert!(
        checked > 2000,
        "the generator produced too few valid queries: {checked}"
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} generated queries changed on reprint; first few:\n{}",
        failures.len(),
        failures
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn a_compound_property_test_keeps_its_quantifier_and_grouping() {
    for source in [
        "any(prop('k'), value > 1 and value < 5)",
        "any(prop('k'), not value = 'a')",
        "any(prop('k'), value = 'a' or value = 'b')",
        "every(prop('k'), not value > 1 and value < 5)",
        "every(prop('k'), not (value > 1 and value < 5))",
        "none(page_prop('s'), (value = 'a' or value = 'b') and value != 'c')",
        "every(prop('k'), not (value between 1 and 5))",
        "any(prop('k'), not (value in ('a', 'b')))",
        "any(prop('k'), value = '')",
        "any(prop('k'), value is not null)",
        "every(children, any(page_prop('s'), true))",
    ] {
        round_trips_exactly(source);
    }
}

/// GH-less og follow-up (C3T): a page-reference operand right after a comma
/// made the persisted macro unreadable (`[[a]])` is not a macro argument), so
/// saving the query was refused. The macro form now spells the operand in
/// parentheses; the pane and the filter are unchanged.
#[test]
fn a_page_reference_operand_prints_into_a_macro_the_parser_reads_back() {
    for source in [
        "any(children, [[a]])",
        "none(children, [[a]] or prop('k') = 'v')",
        "every(children, not any(children, [[a]]))",
        "any(children, [[a]] and any(children, [[b]]))",
        "not (any(children, [[x y]]) and tag('t'))",
    ] {
        let query = tql(source);
        assert!(!query.is_invalid(), "{source}");
        let persisted = query_print(
            &query,
            &ViewSettings::default(),
            PrintDialect::TqlMacro,
            false,
        )
        .unwrap_or_else(|d| panic!("{source} was refused: {}", d.message));
        assert!(persisted.starts_with("@block and "), "{persisted}");
        assert_eq!(
            tql(&persisted).filter,
            query.filter,
            "{source} → {persisted}"
        );
        assert!(
            !print_tql(&query).contains("([["),
            "the pane text keeps the plain spelling: {}",
            print_tql(&query)
        );
    }
    // Text inside a string literal is the user's value: it is copied verbatim,
    // never re-spelled, so a literal `, [[` stays a refusal — loud, and the
    // only shape left that the macro form cannot say.
    let query = tql("content like '%a, [[b]]%'");
    let refused = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect_err("a `, [[` inside a string cannot be a macro argument");
    assert!(
        refused.message.contains("does not read this back"),
        "{}",
        refused.message
    );
    let query = tql("content like '%a b%' and any(children, [[c]])");
    let persisted = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect("a string without a comma is untouched");
    assert!(
        persisted.contains("'%a b%'") && persisted.contains("([[c]])"),
        "{persisted}"
    );
}
