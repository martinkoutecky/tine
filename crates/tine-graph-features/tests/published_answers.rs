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
//!   byte-identical `answers`, and byte-identical files everywhere else (every
//!   HTML and JSON file of the static exports, the whole share document),
//!   through quick share, the live and query static exports and the CLI's
//!   static site.
//! - `a_query_export_names_only_an_exported_host`: the home-query record's
//!   execution context names its host only when the host is exported.
//! - `a_synthetic_home_never_shadows_an_answered_name`: a multi-page
//!   publication's home named like a page or alias (single- or multi-owner)
//!   is renamed by the static export and refused by quick share.
//! - `not_namespace_is_live_only`: the live inventory row of `./x` or a URL
//!   says `not_namespace` (model round 10, S15); answers v1 is frozen, so the
//!   published row does not.
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::publish_query::{
    plan_query, publish_live, publish_query, publish_static, QueryExportRequest,
};
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

#[test]
fn date_alias_group_wire_keeps_an_unclaimed_day_spelling() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    let store = graph(
        dir.path(),
        &[
            (
                "logseq/config.edn",
                "{:journal/page-title-format \"yyyy_MM_dd\"}\n",
            ),
            ("pages/2026_10_23.md", "- unrelated ordinary file\n"),
            ("pages/Owner.md", "alias:: 2026-10-23\n\n- owner\n"),
        ],
    );
    let view = store.whole_graph().unwrap();
    let group = tine_graph_features::inventory_wire::alias_group_wire(&view, "Owner");
    assert!(
        group.pages.is_empty(),
        "unrelated file is not a related page: {group:?}"
    );
    assert_eq!(group.aliases, ["2026-10-23"]);
    assert_eq!(view.link_target(&group.aliases[0]).name, "Owner");
    assert_eq!(view.link_target("2026_10_23").name, "2026_10_23");
    store.close();
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

/// A snapshot as written, with its export time blanked and runtime ids
/// renumbered.
fn read_snapshot(path: &Path) -> Value {
    let mut snapshot: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    snapshot["exported_at"] = Value::String(String::new());
    normalize(&mut snapshot);
    snapshot
}

/// Every file under `dir` by relative path, the snapshot normalized as in
/// [`read_snapshot`].
fn site_files(dir: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
                continue;
            }
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let text = if rel.ends_with("snapshot.json") {
                read_snapshot(&path).to_string()
            } else {
                String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned()
            };
            out.insert(rel, text);
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// The static export's snapshot of every page of `files`, normalized.
fn static_snapshot(root: &Path, files: &[(&str, &str)], all_pages: bool) -> Value {
    static_export(root, files, all_pages).0
}

/// The live static export of `files`: its snapshot and every file it wrote.
fn static_export(
    root: &Path,
    files: &[(&str, &str)],
    all_pages: bool,
) -> (Value, BTreeMap<String, String>) {
    let store = graph(&root.join("graph"), files);
    let output = root.join("output");
    fs::create_dir_all(&output).unwrap();
    publish_live(&store, &output, "Site", all_pages, true, &bundle()).unwrap();
    store.close();
    let site = output.join("site");
    (
        read_snapshot(&site.join("app/snapshot.json")),
        site_files(&site),
    )
}

/// The CLI's static site (`publish_static`, HTML only) of `files`.
fn static_site(root: &Path, files: &[(&str, &str)]) -> BTreeMap<String, String> {
    let store = graph(&root.join("graph"), files);
    let output = root.join("output");
    fs::create_dir_all(&output).unwrap();
    let receipt = publish_static(&store, &output, "Site", false, true).unwrap();
    store.close();
    site_files(Path::new(&receipt.path))
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
    // `cases.json` remains a frozen v1 reader fixture from before R13-C4.
    for (case, files) in [("cases-real-file-first", CASES), ("formats", FORMATS)] {
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
        "public:: true\n\n- [[Topic]] [[Empty Name]] [[G]] [[Alpha]]\n- {{tine-query @page}}\n",
    ),
];

/// Private pages of graph A: unrelated to the exported ones.
const PRIVATE_A: &[(&str, &str)] = &[
    ("pages/Unrelated.md", "- PRIVATE-UNRELATED\n"),
    ("pages/PRIVATE-HOST.md", "- PRIVATE-HOST host\n"),
];

