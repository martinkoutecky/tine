//! RN-B: synthetic user outcomes, with upstream source as the query oracle.
//! OG c67b8b5fa47f8fe1e1954226c9bdfabd46ebb968:
//! query_dsl.cljs::parse uses reader/read-string; query_custom.cljs::custom-query
//! sends authored entity bindings to query-react, without inventing page hops.
use std::fs;
use tine_core::query::ir::{ExecutionContext, QueryResult, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::{parse_query_input, QueryInput};
use tine_graph_features::{config, print, publish};
use tine_store::{IrAnswer, IrRequest, Store};

fn graph(files: &[(&str, &str)]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    (dir, store)
}

fn answer(store: &Store, text: &str) -> QueryResult {
    let (query, view) = parse_query_input(
        text,
        QueryInput::MacroQuery,
        tine_core::date::JournalDate::today(),
        Registry::none(),
    );
    let IrAnswer::Result(result) = store
        .whole_graph()
        .unwrap()
        .query_ir(
            IrRequest::Run {
                query: &query,
                view: &view,
                context: &ExecutionContext::default(),
            },
            &tine_store::Cancel::default(),
        )
        .unwrap()
        .unwrap()
    else {
        panic!("expected a query result")
    };
    *result
}

#[test]
fn audit_l7s1_01_character_literals_keep_hidden_and_surgical_config_edits() {
    let _guard = crate::integration_lock();
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
        let input = format!("; unchanged prefix\n{{:plugin {{:char {literal}}}\n :hidden [\"pages/Secret\"]\n :favorites [{literal} \"Old\"] ; keep this\n :start-of-week 6}}\n; unchanged tail\n");
        let parsed = tine_core::config::Config::parse(&input);
        assert_eq!(parsed.hidden, ["pages/Secret"], "{literal}: exclusion lost");
        let (dir, store) = graph(&[
            ("logseq/config.edn", &input),
            ("pages/Public.md", "- public\n"),
            ("pages/Secret.md", "- excluded\n"),
        ]);
        assert!(
            !store
                .whole_graph()
                .unwrap()
                .corpus()
                .pages
                .iter()
                .any(|p| p.name == "Secret"),
            "{literal}: excluded page admitted"
        );
        config::set_favorites(&store, &["New".into()], None).unwrap();
        let expected = input.replace(
            &format!(":favorites [{literal} \"Old\"]"),
            ":favorites [\"New\"]",
        );
        let text = fs::read_to_string(dir.path().join("logseq/config.edn")).unwrap();
        assert_eq!(text, expected, "{literal}: bytes outside the edit changed");
        assert!(
            tine_core::edn::parse_strict(&text).is_some(),
            "{literal}: invalid EDN"
        );
        config::set_start_of_week(&store, 2).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("logseq/config.edn")).unwrap(),
            expected.replace(":start-of-week 6", ":start-of-week 2")
        );
        assert_eq!(store.config().config.hidden, ["pages/Secret"]);
        store.close();
    }
}

#[test]
fn audit_l7s1_02_entity_constraints_refuse_without_a_guessed_answer() {
    let _guard = crate::integration_lock();
    let (_dir, store) = graph(&[
        ("pages/A.md", "class:: book\n- TODO unclassified child\n"),
        ("pages/B.md", "- TODO own class\n  class:: note\n"),
    ]);
    for source in [
        "[:find (pull ?p [*]) :where [?p :block/properties ?props] [(get ?props :class)]]",
        "[:find (pull ?b [*]) :where [?p :block/properties ?props] [(get ?props :class)] [?b :block/marker \"TODO\"]]",
        "[:find (pull ?b [*]) :where [?b :block/page ?b] [?b :block/marker \"TODO\"]]",
        "[:find (pull ?b [*]) :where [?b :block/page ?p] [?p :block/properties ?props] [(get ?props :class)]]",
    ] {
        let result = answer(&store, source);
        assert!(!result.report.supported, "{source}: unsupported entity constraints were executed");
        assert!(!result.diagnostics.is_empty(), "{source}: no on-query refusal");
        assert_eq!(result.total, 0, "{source}: fabricated answer");
    }
    // Keep the established, explicitly bound journal-page join working.
    let joined = answer(&store, "[:find (pull ?b [*]) :where [?b :block/page ?p] [?p :block/journal? true] [?b :block/marker \"TODO\"]]");
    assert!(joined.report.supported && joined.diagnostics.is_empty());
    assert_eq!(joined.total, 0);
    store.close();
}

