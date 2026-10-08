use serde::Deserialize;
use std::{fs, path::Path};
use tine_graph_features::pages;
use tine_store::Store;

#[derive(Deserialize)]
struct Input {
    old: String,
    new: String,
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

fn collect(root: &Path, rel: &str, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in fs::read_dir(root.join(rel)).unwrap() {
        let entry = entry.unwrap();
        let child = format!("{rel}/{}", entry.file_name().to_string_lossy());
        if entry.file_type().unwrap().is_dir() {
            collect(root, &child, out);
        } else {
            out.push((child, fs::read(entry.path()).unwrap()));
        }
    }
}

#[test]
fn rename_paths_and_bytes_match_master_ddf408c55() {
    let input: Input = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/og-rename-input.json"
    ))
    .unwrap();
    let golden: Golden = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/og-master-rename-golden.json"
    ))
    .unwrap();
    let root = std::env::temp_dir().join(format!("tine-og-diff-rename-{}", std::process::id()));
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
    let store = Store::open(&root, Default::default()).unwrap().0;
    pages::rename_page_expected(&store, &input.old, &input.new, None).unwrap();

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
    actual.sort_by(|a, b| a.0.cmp(&b.0));
    let expected: Vec<_> = golden
        .files
        .into_iter()
        .map(|file| (file.path, file.bytes.into_bytes()))
        .collect();
    assert_eq!(actual, expected, "renamed output paths or bytes differ");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
