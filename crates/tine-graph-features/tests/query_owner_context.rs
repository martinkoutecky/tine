//! I-12: static lists, sheet inputs/layout and baked queries use their owner.
use serde_json::{json, Value};
use std::fs;
use tine_graph_features::publish::sheet_export_inputs;
use tine_graph_features::publish_query::{publish_live, publish_live_with_sheets};
use tine_store::Store;

fn check(query: &str, sheet: bool) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    let out = dir.path().join("output");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(&out).unwrap();
    let host = format!(
        "public:: true\n- {{{{query {query}}}}}\n{}",
        if sheet { "  tine.view:: table\n" } else { "" }
    );
    fs::write(root.join("pages/Public.md"), &host).unwrap();
    fs::write(
        root.join("pages/Second.md"),
        format!("{host}- TODO from second [[Public]]\n"),
    )
    .unwrap();
    fs::write(
        root.join("pages/Third.md"),
        "public:: true\n- TODO from third [[Second]]\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let bundle = vec![("index.html".into(), b"<html><head></head></html>".to_vec())];
    if sheet {
        let inputs = sheet_export_inputs(&store, None, None).unwrap();
        let exports = inputs
            .iter()
            .filter(|input| input.page == "Public" || input.page == "Second")
            .map(|input| {
                let query = input
                    .query
                    .as_ref()
                    .expect("current-page sheet must receive rows");
                assert!(query
                    .rows
                    .iter()
                    .any(|row| row.raw.contains(if input.page == "Public" {
                        "from second"
                    } else {
                        "from third"
                    })));
                if input.page == "Public" {
                    assert!(!query.rows.iter().any(|row| row.raw.contains("from third")));
                }
                let rows: Vec<_> = query
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(ix, row)| json!({ "ix":ix, "title":row.raw, "bg":null, "cells":[] }))
                    .collect();
                serde_json::from_value(
                    json!({"page":input.page, "path":input.path, "fp":query.fp, "query":true,
                "view":"table", "columns":[{"label":"Block", "formula":false}], "rows":rows,
                "footer":null,"filterError":null,"omitted":0}),
                )
                .unwrap()
            })
            .collect();
        publish_live_with_sheets(&store, &out, "Context", false, &bundle, exports).unwrap();
    } else {
        publish_live(&store, &out, "Context", false, &bundle).unwrap();
    }
    let snap: Value =
        serde_json::from_slice(&fs::read(out.join("context/app/snapshot.json")).unwrap()).unwrap();
    for owner in ["Public", "Second"] {
        let html =
            fs::read_to_string(out.join(format!("context/{}.html", owner.to_lowercase()))).unwrap();
        let result = if sheet {
            html.split("<table class=\"sheet-table\">")
                .nth(1)
                .expect("current-page sheet must lay out")
                .split("</table>")
                .next()
                .unwrap()
        } else {
            html.split("class=\"query-results\"")
                .nth(1)
                .expect("owning-page query must return rows")
                .split("</ul>")
                .next()
                .unwrap()
        };
        let baked = snap["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["host"] == owner)
            .unwrap();
        let raw = serde_json::to_string(&baked["result"]).unwrap();
        for text in ["from second", "from third"] {
            assert_eq!(result.contains(text), raw.contains(text), "I-12: static and baked query answers agree for {owner}; exemplar render_query_cache.rs");
        }
        assert!(result.contains(if owner == "Public" {
            "from second"
        } else {
            "from third"
        }));
        if owner == "Public" {
            assert!(!result.contains("from third"));
        }
    }
    store.close();
}

#[test]
fn current_page_queries_match_baked_publication_context() {
    check("{:query [:find (pull ?b [*]) :in $ ?current-page :where [?p :block/name ?current-page] [?b :block/refs ?p] [?b :block/marker \"TODO\"]] :inputs [:current-page]}", false);
}

#[test]
fn current_page_sheets_match_baked_publication_context() {
    check("(and (task TODO) <% current page %>)", true);
}

#[test]
fn current_page_advanced_map_sheets_match_baked_publication_context() {
    check("{:query [:find (pull ?b [*]) :in $ ?current-page :where [?p :block/name ?current-page] [?b :block/refs ?p] [?b :block/marker \"TODO\"]] :inputs [:current-page]}", true);
}

#[test]
fn current_page_template_substitution_is_shared_by_lists_sheets_and_baked_queries() {
    for sheet in [false, true] {
        check("(and (task TODO) <% current page %>)", sheet);
    }
}
