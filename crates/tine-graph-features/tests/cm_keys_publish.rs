//! Conceptual model step 2, census Q1 (the page-name key), the publish path:
//! `publish_query.rs` keys pages by `name.to_ascii_lowercase()` (the owner
//! map) and `name.to_lowercase()` (host page, result closure, baked queries,
//! snapshot, home-name clash) instead of the answerer `page_key`, while the
//! published reader (`src/publishedBackend.ts pageByName`) looks pages up by
//! `pageIdentityKey`, the wasm build of `page_key`. Two probes record where
//! that changes what the user gets:
//! - the generated home page is named so that it clashes with an exported
//!   page under `page_key` but not under `to_lowercase` (boundary slash,
//!   edge whitespace, NFD), so the published site has two pages with one key;
//! - a query whose rows come from two files claiming one page key: refused as
//!   "ambiguous page owner" for an ASCII name, published for a non-ASCII one.
//!
//! No bridge. Differences: part `keysPublish` of `expected-differences.json`
//! (tine-store fixtures); `CM_REFRESH=1` rewrites it.

use std::fs;
use std::path::Path;

use tine_core::query::wire_parse::QueryTextDialect;
use tine_core::refs::page_key;
use tine_graph_features::publish_query::{plan_query, publish_query, QueryExportRequest};
use tine_store::Store;

use tine_store as store_crate;

#[path = "../../tine-store/src/cm_conformance_shared_tests.rs"]
mod shared;

use shared::Section;

fn graph(root: &Path, files: &[(&str, &str)]) -> Store {
    for dir in ["pages", "journals"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, text) in files {
        fs::write(root.join(rel), text).unwrap();
    }
    Store::open(root, Default::default()).unwrap().0
}

fn request(name: &str) -> QueryExportRequest {
    QueryExportRequest {
        argument: "(task TODO)".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: Vec::new(),
        current_page: None,
        name: name.into(),
        host_block_id: None,
        folder: Some("probe".into()),
        replace: false,
        asset_budget_bytes: None,
        include_query_text: false,
        include_comments: None,
    }
}

fn snapshot(root: &Path) -> Option<serde_json::Value> {
    fn walk(dir: &Path) -> Option<std::path::PathBuf> {
        for entry in fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk(&path) {
                    return Some(found);
                }
            } else if path.file_name().is_some_and(|n| n == "snapshot.json") {
                return Some(path);
            }
        }
        None
    }
    let path = walk(&root.join("published-queries"))?;
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn bundle() -> Vec<(String, Vec<u8>)> {
    vec![(
        "index.html".into(),
        b"<!doctype html><html><head><title>Tine</title></head><body><div id=\"root\"></div></body></html>"
            .to_vec(),
    )]
}

#[test]
fn publish_keys_pages_like_the_reader() {
    let _integration_guard = crate::integration_lock();
    let mut section = Section::default();

    // Probe 1: the home page's name against the exported pages, by page_key.
    let mut homes = Vec::new();
    for home in ["Notes", "/Notes", "Notes/", "O\u{308}tes", "\u{D6}tes"] {
        let dir = tempfile::tempdir().unwrap();
        let store = graph(
            dir.path(),
            &[
                ("pages/Notes.md", "- TODO a\n"),
                ("pages/\u{D6}tes.md", "- TODO b\n"),
            ],
        );
        let req = request(home);
        let outcome = plan_query(&store, &req)
            .and_then(|plan| publish_query(&store, &req, &plan.fingerprint, &bundle()));
        let Ok(_) = outcome else {
            homes.push(format!("home {home:?}: publish failed: {outcome:?}"));
            continue;
        };
        let Some(snap) = snapshot(dir.path()) else {
            homes.push(format!("home {home:?}: no snapshot written"));
            continue;
        };
        let names: Vec<String> = snap["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap_or_default().to_owned())
            .collect();
        let mut seen = std::collections::BTreeMap::<String, Vec<String>>::new();
        for name in &names {
            seen.entry(page_key(name)).or_default().push(name.clone());
        }
        for (key, group) in seen {
            if group.len() > 1 {
                homes.push(format!(
                    "home {home:?}: the published site has {} pages with key {key:?}: {group:?}",
                    group.len()
                ));
            }
        }
        store.close();
    }
    section.fixtures.insert("home-name".into(), homes);

    // Probe 2: two files claiming one page key, ASCII and non-ASCII.
    let mut owners = Vec::new();
    for (label, a, b) in [
        ("ascii", "pages/Abc.md", "pages/abc.md"),
        ("non-ascii", "pages/\u{DC}ber.md", "pages/\u{FC}ber.md"),
        ("nfd", "pages/\u{FC}ber.md", "pages/u\u{308}ber.md"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = graph(dir.path(), &[(a, "- TODO one\n"), (b, "- TODO two\n")]);
        if !shared::listed_apart(dir.path(), [a, b]) {
            // One file here, so no two claims to tell apart.
            section.skip_on_volume("duplicate-claim", &format!("{label} ({a}, {b}): "));
            store.close();
            continue;
        }
        let outcome = plan_query(&store, &request("Out"));
        owners.push(format!(
            "{label} ({a}, {b}): {}",
            match outcome {
                Ok(plan) => format!("published, {} pages", plan.pages.len()),
                Err(e) => format!("refused: {e}"),
            }
        ));
        store.close();
    }
    section.fixtures.insert("duplicate-claim".into(), owners);

    shared::check_expected(
        env!("CARGO_MANIFEST_DIR"),
        "keysPublish",
        &section,
        "CM_REFRESH=1 cargo test -p tine-graph-features --test integration cm_keys_publish",
    );
}
