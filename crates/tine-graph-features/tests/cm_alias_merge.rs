//! Conceptual model step 2, census Q7 (the alias rule), the write side: a
//! merge must keep every alias the two pages declared before it. The store's
//! answerer (`tine-store` `query::document_alias_spellings`: the page header,
//! or a properties-only first block when there is no header; keys `alias` and
//! `aliases`) says which spellings name each page before the merge; after a
//! plain merge (`merge_pages`) and after a rename onto the other page
//! (`rename_or_merge_page` with the merge confirmed), each of those spellings
//! must still name the survivor. (The source's own name is not checked: a
//! merge trashes the source and rewrites its references, by design.)
//! `pages/merge_payload.rs` decides the merged
//! header with its own alias rule; each case records which members it loses.
//!
//! Runs without the bridge (every gate). Differences: part `aliasMerge` of
//! `crates/tine-store/tests/fixtures/cm-conformance/expected-differences.json`;
//! `CM_REFRESH=1` rewrites it. No model-side check yet: the bridge runs model
//! r6, whose rename onto a real page is refused by the base guard
//! (`r6-onto-real-page`); merge semantics arrive with the r7 bridge.

use std::fs;
use std::path::Path;

use tine_graph_features::pages;
use tine_store::{Resolved, Store};

use tine_store as store_crate;

#[path = "../../tine-store/src/cm_conformance_shared_tests.rs"]
mod shared;

use shared::Section;

/// (case, source file, its text, survivor file, its text, spellings each page
/// declares before the merge). The source page's name is its file stem.
const CASES: &[(&str, &str, &str, &str, &str, &[&str])] = &[
    (
        "alias-into-alias",
        "pages/src.md",
        "alias:: Former, Shared\n\n- moved\n",
        "pages/dst.md",
        "alias:: Shared, Kept\n\n- kept\n",
        &["Former", "Shared", "Kept"],
    ),
    (
        "aliases-into-aliases",
        "pages/src.md",
        "aliases:: Former\n\n- moved\n",
        "pages/dst.md",
        "aliases:: Kept\n\n- kept\n",
        &["Former", "Kept"],
    ),
    (
        "alias-into-aliases",
        "pages/src.md",
        "alias:: Former\n\n- moved\n",
        "pages/dst.md",
        "aliases:: Kept\n\n- kept\n",
        &["Former", "Kept"],
    ),
    (
        "aliases-into-alias",
        "pages/src.md",
        "aliases:: Former\n\n- moved\n",
        "pages/dst.md",
        "alias:: Kept\n\n- kept\n",
        &["Former", "Kept"],
    ),
    (
        "capital-key",
        "pages/src.md",
        "Alias:: Former\n\n- moved\n",
        "pages/dst.md",
        "alias:: Kept\n\n- kept\n",
        &["Former", "Kept"],
    ),
    (
        "source-first-block-header",
        "pages/src.md",
        "- alias:: Former\n- moved\n",
        "pages/dst.md",
        "- kept\n",
        &["Former"],
    ),
    (
        "survivor-first-block-header",
        "pages/src.md",
        "alias:: Former\n\n- moved\n",
        "pages/dst.md",
        "- alias:: Kept\n- kept\n",
        &["Former", "Kept"],
    ),
    (
        "org-aliases-into-aliases",
        "pages/src.org",
        "#+ALIASES: Former\n\n* moved\n",
        "pages/dst.org",
        "#+ALIASES: Kept\n\n* kept\n",
        &["Former", "Kept"],
    ),
];

fn fixture(root: &Path, files: &[(&str, &str)]) -> std::sync::Arc<Store> {
    for dir in ["pages", "journals"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, text) in files {
        fs::write(root.join(rel), text).unwrap();
    }
    std::sync::Arc::new(Store::open(root, Default::default()).unwrap().0)
}

fn owner(store: &Store, name: &str) -> Option<String> {
    let graph = store.whole_graph().unwrap();
    match graph.resolve(name, false) {
        Resolved::Existing { id, .. } => Some(id.as_str().to_owned()),
        Resolved::Alias { owners } => owners.first().map(|id| id.as_str().to_owned()),
        Resolved::Absent { .. } => None,
    }
}

fn stem(rel: &str) -> &str {
    let file = rel.rsplit('/').next().unwrap();
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

#[test]
fn a_merge_keeps_every_declared_alias() {
    let _integration_guard = crate::integration_lock();
    let mut section = Section::default();
    for &(case, src, src_text, dst, dst_text, members) in CASES {
        for mode in ["merge", "rename-merge"] {
            let dir = tempfile::tempdir().unwrap();
            let store = fixture(dir.path(), &[(src, src_text), (dst, dst_text)]);
            // Precondition: the store's alias rule names a page by each member.
            for member in members {
                assert!(
                    owner(&store, member).is_some(),
                    "{case}: the store does not read {member:?} as an alias before the merge"
                );
            }
            let host = crate::test_host(&store);
            match mode {
                "merge" => pages::merge_pages(&store, &host, src, dst).unwrap(),
                _ => {
                    pages::rename_or_merge_page(
                        &store,
                        &host,
                        stem(src),
                        stem(dst),
                        None,
                        Some(dst),
                        &[],
                    )
                    .unwrap();
                }
            }
            let mut lost = Vec::new();
            for member in members.iter().copied() {
                let now = owner(&store, member);
                if now.as_deref() != Some(dst) {
                    lost.push(format!(
                        "{mode}: {member:?} no longer names the survivor (now {now:?}); survivor file {:?}",
                        fs::read_to_string(dir.path().join(dst)).unwrap()
                    ));
                }
            }
            if !lost.is_empty() {
                section
                    .fixtures
                    .entry(case.to_owned())
                    .or_default()
                    .extend(lost);
            }
            drop(host);
            drop(store);
        }
    }
    shared::check_expected(
        env!("CARGO_MANIFEST_DIR"),
        "aliasMerge",
        &section,
        "CM_REFRESH=1 cargo test -p tine-graph-features --test integration cm_alias_merge",
    );
}
