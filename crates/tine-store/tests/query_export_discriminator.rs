//! I-12 (og C3 L02): query export decides simple vs advanced (datalog) with
//! the one discriminator (`tine_core::query::is_advanced`), not with the
//! caller's flag. ExportModal.tsx computed that flag with its own regex, a
//! third answerer: `{{query "meeting :where"}}` then ran through the datalog
//! engine in the export while the macro showed it as a simple query.

use std::fs;

use tine_core::query::{QueryDialect, QueryExportSpec};
use tine_store::{OpenOptions, Store};

#[test]
fn export_runs_each_query_through_the_engine_the_macro_reader_chose() {
    let dir =
        std::env::temp_dir().join(format!("tine-export-discriminator-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::write(
        dir.join("pages/Notes.md"),
        "- TODO plan the meeting :where to meet\n- DONE other\n",
    )
    .unwrap();
    let (store, _, _) = Store::open(&dir, OpenOptions::default()).unwrap();
    let view = store.whole_graph().unwrap();
    // The caller says nothing about the dialect: a text query that mentions
    // `:where` stays text, and a datalog form is recognised (one answerer, I-12).
    let batch = view
        .export_query_subtrees(&[
            QueryExportSpec {
                key: "text".into(),
                query: r#""meeting :where""#.into(),
                dialect: Default::default(),
            },
            QueryExportSpec {
                key: "datalog".into(),
                query: "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]]".into(),
                dialect: Default::default(),
            },
        ])
        .unwrap();
    let raws = |index: usize| -> Vec<String> {
        batch.results[index]
            .groups
            .iter()
            .flat_map(|group| group.blocks.iter())
            .map(|block| block.raw.clone())
            .collect()
    };
    assert_eq!(
        raws(0),
        vec!["TODO plan the meeting :where to meet".to_string()]
    );
    assert_eq!(
        raws(1),
        vec!["TODO plan the meeting :where to meet".to_string()]
    );
    store.close();
    let _ = fs::remove_dir_all(&dir);
}

/// A `{{tine-query …}}` macro carries TQL, which the export used to leave as
/// literal macro text: the spec had no way to say so and the engine read every
/// source as the OG dialect.
#[test]
fn export_expands_a_tine_query_macro_through_the_tql_dialect() {
    let dir = std::env::temp_dir().join(format!("tine-export-tql-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::write(dir.join("pages/Notes.md"), "- TODO plan\n- DONE other\n").unwrap();
    let (store, _, _) = Store::open(&dir, OpenOptions::default()).unwrap();
    let view = store.whole_graph().unwrap();
    let batch = view
        .export_query_subtrees(&[QueryExportSpec {
            key: "tql".into(),
            query: "task = 'TODO'".into(),
            dialect: QueryDialect::Tql,
        }])
        .unwrap();
    let raws: Vec<String> = batch.results[0]
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| block.raw.clone())
        .collect();
    assert_eq!(raws, vec!["TODO plan".to_string()]);
    store.close();
    let _ = fs::remove_dir_all(&dir);
}
