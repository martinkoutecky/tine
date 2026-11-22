//! Quick-share source closure, envelope integrity, and viewer fixtures.
//!
//! `TINE_QUICK_SHARE_FIXTURES=write` writes a fixture case that does not exist
//! yet and refreshes every `writer.json`; a frozen `tine-share.bin`,
//! `document.json` or `link.txt` is never rewritten.
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::publish_query::QueryExportRequest;
use tine_graph_features::quick_share::{
    self, b64u, b64u_decode, gunzip_bounded, open, seal, OpenError,
};
use tine_store::Store;

const CREATED_AT: &str = "2026-10-17T00:00:00Z";
const KEY: [u8; 32] = *b"tine-quick-share-fixture-key-32b";
const SHARE_ID: [u8; 16] = *b"fixture-share-id";
const NONCE: [u8; 12] = *b"fixture-nonc";
const LOGIN: &str = "tine-fixture";
const GIST: &str = "0123456789abcdef0123";

fn base() -> PathBuf {
    crate::test_support::fresh_dir("tine-quick-share-")
}

/// The fixture corpus: the cases the contract pins (aliases, two identical
/// macros in one block, a query in page properties, page-property backlinks,
/// escaped and duplicate property keys, `id::` uuids with reference counts,
/// an Org page, a journal, a Unicode name, an empty page), plus a private, an
/// outside and a hidden page that must never reach a share.
fn corpus(root: &Path) -> Store {
    let graph = root.join("graph");
    for dir in ["pages", "journals", "logseq", "hidden"] {
        fs::create_dir_all(graph.join(dir)).unwrap();
    }
    let files: &[(&str, &str)] = &[
        (
            "logseq/config.edn",
            "{:file/name-format :triple-lowbar\n :hidden [\"hidden\"]\n :start-of-week 1\n :favorites [\"PRIVATE-FAVORITE\"]\n :macros {:greet ; preserve this gap\n \"Hello $1\" :unused \"UNUSED-MACRO-SECRET\"}}\n",
        ),
        (
            "pages/Project.md",
            "alias:: Proj\nstatus:: active\n\n- TODO first task\n  id:: 6512a000-0000-4000-8000-000000000001\n- Links to [[Notes ü]] and ((6512a000-0000-4000-8000-000000000002))\n- {{query (task TODO)}} and again {{query (task TODO)}}\n- odd keys\n  a.b:: dotted\n  dup:: one\n  dup:: two\n",
        ),
        (
            "pages/Notes ü.md",
            "type:: note\nrelated:: [[Project]]\n\n- block with an id\n  id:: 6512a000-0000-4000-8000-000000000002\n- refers ((6512a000-0000-4000-8000-000000000001)) and [[Proj]]\n",
        ),
        (
            "pages/Dashboard.md",
            "description:: {{query (page-property type note)}}\n\n- tasks here: {{query (task TODO)}}\n",
        ),
        ("pages/Empty.md", ""),
        (
            "pages/Org page.org",
            "#+title: Org page\n* TODO org task\n* see [[Project]]\n",
        ),
        ("journals/2026_10_17.md", "- journal note on [[Project]]\n"),
        (
            "pages/Secret.md",
            "public:: false\n\n- SECRET-MARKER links [[Project]]\n  secret-prop:: zzz\n",
        ),
        (
            "pages/Outside.md",
            "- OUTSIDE-MARKER links [[Project]]\n  outside-only-prop:: OUTSIDE-VALUE\n- TODO OUTSIDE-TASK\n",
        ),
        ("hidden/Hidden.md", "- HIDDEN-MARKER links [[Project]]\n"),
        ("pages/Comments.md", "- public body sentinel\n  - private comment sentinel\n    tine.quote:: public body sentinel\n    - private reply sentinel\n"),
        ("pages/Configured.md", "- {{greet reader}}\n- `{{unused}}` is example code\n"),
    ];
    for (rel, body) in files {
        fs::write(graph.join(rel), body).unwrap();
    }
    let store = Store::open(&graph, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    store
}

fn request(argument: &str, name: &str) -> QueryExportRequest {
    QueryExportRequest {
        argument: argument.into(),
        dialect: QueryTextDialect::MacroTql,
        properties: vec![],
        current_page: None,
        name: name.into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: true,
        include_comments: None,
    }
}

/// The corpus's one macro written in page properties (`Dashboard`'s
/// `description::`), and a selection that holds it and the page it finds.
const PAGE_PROPERTY_QUERY: &str = "(page-property type note)";
const PAGE_QUERIES: &str = "@page and name = 'Dashboard' or name = 'Notes ü'";

const MANY: &str = "@page and name = 'Project' or name = 'Notes ü' or name = 'Dashboard' or name = 'Empty' or name = 'Org page' or name = 'Oct 17th, 2026' or name = 'Secret' or name = 'Hidden'";

fn document(store: &Store, argument: &str, name: &str) -> (quick_share::SharePreview, Vec<u8>) {
    let prepared = quick_share::prepare(store, &request(argument, name), CREATED_AT).unwrap();
    let doc = gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap();
    (prepared.preview, doc)
}

#[test]
fn quick_share_envelope_round_trips_and_refuses_damage() {
    let _integration_guard = crate::integration_lock();
    let payload = b"the share document";
    let envelope = seal(payload, &KEY, &SHARE_ID, &NONCE);
    assert_eq!(open(&envelope, &KEY).unwrap(), payload);
    // Known answer, checked against Python `cryptography` (HKDF + AESGCM); the
    // browser decoder opens these exact bytes
    // (src/quickShare/quickShare.test.ts).
    assert_eq!(
        envelope,
        "VFNIMQFmaXh0dXJlLXNoYXJlLWlkZml4dHVyZS1ub25jxlR5aY52hv6DfbD-0w-Gt7EoF0a_kyGqrXYGhJDTl1KKHQ"
    );
    let file = b64u_decode(&envelope).unwrap();
    for at in [0usize, 4, 5, 20, 21, 32, 33, file.len() - 1] {
        let mut damaged = file.clone();
        damaged[at] ^= 1;
        let expected = match at {
            0 => OpenError::NotAShare,
            4 => OpenError::NewerEnvelope(0),
            _ => OpenError::Unauthentic,
        };
        assert_eq!(open(&b64u(&damaged), &KEY), Err(expected), "byte {at}");
    }
    let mut other = KEY;
    other[0] ^= 1;
    assert_eq!(open(&envelope, &other), Err(OpenError::Unauthentic));
    assert_eq!(
        open(&format!("{envelope}="), &KEY),
        Err(OpenError::NotBase64url)
    );
    assert_eq!(
        open(&format!("{envelope} "), &KEY),
        Err(OpenError::NotBase64url)
    );
    assert_eq!(open(&envelope[..60], &KEY), Err(OpenError::Truncated));
    // Non-zero trailing bits: the last character of a 2-byte tail differs.
    let short = b64u(&[0xff; 49]);
    let mut bent = short.clone().into_bytes();
    let last = bent.len() - 1;
    bent[last] = if bent[last] == b'w' { b'x' } else { b'w' };
    assert!(b64u_decode(std::str::from_utf8(&bent).unwrap()).is_none());
    let fresh = quick_share::seal_fresh(payload).unwrap();
    assert_eq!(open(&fresh.envelope, &fresh.key).unwrap(), payload);
    assert_ne!(fresh.key, KEY);
    // D-20261018-QS3: the share id is chosen at prepare and spelled into the
    // graph as `pending/<id>`; it must never read as a gist id.
    for _ in 0..256 {
        let id = quick_share::fresh_share_id().unwrap();
        assert!(!quick_share::looks_like_gist_id(&b64u(&id)));
    }
    assert!(quick_share::looks_like_gist_id("0123456789abcdef0123ab"));
    assert!(!quick_share::looks_like_gist_id("0123456789abcdef0123aB"));
    let chosen = *b"fixture-share-id";
    let sealed = quick_share::seal_fresh_as(payload, chosen).unwrap();
    assert_eq!(sealed.share_id, chosen);
    assert_eq!(open(&sealed.envelope, &sealed.key).unwrap(), payload);
}

pub(super) fn snapshot_graph(snapshot: &Value) -> tine_store::WholeGraph {
    assert_eq!(snapshot["schema"], 2);
    tine_store::WholeGraph::from_files(
        snapshot["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| {
                (
                    file["path"].as_str().unwrap().to_owned(),
                    file["text"].as_str().unwrap().as_bytes().to_vec(),
                )
            })
            .collect(),
    )
    .unwrap()
    .0
}