/// Private pages of graph B: a later-path claimant of an exported name, an alias
/// declaration bridging two exported pages' groups, and a declarer of the
/// exported empty file's name. Each would change a live answer of the whole
/// graph; none may change the published one.
const PRIVATE_B: &[(&str, &str)] = &[
    (
        "pages/Z-Other.md",
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
    ("pages/PRIVATE-HOST.md", "- PRIVATE-HOST host\n"),
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
    assert!(!empty.empty_alias && empty.link.is_none(), "{empty:?}");
}

/// The request selecting the exported pages by name; `host` is the page the
/// query runs on.
fn closure_request(name: &str, host: Option<&str>) -> QueryExportRequest {
    QueryExportRequest {
        argument: "@page and name = 'Topic' or name = 'Alpha' or name = 'Gamma' or name = 'Empty Name' or name = 'Index'".into(),
        dialect: QueryTextDialect::MacroTql,
        properties: vec![],
        current_page: host.map(str::to_owned),
        name: name.into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: true,
        include_comments: None,
    }
}

/// The whole share document (normalized) of `request` over `files`, or the
/// refusal.
fn share_document(
    root: &Path,
    files: &[(&str, &str)],
    request: &QueryExportRequest,
) -> Result<Value, String> {
    let store = graph(&root.join("graph"), files);
    if files.iter().any(|(rel, _)| *rel == "pages/Bridge.md") {
        assert_private_pages_matter(&store);
    }
    let scratch = root.join("scratch");
    fs::create_dir_all(&scratch).unwrap();
    let prepared = quick_share::prepare(&store, request, "2026-11-04T00:00:00Z", &scratch);
    store.close();
    let prepared = prepared.map_err(|error| error.to_string())?;
    assert_eq!(
        fs::read_dir(&scratch).unwrap().count(),
        0,
        "the build graph is removed"
    );
    let doc =
        quick_share::gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap();
    let mut doc: Value = serde_json::from_slice(&doc).unwrap();
    normalize(&mut doc);
    Ok(doc)
}

/// The query export of `request` over `files`: its snapshot and every file.
fn query_export(
    root: &Path,
    files: &[(&str, &str)],
    request: &QueryExportRequest,
) -> (Value, BTreeMap<String, String>) {
    let store = graph(&root.join("graph"), files);
    let plan = plan_query(&store, request).unwrap();
    let receipt = publish_query(&store, request, &plan.fingerprint, &bundle()).unwrap();
    store.close();
    let site = Path::new(&receipt.path);
    (
        read_snapshot(&site.join("app/snapshot.json")),
        site_files(site),
    )
}

/// Every emitted file is identical across the two graphs and names no private
/// page.
fn assert_files_closed(route: &str, a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) {
    assert!(a.keys().any(|path| path.ends_with(".html")) || a.contains_key("share.json"));
    assert_eq!(
        a.keys().collect::<Vec<_>>(),
        b.keys().collect::<Vec<_>>(),
        "{route}: private pages changed the emitted file set"
    );
    for (path, text) in a {
        assert_eq!(text, &b[path], "{route}: private pages changed {path}");
        for marker in PRIVATE_MARKERS {
            assert!(!text.contains(marker), "{route}: {path} names {marker:?}");
        }
    }
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
    let request = closure_request("Closure", None);
    let (doc_a, doc_b) = (
        share_document(a.path(), &files(PRIVATE_A), &request).unwrap(),
        share_document(b.path(), &files(PRIVATE_B), &request).unwrap(),
    );
    assert_closed("quick share", &doc_a["snapshot"], &doc_b["snapshot"]);
    let whole = |doc: &Value| BTreeMap::from([("share.json".to_owned(), doc.to_string())]);
    assert_files_closed("quick share", &whole(&doc_a), &whole(&doc_b));
    // The exported empty file stays itself: no exported page declares its
    // name (the private declarer would make it an empty alias live).
    let rows = doc_b["snapshot"]["answers"]["inventory"]
        .as_array()
        .unwrap();
    let empty = rows.iter().find(|row| row["name"] == "Empty Name").unwrap();
    assert!(empty.get("link").is_none() && empty.get("empty_alias").is_none());

    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ((static_a, site_a), (static_b, site_b)) = (
        static_export(a.path(), &files(PRIVATE_A), false),
        static_export(b.path(), &files(PRIVATE_B), false),
    );
    assert_closed("static export", &static_a, &static_b);
    assert_files_closed("static export", &site_a, &site_b);
    // The query on Index is rendered in the HTML fallback, so the comparison
    // is not vacuous: over the whole graph its "results on non-public pages
    // omitted" count differed between graphs A and B. Over the build graph it
    // counts only what the exported documents name without a file of their
    // own (here `[[Empty Name]]`, not public in this export).
    let index = site_a
        .iter()
        .find(|(path, _)| path.starts_with("index"))
        .map(|(_, text)| text)
        .unwrap();
    assert!(index.contains("query-count"), "{index}");
    assert!(
        index.contains("1 result on non-public pages omitted"),
        "{index}"
    );

    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (site_a, site_b) = (
        static_site(a.path(), &files(PRIVATE_A)),
        static_site(b.path(), &files(PRIVATE_B)),
    );
    assert_files_closed("static site", &site_a, &site_b);

    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let request = closure_request("Closure", Some("PRIVATE-HOST"));
    let ((query_a, files_a), (query_b, files_b)) = (
        query_export(a.path(), &files(PRIVATE_A), &request),
        query_export(b.path(), &files(PRIVATE_B), &request),
    );
    assert_closed("query export", &query_a, &query_b);
    assert_files_closed("query export", &files_a, &files_b);
}

#[test]
fn a_query_export_names_only_an_exported_host() {
    let _integration_guard = crate::integration_lock();
    let context = |host: &str| {
        let root = tempfile::tempdir().unwrap();
        let (snapshot, _) = query_export(
            root.path(),
            &files(PRIVATE_A),
            &closure_request("Closure", Some(host)),
        );
        let record = snapshot["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["host"] == "Closure")
            .cloned()
            .unwrap();
        record["executed_context"].clone()
    };
    assert_eq!(context("PRIVATE-HOST"), serde_json::json!({}));
    // An exported host is named by its exported (canonical) name.
    assert_eq!(
        context("index"),
        serde_json::json!({ "current_page": "Index" })
    );
}

/// Pages for the home-clash cases: a two-owner alias, a one-owner alias and a
/// page linking both.
const CLASH: &[(&str, &str)] = &[
    ("logseq/config.edn", CONFIG),
    ("pages/Alpha.md", "alias:: Shared\n\n- alpha body\n"),
    ("pages/Beta.md", "alias:: Shared\n\n- beta body\n"),
    ("pages/Gamma.md", "alias:: G\n\n- gamma body\n"),
    (
        "pages/Links.md",
        "- [[Shared]] [[G]] [[Nowhere]] [[2026-10-26]]\n",
    ),
];

fn clash_request(name: &str) -> QueryExportRequest {
    QueryExportRequest {
        argument: "@page and name = 'Alpha' or name = 'Beta' or name = 'Gamma' or name = 'Links'"
            .into(),
        dialect: QueryTextDialect::MacroTql,
        properties: vec![],
        current_page: None,
        name: name.into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: false,
        include_comments: None,
    }
}

fn row<'a>(snapshot: &'a Value, name: &str) -> &'a Value {
    snapshot["answers"]["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name && row["is_journal"] == false)
        .unwrap_or_else(|| panic!("no row {name}"))
}

#[test]
fn a_synthetic_home_never_shadows_an_answered_name() {
    let _integration_guard = crate::integration_lock();
    for (name, renamed) in [
        ("Shared", "Shared (export)"),
        ("g", "g (export)"),
        ("Links", "Links (export)"),
        ("2026-10-26", "2026-10-26 (export)"),
    ] {
        // Static query export: the home is renamed; the alias row stays the
        // build graph's answer.
        let root = tempfile::tempdir().unwrap();
        let (snapshot, _) = query_export(root.path(), CLASH, &clash_request(name));
        assert_eq!(snapshot["home"], renamed);
        assert_eq!(row(&snapshot, renamed)["target"]["id"], "");
        let shadowed = row(&snapshot, if name == "g" { "G" } else { name });
        let expected = match name {
            "Shared" => {
                serde_json::json!({ "kind": "alias", "owners": ["pages/Alpha.md", "pages/Beta.md"] })
            }
            "g" => serde_json::json!({ "kind": "alias", "owners": ["pages/Gamma.md"] }),
            "2026-10-26" => {
                assert_eq!(
                    shadowed["link"],
                    serde_json::json!({
                        "kind": "journal", "name": "Oct 26th, 2026"
                    })
                );
                serde_json::json!({ "kind": "absent", "id": "pages/2026-10-26.md" })
            }
            _ => serde_json::json!({ "kind": "existing", "id": "pages/Links.md", "others": [] }),
        };
        assert_eq!(shadowed["target"], expected, "{name}: {shadowed}");
        // Quick share: the share name is refused.
        let root = tempfile::tempdir().unwrap();
        let refused = share_document(root.path(), CLASH, &clash_request(name)).unwrap_err();
        assert!(refused.contains("already called"), "{name}: {refused}");
        assert_eq!(
            fs::read_dir(root.path().join("scratch")).unwrap().count(),
            0,
            "{name}: the build graph is removed after a refusal"
        );
    }
    // A reference-only name answers nothing: the home may take it.
    let root = tempfile::tempdir().unwrap();
    let doc = share_document(root.path(), CLASH, &clash_request("Nowhere")).unwrap();
    assert_eq!(doc["snapshot"]["home"], "Nowhere");
    assert_eq!(row(&doc["snapshot"], "Nowhere")["target"]["id"], "");
    let root = tempfile::tempdir().unwrap();
    let (snapshot, _) = query_export(root.path(), CLASH, &clash_request("Nowhere"));
    assert_eq!(snapshot["home"], "Nowhere");
}

#[test]
fn not_namespace_is_live_only() {
    let _integration_guard = crate::integration_lock();
    let files = [
        (
            "pages/dot.md",
            "title:: .\n\n- [[./x]] [[a/b]] [[https://a.b/c]]\n",
        ),
        ("pages/dotx.md", "title:: ./x\n\n- x\n"),
        ("pages/owner.md", "alias:: old/sub\n\n- owner\n"),
    ];
    let root = tempfile::tempdir().unwrap();
    let store = graph(&root.path().join("live"), &files);
    let view = store.whole_graph().unwrap();
    let live: Vec<_> = view
        .inventory(tine_store::InventoryScope::All)
        .0
        .iter()
        .map(tine_graph_features::inventory_wire::entry_wire)
        .map(|row| (row.name, row.not_namespace))
        .collect();
    for (name, flagged) in [
        ("./x", true),
        ("https://a.b/c", true),
        ("a/b", false),
        (".", false),
    ] {
        assert!(
            live.contains(&(name.to_owned(), flagged)),
            "{name}: {live:?}"
        );
    }
    let alias = view
        .inventory(tine_store::InventoryScope::All)
        .0
        .iter()
        .find(|row| row.name == "old/sub")
        .cloned()
        .unwrap();
    assert!(tine_graph_features::inventory_wire::entry_wire(&alias).namespace_excluded);
    store.close();
    let snapshot = static_snapshot(&root.path().join("site"), &files, true);
    for name in ["./x", "https://a.b/c"] {
        assert!(
            row(&snapshot, name).get("not_namespace").is_none(),
            "{name}"
        );
    }
    assert!(row(&snapshot, "old/sub")
        .get("namespace_excluded")
        .is_none());
}

/// GH #881: `tine.breadcrumbs:: false` is presentation only. A published query
/// record's key must not carry it (the renderer leaves it out of its lookup, so
/// a record keyed with it is never found), and a generated query home keeps it
/// so the export hides breadcrumbs like the source block.
#[test]
fn breadcrumb_switch_is_not_part_of_a_published_query_key() {
    let _integration_guard = crate::integration_lock();
    let files: &[(&str, &str)] = &[(
        "pages/Shop.md",
        "- {{query (task TODO)}}\n  tine.breadcrumbs:: false\n- TODO Buy apples\n",
    )];
    let keyed_with_breadcrumbs = |snapshot: &Value| {
        snapshot["queries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| {
                record["properties"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|pair| pair[0] == "tine.breadcrumbs")
            })
    };
    let dir = tempfile::tempdir().unwrap();
    let snapshot = static_snapshot(dir.path(), files, true);
    assert!(!snapshot["queries"].as_array().unwrap().is_empty());
    assert!(!keyed_with_breadcrumbs(&snapshot));

    let dir = tempfile::tempdir().unwrap();
    let mut request = closure_request("Shopping list", None);
    request.argument = "(task TODO)".into();
    request.dialect = QueryTextDialect::MacroQuery;
    request.properties = vec![("tine.breadcrumbs".into(), "false".into())];
    let (snapshot, _) = query_export(dir.path(), files, &request);
    assert!(!keyed_with_breadcrumbs(&snapshot));
    assert!(snapshot.to_string().contains("tine.breadcrumbs:: false"));
}
