//! REG-CM-MERGE-LOSES-ALIASES: a merge (and a rename onto an existing page,
//! merge confirmed) keeps every alias either page declared. "Declared" is the
//! store's own alias answer (`tine-store` `query::document_alias_spellings`:
//! keys `alias` and `aliases`, from the page header, or from a properties-only
//! first block when the page has no header); after the merge each spelling
//! must still open the survivor. Before the fix `pages/merge_payload.rs` kept
//! its own rule: it united only the key `alias` and read only the header, so
//! `aliases::` on both pages, Org `#+ALIASES:`, and aliases written as either
//! page's first bullet stopped naming the merged page (the text moved into the
//! body). The source's own name is not checked: a merge trashes the source and
//! rewrites its references, by design.

use std::fs;
use std::path::Path;

use tine_graph_features::pages;
use tine_store::{Resolved, Store};

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
        "both-first-block-headers",
        "pages/src.md",
        "- aliases:: Former\n- moved\n",
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
    let mut lost = Vec::new();
    for &(case, src, src_text, dst, dst_text, members) in CASES {
        for mode in ["merge", "rename-merge"] {
            let dir = tempfile::tempdir().unwrap();
            let store = fixture(dir.path(), &[(src, src_text), (dst, dst_text)]);
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
            let survivor = fs::read_to_string(dir.path().join(dst)).unwrap();
            for member in members.iter().copied() {
                let now = owner(&store, member);
                if now.as_deref() != Some(dst) {
                    lost.push(format!(
                        "{case} ({mode}): {member:?} no longer opens the survivor (now {now:?}); survivor file {survivor:?}"
                    ));
                }
            }
            // Nothing the user wrote is dropped: every line of both pages'
            // text is still in the survivor (moved, never deleted).
            for word in ["moved", "kept"] {
                if !survivor.contains(word) {
                    lost.push(format!(
                        "{case} ({mode}): block {word:?} is missing: {survivor:?}"
                    ));
                }
            }
            drop(host);
            drop(store);
        }
    }
    assert!(
        lost.is_empty(),
        "a merge lost aliases:\n{}",
        lost.join("\n")
    );
}