#[test]
fn quick_share_is_closed_to_the_shared_pages() {
    let _integration_guard = crate::integration_lock();
    let root = base();
    let store = corpus(&root);
    let (preview, doc) = document(&store, MANY, "Fixture share");
    let names: Vec<_> = preview.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names.len(), 6, "{names:?}");
    assert!(!names.contains(&"Secret"));
    assert_eq!(preview.left_out.len(), 1);
    assert_eq!(preview.left_out[0].name, "Secret");
    let text = std::str::from_utf8(&doc).unwrap();
    for leak in [
        "SECRET-MARKER",
        "secret-prop",
        "OUTSIDE-MARKER",
        "OUTSIDE-VALUE",
        "outside-only-prop",
        "OUTSIDE-TASK",
        "HIDDEN-MARKER",
        "Outside",
        "Secret",
        "hidden",
        "tine-quick-share-",
        root.to_str().unwrap(),
        "name = 'Project'",
    ] {
        assert!(!text.contains(leak), "the share carries {leak:?}");
    }
    let doc: Value = serde_json::from_slice(&doc).unwrap();
    assert_eq!(doc.as_object().unwrap().len(), 2);
    assert_eq!(doc["quick_share"], 2);
    let snapshot = &doc["snapshot"];
    assert_eq!(snapshot["name"], "Fixture share");
    assert_eq!(snapshot["exported_at"], CREATED_AT);
    assert_eq!(snapshot.as_object().unwrap().len(), 4);
    let graph = snapshot_graph(snapshot);
    assert_eq!(
        graph.config().default_home.as_deref(),
        Some("Fixture share")
    );
    let corpus = graph.corpus(None, true);
    let home = corpus
        .pages
        .iter()
        .find(|page| page.name == "Fixture share")
        .unwrap();
    let links: Vec<_> = home
        .document
        .roots
        .iter()
        .map(|block| block.raw())
        .collect();
    assert_eq!(
        links,
        preview
            .pages
            .iter()
            .map(|page| format!("[[{}]]", page.name))
            .collect::<Vec<_>>()
    );
    assert_eq!(graph.link_target("Proj").name, "Project");
    assert!(graph.backlinks("Project").unwrap().iter().any(|group| {
        group
            .blocks
            .iter()
            .any(|block| block.raw.contains("journal note"))
    }));
}

