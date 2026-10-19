//! Quick share generation 1, writer side (`docs/contracts/quick-share.md`):
//! the envelope, closure of the shared slice, and the frozen fixtures the
//! viewer decodes (`tests/fixtures/quick-share/g1/`, also read by
//! `src/quickShare/quickShare.fixtures.test.ts`).
//!
//! `TINE_QUICK_SHARE_FIXTURES=write` writes a fixture case that does not exist
//! yet and refreshes every `writer.json`; a frozen `tine-share.bin`,
//! `document.json` or `link.txt` is never rewritten.
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-quick-share-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("scratch")).unwrap();
    root
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
            "{:file/name-format :triple-lowbar\n :hidden [\"hidden\"]}\n",
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
    }
}

const MANY: &str = "@page and name = 'Project' or name = 'Notes ü' or name = 'Dashboard' or name = 'Empty' or name = 'Org page' or name = 'Oct 17th, 2026' or name = 'Secret' or name = 'Hidden'";

fn document(
    store: &Store,
    root: &Path,
    argument: &str,
    name: &str,
) -> (quick_share::SharePreview, Vec<u8>) {
    let prepared = quick_share::prepare(
        store,
        &request(argument, name),
        CREATED_AT,
        &root.join("scratch"),
    )
    .unwrap();
    assert_eq!(
        fs::read_dir(root.join("scratch")).unwrap().count(),
        0,
        "the private build graph is removed"
    );
    let doc = gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap();
    (prepared.preview, doc)
}

#[test]
fn quick_share_envelope_round_trips_and_refuses_damage() {
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

#[test]
fn quick_share_is_closed_to_the_shared_pages() {
    let _integration_guard = crate::integration_lock();
    let root = base();
    let store = corpus(&root);
    let (preview, doc) = document(&store, &root, MANY, "Fixture share");
    let names: Vec<_> = preview.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names.len(), 6, "{names:?}");
    assert!(!names.contains(&"Secret"));
    assert_eq!(preview.left_out.len(), 1);
    assert_eq!(preview.left_out[0].name, "Secret");
    let text = String::from_utf8(doc.clone()).unwrap();
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
        "tine-quick-share-",
        root.to_str().unwrap(),
        "select-query",
    ] {
        assert!(!text.contains(leak), "the share carries {leak:?}");
    }
    let doc: Value = serde_json::from_slice(&doc).unwrap();
    assert_eq!(doc["quick_share"], 1);
    assert_eq!(doc["created_at"], CREATED_AT);
    assert_eq!(doc["title"], "Fixture share");
    let snapshot = &doc["snapshot"];
    assert_eq!(snapshot["schema"], 1);
    assert_eq!(snapshot["exported_at"], CREATED_AT);
    assert_eq!(snapshot["home"], "Fixture share");
    // The selecting query never ships: the home only lists the pages.
    assert!(!text.contains("name = 'Project'"));
    // A baked query on a shared page answers from shared pages only.
    let tasks = snapshot["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["host"] == "Project")
        .unwrap();
    let rows = serde_json::to_string(&tasks["result"]).unwrap();
    assert!(rows.contains("first task") && rows.contains("org task"));
    // Backlinks and aliases come from the slice.
    let backlinks = serde_json::to_string(&snapshot["backlinks"]["Project"]).unwrap();
    assert!(backlinks.contains("journal note"));
    assert!(snapshot["aliases"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a[0] == "proj" || a[0] == "Proj"));
}

#[test]
fn quick_share_of_one_page_is_that_page() {
    let _integration_guard = crate::integration_lock();
    let root = base();
    let store = corpus(&root);
    let (preview, doc) = document(&store, &root, "@page and name = 'Empty'", "Ignored");
    assert_eq!(preview.title, "Empty");
    let doc: Value = serde_json::from_slice(&doc).unwrap();
    assert_eq!(doc["snapshot"]["home"], "Empty");
    assert_eq!(doc["snapshot"]["pages"].as_array().unwrap().len(), 1);
    for name in ["Project", "proj", "/Project/"] {
        let clash = quick_share::prepare(
            &store,
            &request("@page and name = 'Project' or name = 'Empty'", name),
            CREATED_AT,
            &root.join("scratch"),
        );
        let message = clash.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            message.contains("give the share another name"),
            "{name}: {message}"
        );
    }
    // Review g1 r2 B1: a BOM is not whitespace to page_key, so this name is no clash (the viewer agrees).
    let bom = quick_share::prepare(
        &store,
        &request(
            "@page and name = 'Project' or name = 'Empty'",
            "Project\u{feff}",
        ),
        CREATED_AT,
        &root.join("scratch"),
    );
    assert!(bom.is_ok(), "{:?}", bom.err().map(|e| e.to_string()));
    let refused = quick_share::prepare(
        &store,
        &request("@page and name = 'Secret'", "x"),
        CREATED_AT,
        &root.join("scratch"),
    );
    assert!(refused.is_err());
}

/// Runtime block ids are fresh per parse; a golden compares them by position.
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

#[test]
fn quick_share_generation_1_fixtures_stay_readable() {
    let _integration_guard = crate::integration_lock();
    let write = std::env::var("TINE_QUICK_SHARE_FIXTURES").as_deref() == Ok("write");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/quick-share/g1");
    let root = base();
    let store = corpus(&root);
    for (case, argument, name) in [
        ("pages", MANY, "Fixture share"),
        ("one-page", "@page and name = 'Project'", "Ignored"),
    ] {
        let (_, doc) = document(&store, &root, argument, name);
        let mut fresh: Value = serde_json::from_slice(&doc).unwrap();
        normalize(&mut fresh);
        let case_dir = dir.join(case);
        let bin = case_dir.join("tine-share.bin");
        if !bin.exists() {
            assert!(
                write,
                "missing frozen fixture {case}; run with TINE_QUICK_SHARE_FIXTURES=write"
            );
            fs::create_dir_all(&case_dir).unwrap();
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(9));
            std::io::Write::write_all(&mut gz, &doc).unwrap();
            fs::write(&bin, seal(&gz.finish().unwrap(), &KEY, &SHARE_ID, &NONCE)).unwrap();
            fs::write(case_dir.join("document.json"), &doc).unwrap();
            fs::write(
                case_dir.join("link.txt"),
                quick_share::link(LOGIN, GIST, &KEY) + "\n",
            )
            .unwrap();
        }
        // The frozen bytes open to the frozen document, forever.
        let frozen_doc = fs::read(case_dir.join("document.json")).unwrap();
        let opened = open(fs::read_to_string(&bin).unwrap().trim_end(), &KEY).unwrap();
        assert_eq!(
            gunzip_bounded(&opened, 64 << 20).unwrap(),
            frozen_doc,
            "{case}"
        );
        // The writer still produces that document (runtime ids aside): an
        // app-side DTO change that alters generation 1 fails here.
        let golden = case_dir.join("writer.json");
        let fresh_text = serde_json::to_string_pretty(&fresh).unwrap() + "\n";
        if write {
            fs::write(&golden, &fresh_text).unwrap();
        }
        let mut frozen: Value = serde_json::from_slice(&frozen_doc).unwrap();
        normalize(&mut frozen);
        assert_eq!(
            fs::read_to_string(&golden).unwrap(),
            fresh_text,
            "{case}: writer drifted"
        );
        assert_eq!(
            frozen, fresh,
            "{case}: writer no longer writes the frozen document"
        );
    }
}