#[test]
fn audit_l7s1_03_quoted_operands_decode_edn_or_visibly_refuse() {
    let _guard = crate::integration_lock();
    let (_dir, store) = graph(&[("pages/A.md", "- escaped text\n  code:: A\n")]);
    for source in [
        r#"(page "\u0041")"#,
        r#"(page "\101")"#,
        r#"(property code "\u0041")"#,
        r#"[:find (pull ?b [*]) :where [?b :block/page ?p] [?p :block/name "\u0061"]]"#,
    ] {
        let result = answer(&store, source);
        assert!(
            result.diagnostics.is_empty(),
            "{source}: {:?}",
            result.diagnostics
        );
        assert!(result.report.supported, "{source}");
        assert_eq!(result.total, 1, "{source}: decoded operand did not find A");
        let QueryRows::Block { groups } = result.rows else {
            panic!("block results")
        };
        assert!(groups
            .iter()
            .flat_map(|g| &g.blocks)
            .any(|b| b.raw.contains("escaped text")));
    }
    for source in [
        r#"(page "\U0041")"#,
        r#"(page "C:\Users\x")"#,
        r#"(task ["\U0041"])"#,
        r#"(and (task TODO) "\é")"#,
    ] {
        let result = answer(&store, source);
        assert!(
            !result.diagnostics.is_empty(),
            "{source}: invalid escape was accepted"
        );
        assert_eq!(result.total, 0);
    }
    store.close();
}

#[test]
fn audit_l7s2_03_print_and_publication_keep_refusal_text_in_sheet_fallbacks() {
    let _guard = crate::integration_lock();
    for extension in ["md", "org"] {
        let mut files = Vec::new();
        for (name, query) in [
            ("Window", "{{query (between scheduled -7d today)}}"),
            ("Unknown", "{{query (frobnicate x)}}"),
            ("InvalidTql", "{{tine-query @block task =}}"),
            ("Empty", "{{query (task CANCELED)}}"),
            ("Binding", "{{query [:find (pull ?p [*]) :where [?p :block/properties ?props] [(get ?props :class)]]}}"),
        ] {
            let path = format!("pages/{name}.{extension}");
            let text = if extension == "org" {
                format!("#+PUBLIC: true\n* {query}\n:PROPERTIES:\n:tine.view: table\n:END:\n* {query}\n")
            } else {
                format!("public:: true\n- {query}\n  tine.view:: table\n- {query}\n")
            };
            files.push((path, text));
        }
        let borrowed: Vec<_> = files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        let (_dir, store) = graph(&borrowed);
        let (site, _) = publish::publish_html(&store).unwrap();
        for name in ["Window", "Unknown", "InvalidTql", "Binding", "Empty"] {
            let printed = print::page_print_html(&store, name, Default::default())
                .unwrap()
                .unwrap();
            let published = fs::read_to_string(
                std::path::Path::new(&site).join(format!("{}.html", name.to_lowercase())),
            )
            .unwrap();
            for html in [&printed, &published] {
                if name == "Empty" {
                    assert!(
                        html.contains("No matching blocks."),
                        "{extension}/{name}: {html}"
                    );
                    assert!(!html.contains("query-refused"));
                } else {
                    let body = html.split("</style>").last().unwrap();
                    assert_eq!(
                        html.matches("query-refused").count(),
                        2,
                        "{extension}/{name}: both macros need a visible refusal: {body}"
                    );
                    assert!(
                        !html.contains("No matching blocks."),
                        "{extension}/{name}: refusal became an empty answer: {body}"
                    );
                    if name == "Window" {
                        assert!(
                            html.contains("Convert it to a Tine query"),
                            "lost A14 conversion explanation"
                        );
                    }
                }
            }
        }
        store.close();
    }
}
