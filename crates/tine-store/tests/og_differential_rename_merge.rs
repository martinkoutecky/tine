//! Master-vs-og byte differential for rename onto an existing page (merge,
//! master 7160c501cb88 `merge_pages_after_rename`; GH #327). The golden is the
//! master oracle's output on the same input. og differs by design in exactly
//! one file, asserted literally below: the survivor unites the source's
//! `alias::` (master drops it, leaving `[[Former]]` dangling).
use serde::Deserialize;
use std::{fs, path::Path};
use tine_graph_features::pages;
use tine_store::Store;

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

#[derive(Deserialize)]
struct Input {
    old: String,
    new: String,
    src: String,
    dst: String,
    files: Vec<File>,
}

#[derive(Deserialize)]
struct File {
    path: String,
    bytes: String,
}

#[derive(Deserialize)]
struct Golden {
    removed: Vec<String>,
    files: Vec<File>,
}

fn collect(root: &Path, rel: &str, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(root.join(rel)).unwrap() {
        let entry = entry.unwrap();
        let child = format!("{rel}/{}", entry.file_name().to_string_lossy());
        if entry.file_type().unwrap().is_dir() {
            collect(root, &child, out);
        } else {
            out.push((child, fs::read_to_string(entry.path()).unwrap()));
        }
    }
}

#[test]
fn rename_merge_paths_and_bytes_match_master_7160c501() {
    let input: Input = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/og-rename-merge-input.json"
    ))
    .unwrap();
    let golden: Golden = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/og-master-rename-merge-golden.json"
    ))
    .unwrap();
    let root = std::env::temp_dir().join(format!("tine-og-diff-merge-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    for dir in ["pages", "journals"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for file in &input.files {
        let dest = root.join(&file.path);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(dest, file.bytes.as_bytes()).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    hosted(&store, |host| {
        pages::rename_or_merge_page(
            &store,
            host,
            &input.old,
            &input.new,
            Some(&input.src),
            Some(&input.dst),
            &[],
        )
    })
    .unwrap();

    let mut removed: Vec<String> = input
        .files
        .iter()
        .filter(|file| !root.join(&file.path).exists())
        .map(|file| file.path.clone())
        .collect();
    removed.sort();
    assert_eq!(removed, golden.removed, "removed source paths differ");

    let mut actual = Vec::new();
    for dir in ["pages", "journals"] {
        collect(&root, dir, &mut actual);
    }
    actual.sort();
    let mut expected: Vec<_> = golden
        .files
        .into_iter()
        .map(|f| (f.path, f.bytes))
        .collect();
    let survivor = expected
        .iter_mut()
        .find(|(path, _)| path == &input.dst)
        .unwrap();
    assert_eq!(
        survivor.1,
        "alias:: Kept\nicon:: star\n\n- new body links [[New]]\n- old body links [[New]] and [[New/Child]]\n\t- nested #New\n",
        "master golden changed; re-derive the argued difference"
    );
    survivor.1 = survivor
        .1
        .replace("alias:: Kept\n", "alias:: Kept, Former\n");
    assert_eq!(actual, expected, "merged output paths or bytes differ");
    let graph = store.whole_graph().unwrap();
    assert!(
        matches!(
            graph.resolve("Former", false),
            tine_store::Resolved::Alias { .. }
        ),
        "the source alias keeps resolving to the survivor"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
