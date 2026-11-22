//! Closure and identity through the publication writer and the browser graph loader.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::publish_query::{
    plan_query, publish_live, publish_query, publish_static, QueryExportRequest,
};
use tine_graph_features::quick_share;
use tine_store::{Resolved, Store};

use crate::quick_share::snapshot_graph;

pub(super) fn parsed_query(
    core: &tine_store::WholeGraph,
    argument: &str,
    properties: &[(String, String)],
) -> tine_core::query::wire_parse::ParsedQuery {
    let registry = core
        .query_ir(tine_store::IrRequest::Registry, &Default::default())
        .unwrap()
        .unwrap();
    let tine_store::IrAnswer::Registry(registry) = registry else {
        panic!("expected registry")
    };
    tine_core::query::wire_parse::parse_query_pair(
        argument,
        QueryTextDialect::MacroQuery,
        properties,
        &registry,
    )
}

pub(super) fn run_query(
    core: &tine_store::WholeGraph,
    parsed: &tine_core::query::wire_parse::ParsedQuery,
    current_page: Option<&str>,
) -> tine_core::query::ir::QueryResult {
    let view = tine_core::query::wire_parse::anchored_view(parsed, parsed.query.anchor);
    let context = tine_core::query::ir::ExecutionContext {
        current_page: current_page.map(str::to_owned),
    };
    let result = core
        .query_ir(
            tine_store::IrRequest::Run {
                query: &parsed.query,
                view: &view,
                context: &context,
            },
            &Default::default(),
        )
        .unwrap()
        .unwrap();
    let tine_store::IrAnswer::Result(result) = result else {
        panic!("expected query answer")
    };
    *result
}

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

/// A date alias with no file of its day stays an alias spelling; once a file
/// holds the day (here the date-named `pages/2026_10_23.md`, CM-DAYFILE,
/// D-20261111-CMR14) the alias names that real day page, which opens itself
/// (CM SPEC §2.3). (Superseded: before CM step 2 the pages file was an
/// unrelated ordinary page and the alias stayed unclaimed beside it.)
#[test]
fn a_date_alias_group_wire_names_the_day_and_its_file_once_one_holds_it() {
    let _integration_guard = crate::integration_lock();
    for day_file in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut files = vec![
            (
                "logseq/config.edn",
                "{:journal/page-title-format \"yyyy_MM_dd\"}\n",
            ),
            ("pages/Owner.md", "alias:: 2026-10-23\n\n- owner\n"),
        ];
        if day_file {
            files.push(("pages/2026_10_23.md", "- the day\n"));
        }
        let store = graph(dir.path(), &files);
        let view = store.whole_graph().unwrap();
        let group = tine_graph_features::inventory_wire::alias_group_wire(&view, "Owner");
        if day_file {
            assert!(group.aliases.is_empty(), "{group:?}");
            assert_eq!(group.pages, ["2026_10_23"], "{group:?}");
            assert_eq!(view.link_target("2026-10-23").name, "2026_10_23");
            assert!(view.link_target("2026-10-23").is_journal);
        } else {
            // The alias member is the day, named by its title.
            assert!(group.pages.is_empty(), "{group:?}");
            assert_eq!(group.aliases, ["2026_10_23"]);
            assert_eq!(view.link_target("2026-10-23").name, "Owner");
            assert_eq!(view.link_target("2026_10_23").name, "Owner");
        }
        store.close();
    }
}

/// A snapshot as written, with its export time blanked.
fn read_snapshot(path: &Path) -> Value {
    let mut snapshot: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    snapshot["exported_at"] = Value::String(String::new());
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
        textual_current_page: host.map(str::to_owned),
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
    let prepared = quick_share::prepare(&store, request, "2026-11-04T00:00:00Z");
    store.close();
    let prepared = prepared.map_err(|error| error.to_string())?;
    let doc =
        quick_share::gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap();
    Ok(serde_json::from_slice(&doc).unwrap())
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
    assert_eq!(a["schema"], 2, "{route}: writes source snapshots");
    assert!(a.get("answers").is_none() && a.get("queries").is_none());
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
fn source_snapshots_are_closed_by_the_exported_documents() {
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
    let browser = snapshot_graph(&doc_b["snapshot"]);
    assert!(
        matches!(browser.resolve("Empty Name", false), Resolved::Existing { id, .. }
        if id.as_str() == "pages/Empty Name.md")
    );
    assert!(
        matches!(browser.resolve("Topic", false), Resolved::Existing { others, .. }
        if others.is_empty())
    );

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
        textual_current_page: None,
        name: name.into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: false,
        include_comments: None,
    }
}

