use tine_core::date::JournalDate;
use tine_core::query::ir::Query;
use tine_core::query::print::print_tql;
use tine_core::query::{parse_query_text, QueryDialect};

/// The first GH852 reporter transform, the literal
/// `tine-core/src/query/advanced_sort_tests.rs` recognises whole.
const FIRST: &str = r#"(fn [result] (sort-by (juxt (fn [h] (get h :block/priority "Z")) (fn [h] (get h :block/content))) result))"#;

fn og(source: &str) -> Query {
    parse_query_text(
        source,
        QueryDialect::Og,
        JournalDate::from_ordinal(20260904),
    )
    .0
}

fn tql(source: &str) -> Query {
    parse_query_text(
        source,
        QueryDialect::Tql,
        JournalDate::from_ordinal(20260904),
    )
    .0
}

#[test]
fn comment_output_choice_is_in_the_onboarding_guide() {
    let guide = include_str!("../src/templates/share-pages.md");
    for expected in [
        "Include comments",
        "on by default",
        "tine.comments:: false",
        "--no-comments",
        "whole threads",
        "Markdown/Org",
    ] {
        assert!(guide.contains(expected), "Guide must explain {expected}");
    }
}

#[test]
fn gh852_recognised_result_sorts_are_in_the_onboarding_guide() {
    let model = include_str!("../src/templates/query-model.md");
    assert!(model.contains(FIRST));
    assert!(model.contains("(get h :block/scheduled (get h :block/deadline))"));
    assert!(model.contains("full raw block text, including properties"));
    assert!(model.contains("UTF-16 code units"));
    assert!(model.contains("even one unsupported key or default refuses the whole query"));
    assert!(include_str!("../src/templates/queries.md").contains("a `juxt` of `get` functions"));
}

#[test]
fn audit_rn_b_guide_names_binding_refusal_edn_strings_and_static_notices() {
    let page = tine_guide::bundled_guide_pages()
        .into_iter()
        .find(|page| page.title == "Reference/Tine query model")
        .expect("the query model is bundled for onboarding");
    let guide = page.markdown;
    for required in [
        "Entity bindings are not guessed",
        "Tine does not translate these pairs, even with a page join",
        "[?b :block/page ?b]",
        "Quoted operands use EDN string escapes",
        "Print and static exports keep query refusal notices",
        "Only a successfully answered query with no results",
    ] {
        assert!(guide.contains(required), "Guide must explain {required}");
    }
}

/// Every `{{query …}}` shipped in the repository — the templates the app
/// installs and the parser fixture — parsed as OG, printed as TQL, and
/// re-parsed. This is the corpus round-trip §4.3 asks for; the private
/// anonymized graph contains no `{{query}}` at all, so it cannot supply one.
#[test]
fn the_shipped_query_corpus_round_trips_through_the_tql_printer() {
    const CORPUS: &[&str] = &[
        include_str!("../src/templates/showcase.md"),
        include_str!("../src/templates/sheets.md"),
    ];
    let mut seen = 0usize;
    for text in CORPUS {
        for source in macro_arguments(text) {
            let query = og(&source);
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