#[test]
fn quick_share_of_one_page_is_that_page() {
    let _integration_guard = crate::integration_lock();
    let root = base();
    let store = corpus(&root);
    let (preview, doc) = document(&store, "@page and name = 'Empty'", "Ignored");
    assert_eq!(preview.title, "Empty");
    let doc: Value = serde_json::from_slice(&doc).unwrap();
    let graph = snapshot_graph(&doc["snapshot"]);
    assert_eq!(graph.config().default_home.as_deref(), Some("Empty"));
    assert_eq!(graph.corpus(None, true).pages.len(), 1);
    for name in ["Project", "proj", "/Project/"] {
        let result = quick_share::prepare(
            &store,
            &request("@page and name = 'Project' or name = 'Empty'", name),
            CREATED_AT,
        );
        let message = result.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            message.contains("give the share another name"),
            "{name}: {message}"
        );
    }
    let bom = quick_share::prepare(
        &store,
        &request(
            "@page and name = 'Project' or name = 'Empty'",
            "Project\u{feff}",
        ),
        CREATED_AT,
    );
    assert!(bom.is_ok(), "{:?}", bom.err().map(|e| e.to_string()));
    assert!(quick_share::prepare(
        &store,
        &request("@page and name = 'Secret'", "x"),
        CREATED_AT
    )
    .is_err());
}

