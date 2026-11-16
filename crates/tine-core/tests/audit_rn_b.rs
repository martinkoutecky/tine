use tine_core::date::JournalDate;
use tine_core::edn::{self, Edn};
use tine_core::query::ir::ExecutionContext;
use tine_core::query::registry::Registry;
use tine_core::query::{parse_query_input, resolve_for_execution, QueryInput};

#[test]
fn audit_rn_b_guide_names_binding_refusal_edn_strings_and_static_notices() {
    let page = tine_core::guide::bundled_guide_pages()
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

#[test]
fn audit_l7s1_03_decoded_strings_keep_authored_source_and_filter_meaning() {
    let today = JournalDate::today();
    for (encoded, decoded) in [
        (r"\u0041", "A"),
        (r"\101", "A"),
        (r"\n", "\n"),
        (r"\t", "\t"),
        (r"\r", "\r"),
        (r"\b", "\u{8}"),
        (r"\f", "\u{c}"),
        (r"\u00e9", "é"),
        (r"\uD83D\uDE00", "😀"),
        (r#"a\"b\\c"#, "a\"b\\c"),
    ] {
        let quoted = edn::to_string(&Edn::Str(decoded.into()));
        for (source, expected) in [
            (
                format!(r#"(property code "{encoded}")"#),
                format!("(property code {quoted})"),
            ),
            (format!(r#""{encoded}""#), quoted.clone()),
            (
                format!(r#"(priority ["{encoded}"])"#),
                format!("(priority [{quoted}])"),
            ),
            (
                format!(r#"[:find (pull ?b [*]) :where (property ?b :code "{encoded}")]"#),
                format!("[:find (pull ?b [*]) :where (property ?b :code {quoted})]"),
            ),
        ] {
            let query =
                parse_query_input(&source, QueryInput::MacroQuery, today, Registry::none()).0;
            let expected =
                parse_query_input(&expected, QueryInput::MacroQuery, today, Registry::none()).0;
            let resolved = resolve_for_execution(&query, &ExecutionContext::default(), today);
            let expected = resolve_for_execution(&expected, &ExecutionContext::default(), today);
            assert!(
                resolved.is_executable(),
                "{source}: {:?}",
                resolved.query().diagnostics
            );
            assert_eq!(resolved.query().filter, expected.query().filter, "{source}");
            assert_eq!(
                query.source.original(),
                Some(source.as_str()),
                "authored bytes changed"
            );
        }
    }
}

#[test]
fn audit_l7s1_01_character_literal_delimiters_do_not_end_collections() {
    for literal in [
        r"\}",
        r"\]",
        r"\{",
        r"\[",
        r"\(",
        r"\)",
        r"\;",
        r#"\""#,
        r"\\",
        r"\newline",
        r"\space",
        r"\u007D",
        r"\é",
    ] {
        let source = format!("{{:custom {literal} :hidden [\"pages/Secret\"]}}");
        assert_eq!(
            tine_core::config::Config::parse(&source).hidden,
            ["pages/Secret"],
            "{literal}"
        );
        let vector = format!("[{literal} \"keep\"]");
        assert_eq!(
            tine_core::config::match_close_bracket(&vector, 0),
            vector.len() - 1,
            "{literal}"
        );
    }
}