#[test]
fn a_generated_home_never_shadows_an_answered_name() {
    let _integration_guard = crate::integration_lock();
    for (name, renamed) in [
        ("Shared", "Shared (export)"),
        ("g", "g (export)"),
        ("Links", "Links (export)"),
        ("2026-10-26", "2026-10-26 (export)"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (snapshot, _) = query_export(root.path(), CLASH, &clash_request(name));
        let graph = snapshot_graph(&snapshot);
        assert_eq!(graph.config().default_home.as_deref(), Some(renamed));
        assert!(
            matches!(graph.resolve(renamed, false), Resolved::Existing { id, .. } if !id.as_str().is_empty())
        );
        let target = graph.link_target(name);
        let expected = match name {
            "Shared" => "Alpha",
            "g" => "Gamma",
            "2026-10-26" => "Oct 26th, 2026",
            _ => name,
        };
        assert_eq!(target.name, expected);
        let root = tempfile::tempdir().unwrap();
        let refused = share_document(root.path(), CLASH, &clash_request(name)).unwrap_err();
        assert!(
            refused.contains("give the share another name"),
            "{name}: {refused}"
        );
    }
    let root = tempfile::tempdir().unwrap();
    let doc = share_document(root.path(), CLASH, &clash_request("Nowhere")).unwrap();
    let graph = snapshot_graph(&doc["snapshot"]);
    assert_eq!(graph.config().default_home.as_deref(), Some("Nowhere"));
    assert!(matches!(
        graph.resolve("Nowhere", false),
        Resolved::Existing { .. }
    ));
    let root = tempfile::tempdir().unwrap();
    let (snapshot, _) = query_export(root.path(), CLASH, &clash_request("Nowhere"));
    assert_eq!(
        snapshot_graph(&snapshot).config().default_home.as_deref(),
        Some("Nowhere")
    );
}

#[test]
fn the_query_home_keeps_its_authored_argument_and_presentation() {
    let _integration_guard = crate::integration_lock();
    let files = [
        (
            "pages/Shop.md",
            "- TODO Buy apples\n- TODO Exclude [[Shopping list]]\n",
        ),
        ("pages/PRIVATE-HOST.md", "- private context\n"),
    ];
    let mut request = closure_request("Shopping list", Some("PRIVATE-HOST"));
    request.argument = "(and (task TODO) (not <% current page %>))".into();
    request.dialect = QueryTextDialect::MacroQuery;
    request.properties = vec![("tine.breadcrumbs".into(), "false".into())];
    let dir = tempfile::tempdir().unwrap();
    let store = graph(dir.path(), &files);
    let plan = plan_query(&store, &request).unwrap();
    assert_eq!(plan.row_count, 2);
    assert_eq!(
        serde_json::to_value(&plan).unwrap()["usesCurrentPage"],
        true
    );
    store.close();
    let dir = tempfile::tempdir().unwrap();
    let (snapshot, _) = query_export(dir.path(), &files, &request);
    assert!(!snapshot.to_string().contains("PRIVATE-HOST"));
    assert!(snapshot.get("query_home").is_none());
    let core = snapshot_graph(&snapshot);
    let corpus = core.corpus(None, true);
    let home = corpus
        .pages
        .iter()
        .find(|page| page.name == "Shopping list")
        .unwrap();
    let query = &home.document.roots[0];
    assert_eq!(home.document.roots.len(), 1);
    assert!(query
        .raw()
        .contains("{{query (and (task TODO) (not <% current page %>))}}"));
    assert_eq!(query.property("tine.breadcrumbs").as_deref(), Some("false"));
    let result = run_query(
        &core,
        &parsed_query(
            &core,
            "(and (task TODO) (not [[Shopping list]]))",
            &query.properties(),
        ),
        Some(&home.name),
    );
    let live = serde_json::to_string(&result.rows).unwrap();
    assert_eq!(result.total, 1);
    assert!(live.contains("Buy apples") && !live.contains("Exclude"));
}

#[test]
fn export_review_reports_the_dependency_of_the_effective_inputs() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    let store = graph(
        dir.path(),
        &[
            ("pages/Focused.md", "- focused\n"),
            ("pages/Literal.md", "- first literal\n- second literal\n"),
        ],
    );
    let mut request = closure_request("Review", Some("Focused"));
    request.dialect = QueryTextDialect::MacroQuery;
    let vector =
        "[:find (pull ?b [*]) :in $ ?p :where [?page :block/name ?p] [?b :block/page ?page]]";
    for (inputs, count, depends) in [
        ("; comment\n (#_ :ignored :current-page)", 1, true),
        (r#"[#_ :current-page "literal"]"#, 2, false),
    ] {
        request.argument = format!("{{:query {vector} :inputs {inputs}}}");
        let plan = plan_query(&store, &request).unwrap();
        assert_eq!(plan.row_count, count);
        assert_eq!(plan.uses_current_page, depends);
    }
    store.close();
}

#[test]
fn multiline_journal_name_uses_ordinary_home_collision_rules() {
    let _integration_guard = crate::integration_lock();
    for newline in ["\n", "\r\n", "\r"] {
        let name = format!("Nov{newline}22nd, 2026");
        let request = clash_request(&name);
        let root = tempfile::tempdir().unwrap();
        let (snapshot, _) = query_export(root.path(), CLASH, &request);
        assert_eq!(snapshot["name"], name.as_str());
        let graph = snapshot_graph(&snapshot);
        assert_eq!(
            graph.config().default_home.as_deref(),
            Some("Nov 22nd, 2026 (export)")
        );
        let home = graph.corpus(Some("Nov 22nd, 2026 (export)"), true);
        assert_eq!(home.pages[0].kind, tine_core::model::PageKind::Page);
        let root = tempfile::tempdir().unwrap();
        let refused = share_document(root.path(), CLASH, &request).unwrap_err();
        assert!(refused.contains("journal date"), "{refused}");
    }
}
