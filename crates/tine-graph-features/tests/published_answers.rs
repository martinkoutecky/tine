//! The published snapshot's `answers` (CM step 2, census Q13;
//! `docs/contracts/quick-share.md`, "Published answers"): the writer fills it
//! with the live answerers over the private build graph of the exported pages.
//!
//! - `published_answer_fixtures_match_the_writer`: the snapshots the viewer
//!   tests read (`tests/fixtures/published-answers/*.json`, used by
//!   `src/publishedAnswers*.test.ts`), written by the static export.
//!   `TINE_PUBLISHED_ANSWERS_FIXTURES=write` rewrites them.
//! - `answers_are_closed_by_the_exported_documents`: two graphs whose exported
//!   documents are identical but whose private pages differ write
//!   byte-identical `answers`, through quick share and the static export.
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::publish_query::{publish_live, QueryExportRequest};
use tine_graph_features::quick_share;
use tine_store::Store;

fn bundle() -> Vec<(String, Vec<u8>)> {
    vec![(
        "index.html".into(),
        b"<!doctype html><html><head><title>Tine</title></head><body></body></html>".to_vec(),
    )]
}

fn graph(root: &Path, files: &[(&str, &str)]) -> Store {
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = Store::open(root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    store
}

/// Runtime block ids are fresh per parse: renumber them by first occurrence.
fn normalize(doc: &mut Value) {
    fn collect(v: &Value, ids: &mut HashMap<String, String>) {
        match v {
            Value::Object(map) => {
                if map.get("has_id") == Some(&Value::Bool(false)) {
                    if let Some(Value::String(id)) = map.get("id") {
                        let next = format!("runtime-{}", ids.len());
                        ids.entry(id.clone()).or_insert(next);
                    }
                }
                map.values().for_each(|c| collect(c, ids));
            }
            Value::Array(items) => items.iter().for_each(|c| collect(c, ids)),
            _ => {}
        }
    }
    fn replace(v: &mut Value, ids: &HashMap<String, String>) {
        match v {
            Value::String(s) => {
                if let Some(to) = ids.get(s) {
                    *s = to.clone();
                }
            }
            Value::Object(map) => map.values_mut().for_each(|c| replace(c, ids)),
            Value::Array(items) => items.iter_mut().for_each(|c| replace(c, ids)),
            _ => {}
        }
    }
    let mut ids = HashMap::new();
    collect(doc, &mut ids);
    replace(doc, &ids);
}

/// The static export's snapshot of every page of `files`, normalized.
fn static_snapshot(root: &Path, files: &[(&str, &str)], all_pages: bool) -> Value {
    let store = graph(&root.join("graph"), files);
    let output = root.join("output");
    fs::create_dir_all(&output).unwrap();
    publish_live(&store, &output, "Site", all_pages, &bundle()).unwrap();
    store.close();
    let mut snapshot: Value =
        serde_json::from_slice(&fs::read(output.join("site/app/snapshot.json")).unwrap()).unwrap();
    snapshot["exported_at"] = Value::String(String::new());
    normalize(&mut snapshot);
    snapshot
}

const CONFIG: &str = "{:file/name-format :triple-lowbar}\n";

/// D1–D5, D7b and the permalink cases (census Q13 map), default formats.
const CASES: &[(&str, &str)] = &[
    ("logseq/config.edn", CONFIG),
    // D1: an empty file named like an alias opens the alias owner.
    ("pages/Machine Learning.md", "alias:: ML\n\n- ML body\n"),
    ("pages/ML.md", "- \n"),
    // D2: a date spelling opens the day's journal; D4: a journal's alias.
    (
        "journals/2026_10_26.md",
        "alias:: launch day\n\n- Launch journal body\n",
    ),
    // D7b: an empty journal file is not a content day.
    ("journals/2026_10_27.md", ""),
    // D3: a namespace parent with no file of its own.
    ("pages/n___y.md", "- namespace child body\n"),
    // D4: an alias two pages declare opens the first owner by path.
    ("pages/Alpha.md", "alias:: Shared\n\n- alpha body\n"),
    ("pages/Beta.md", "alias:: Shared\n\n- beta body\n"),
    // D5: page B declares page A's name: A's header lists B.
    ("pages/Group A.md", "- group a body\n"),
    ("pages/Group B.md", "alias:: Group A\n\n- group b body\n"),
    (
        "pages/Links.md",
        "- [[ML]] [[2026-10-26]] [[Shared]] [[launch day]] [[Group A]]\n",
    ),
];

/// D9: a configured journal title format.
const FORMATS: &[(&str, &str)] = &[
    (
        "logseq/config.edn",
        "{:journal/page-title-format \"dd.MM.yyyy\"}\n",
    ),
    ("journals/2026_10_26.md", "- formatted journal body\n"),
    ("pages/Links.md", "- [[26.10.2026]]\n"),
];

#[test]
fn published_answer_fixtures_match_the_writer() {
    let _integration_guard = crate::integration_lock();
    let write = std::env::var("TINE_PUBLISHED_ANSWERS_FIXTURES").as_deref() == Ok("write");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/published-answers");
    for (case, files) in [("cases", CASES), ("formats", FORMATS)] {
        let root = tempfile::tempdir().unwrap();
        let snapshot = static_snapshot(root.path(), files, true);
        let text = serde_json::to_string_pretty(&snapshot).unwrap() + "\n";
        let path = dir.join(format!("{case}.json"));
        if write {
            fs::create_dir_all(&dir).unwrap();
            fs::write(&path, &text).unwrap();
        }
        assert_eq!(
            fs::read_to_string(&path).unwrap_or_default(),
            text,
            "{case}: the writer drifted from the viewer fixture; rerun with TINE_PUBLISHED_ANSWERS_FIXTURES=write and review"
        );
    }
}

/// Exported documents shared by both closure graphs: `public:: true` pages
/// for the static export, all selected by name for quick share.
const EXPORTED: &[(&str, &str)] = &[
    ("logseq/config.edn", CONFIG),
    (
        "pages/Topic.md",
        "public:: true\n\n- topic body links [[Alpha]]\n",
    ),
    ("pages/Alpha.md", "public:: true\n\n- alpha body\n"),
    (
        "pages/Gamma.md",
        "public:: true\nalias:: G\n\n- gamma body\n",
    ),
    ("pages/Empty Name.md", ""),
    (
        "pages/Index.md",
        "public:: true\n\n- [[Topic]] [[Empty Name]] [[G]] [[Alpha]]\n",
    ),
];

/// Private pages of graph A: unrelated to the exported ones.
const PRIVATE_A: &[(&str, &str)] = &[("pages/Unrelated.md", "- PRIVATE-UNRELATED\n")];

/// Private pages of graph B: a second claimant of an exported name, an alias
/// declaration bridging two exported pages' groups, and a declarer of the
/// exported empty file's name. Each would change a live answer of the whole
/// graph; none may change the published one.
const PRIVATE_B: &[(&str, &str)] = &[
    (
        "pages/Other.md",
        "title:: Topic\npublic:: false\n\n- PRIVATE-CLAIMANT\n",
    ),
    (
        "pages/Bridge.md",
        "alias:: Alpha, Gamma\n\n- PRIVATE-BRIDGE\n",
    ),
    (
        "pages/Declarer.md",
        "alias:: Empty Name\n\n- PRIVATE-DECLARER\n",
    ),
];

const PRIVATE_MARKERS: &[&str] = &[
    "PRIVATE-",
    "Unrelated",
    "unrelated",
    "Other.md",
    "Bridge",
    "bridge",
    "Declarer",
    "declarer",
];

fn files(private: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    EXPORTED.iter().chain(private).copied().collect()
}

/// The whole graph B's own answers differ where the private pages touch the
/// exported ones, so the closure test is not vacuous.
fn assert_private_pages_matter(store: &Store) {
    let view = store.whole_graph().unwrap();
    match view.resolve("Topic", false) {
        tine_store::Resolved::Existing { others, .. } => assert!(!others.is_empty()),
        other => panic!("Topic resolves to {other:?}"),
    }
    let group = tine_graph_features::inventory_wire::alias_group_wire(&view, "Alpha");
    assert!(group.pages.iter().any(|name| name == "Gamma"), "{group:?}");
    let inventory = view.inventory(tine_store::InventoryScope::All);
    let empty = inventory
        .0
        .iter()
        .find(|row| row.name == "Empty Name")
        .unwrap();
    assert!(empty.empty_alias, "{empty:?}");
}

fn share_snapshot(root: &Path, files: &[(&str, &str)]) -> Value {
    let store = graph(&root.join("graph"), files);
    if files.iter().any(|(rel, _)| *rel == "pages/Bridge.md") {
        assert_private_pages_matter(&store);
    }
    let scratch = root.join("scratch");
    fs::create_dir_all(&scratch).unwrap();
    let request = QueryExportRequest {
        argument: "@page and name = 'Topic' or name = 'Alpha' or name = 'Gamma' or name = 'Empty Name' or name = 'Index'".into(),
        dialect: QueryTextDialect::MacroTql,
        properties: vec![],
        current_page: None,
        name: "Closure".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: true,
    };
    let prepared =
        quick_share::prepare(&store, &request, "2026-11-04T00:00:00Z", &scratch).unwrap();
    store.close();
    let doc =
        quick_share::gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap();
    let mut doc: Value = serde_json::from_slice(&doc).unwrap();
    normalize(&mut doc);
    doc["snapshot"].take()
}

fn assert_closed(route: &str, a: &Value, b: &Value) {
    let answers_a = serde_json::to_vec(&a["answers"]).unwrap();
    let answers_b = serde_json::to_vec(&b["answers"]).unwrap();
    assert!(a["answers"]["v"] == 1, "{route}: answers written");
    assert_eq!(
        String::from_utf8(answers_a).unwrap(),
        String::from_utf8(answers_b).unwrap(),
        "{route}: private pages changed the published answers"
    );
    assert_eq!(a, b, "{route}: private pages changed the snapshot");
    let text = b.to_string();
    for marker in PRIVATE_MARKERS {
        assert!(
            !text.contains(marker),
            "{route}: the snapshot names {marker:?}"
        );
    }
}

#[test]
fn answers_are_closed_by_the_exported_documents() {
    let _integration_guard = crate::integration_lock();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (share_a, share_b) = (
        share_snapshot(a.path(), &files(PRIVATE_A)),
        share_snapshot(b.path(), &files(PRIVATE_B)),
    );
    assert_closed("quick share", &share_a, &share_b);
    // The exported empty file stays itself: no exported page declares its
    // name (the private declarer would make it an empty alias live).
    let rows = share_b["answers"]["inventory"].as_array().unwrap();
    let empty = rows.iter().find(|row| row["name"] == "Empty Name").unwrap();
    assert!(empty.get("link").is_none() && empty.get("empty_alias").is_none());
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (static_a, static_b) = (
        static_snapshot(a.path(), &files(PRIVATE_A), false),
        static_snapshot(b.path(), &files(PRIVATE_B), false),
    );
    assert_closed("static export", &static_a, &static_b);
}