#[test]
fn quick_share_of_the_deepest_page_the_app_opens() {
    let _integration_guard = crate::integration_lock();
    let root = base();
    let graph = root.join("graph");
    fs::create_dir_all(graph.join("pages")).unwrap();
    let levels = tine_core::outline_kernel::OUTLINE_MAX_DEPTH;
    let text: String = (0..levels)
        .map(|level| format!("{}- level {level}\n", "  ".repeat(level)))
        .collect();
    fs::write(graph.join("pages/Deep.md"), &text).unwrap();
    let store = Store::open(&graph, Default::default()).unwrap().0;
    let (preview, doc) = document(&store, "@page and name = 'Deep'", "Ignored");
    assert_eq!(preview.pages.len(), 1);
    let doc: Value = serde_json::from_slice(&doc).unwrap();
    let core = snapshot_graph(&doc["snapshot"]);
    let corpus = core.corpus(None, true);
    let mut depth = 0;
    let mut roots = corpus.pages[0].document.roots.as_slice();
    while let Some(block) = roots.first() {
        depth += 1;
        roots = &block.children;
    }
    assert_eq!(depth, levels);
}

#[test]
fn quick_share_keeps_queries_in_page_properties_and_body() {
    let _integration_guard = crate::integration_lock();
    let store = corpus(&base());
    let (_, doc) = document(&store, PAGE_QUERIES, "Fixture share");
    let doc: Value = serde_json::from_slice(&doc).unwrap();
    let graph = snapshot_graph(&doc["snapshot"]);
    let corpus = graph.corpus(None, true);
    let dashboard = corpus
        .pages
        .iter()
        .find(|page| page.name == "Dashboard")
        .unwrap();
    assert!(dashboard
        .document
        .pre_block
        .as_deref()
        .unwrap()
        .contains(&format!("{{{{query {PAGE_PROPERTY_QUERY}}}}}")));
    assert!(dashboard.document.roots[0]
        .raw()
        .contains("{{query (task TODO)}}"));
    let registry = graph
        .query_ir(tine_store::IrRequest::Registry, &Default::default())
        .unwrap()
        .unwrap();
    let tine_store::IrAnswer::Registry(registry) = registry else {
        panic!("expected registry")
    };
    let query = tine_core::query::wire_parse::parse_query_pair(
        PAGE_PROPERTY_QUERY,
        QueryTextDialect::MacroQuery,
        &[],
        &registry,
    );
    let view = tine_core::query::wire_parse::anchored_view(&query, query.query.anchor);
    let result = graph
        .query_ir(
            tine_store::IrRequest::Run {
                query: &query.query,
                view: &view,
                context: &Default::default(),
            },
            &Default::default(),
        )
        .unwrap()
        .unwrap();
    let tine_store::IrAnswer::Result(result) = result else {
        panic!("expected query answer")
    };
    assert!(serde_json::to_string(&result.rows)
        .unwrap()
        .contains("Notes ü"));
}

#[test]
fn quick_share_lists_pages_in_the_order_the_review_showed() {
    let _integration_guard = crate::integration_lock();
    let store = corpus(&base());
    let (preview, doc) = document(&store, MANY, "Fixture share");
    let doc: Value = serde_json::from_slice(&doc).unwrap();
    let reviewed: Vec<&str> = preview.pages.iter().map(|p| p.path.as_str()).collect();
    assert!(reviewed.len() > 3 && reviewed.windows(2).all(|w| w[0] < w[1]));
    let files = doc["snapshot"]["files"].as_array().unwrap();
    let listed: Vec<_> = files[..reviewed.len()]
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(listed, reviewed);
}

fn fixture_root(generation: u8) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tests/fixtures/quick-share/g{generation}"))
}

fn frozen_document(directory: &Path) -> Vec<u8> {
    let opened = open(
        fs::read_to_string(directory.join("tine-share.bin"))
            .unwrap()
            .trim_end(),
        &KEY,
    )
    .unwrap();
    let actual = gunzip_bounded(&opened, quick_share::MAX_DOCUMENT_BYTES).unwrap();
    assert_eq!(actual, fs::read(directory.join("document.json")).unwrap());
    actual
}

#[test]
fn quick_share_generation_1_frozen_fixtures_stay_readable() {
    let _integration_guard = crate::integration_lock();
    for case in ["pages", "one-page", "answers", "page-queries"] {
        let directory = fixture_root(1).join(case);
        let document = frozen_document(&directory);
        for bytes in [document, fs::read(directory.join("writer.json")).unwrap()] {
            quick_share::validate_legacy_document(&bytes).unwrap();
            let parsed: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(parsed["quick_share"], 1, "{case}");
            assert_eq!(parsed["snapshot"]["schema"], 1, "{case}");
        }
        assert_eq!(
            fs::read_to_string(directory.join("link.txt")).unwrap(),
            format!("https://tine.page/s/1/#{LOGIN}/{GIST}/{}\n", b64u(&KEY)),
        );
    }
}

#[test]
fn quick_share_generation_2_fixtures_match_the_writer() {
    let _integration_guard = crate::integration_lock();
    let write = std::env::var("TINE_QUICK_SHARE_FIXTURES").as_deref() == Ok("write");
    let store = corpus(&base());
    for (case, argument) in [
        ("pages", MANY),
        ("page-queries", PAGE_QUERIES),
        ("comments-excluded", "@page and name = 'Comments'"),
        ("reduced-config", "@page and name = 'Configured'"),
    ] {
        let mut request = request(argument, "Fixture share");
        request.include_comments = Some(case != "comments-excluded");
        let prepared = quick_share::prepare(&store, &request, CREATED_AT).unwrap();
        let doc = gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap();
        let parsed: Value = serde_json::from_slice(&doc).unwrap();
        let graph = snapshot_graph(&parsed["snapshot"]);
        if case == "comments-excluded" {
            let text = std::str::from_utf8(&doc).unwrap();
            assert!(text.contains("public body sentinel"));
            assert!(
                !text.contains("private comment sentinel")
                    && !text.contains("private reply sentinel")
            );
        }
        if case == "reduced-config" {
            let config = graph.config();
            assert_eq!(config.start_of_week, 1);
            assert_eq!(config.macros.len(), 1);
            assert_eq!(
                config.macros.get("greet").map(String::as_str),
                Some("Hello $1")
            );
            assert!(config.favorites.is_empty());
            let text = std::str::from_utf8(&doc).unwrap();
            assert!(!text.contains("UNUSED-MACRO-SECRET") && !text.contains("PRIVATE-FAVORITE"));
        }
        let directory = fixture_root(2).join(case);
        let link = quick_share::link(LOGIN, GIST, &KEY) + "\n";
        let golden = serde_json::to_string_pretty(&parsed).unwrap() + "\n";
        if write {
            fs::create_dir_all(&directory).unwrap();
            let envelope = directory.join("tine-share.bin");
            if !envelope.exists() {
                fs::write(envelope, seal(&prepared.payload, &KEY, &SHARE_ID, &NONCE)).unwrap();
                fs::write(directory.join("document.json"), &doc).unwrap();
                fs::write(directory.join("link.txt"), &link).unwrap();
            }
            fs::write(directory.join("writer.json"), &golden).unwrap();
        }
        assert_eq!(
            frozen_document(&directory),
            doc,
            "{case}: frozen bytes drifted"
        );
        assert_eq!(
            fs::read_to_string(directory.join("link.txt")).unwrap(),
            link
        );
        assert_eq!(
            fs::read_to_string(directory.join("writer.json")).unwrap(),
            golden
        );
    }
}
