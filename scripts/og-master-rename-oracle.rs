// Copy into the ddf408c55 tine-core examples directory and run with the
// synthetic input JSON path. Output is a raw-byte manifest of the rename.
use serde::{Deserialize, Serialize};
use std::{env, fs, path::{Path, PathBuf}};

#[derive(Deserialize)]
struct Input { old: String, new: String, files: Vec<File> }

#[derive(Deserialize, Serialize)]
struct File { path: String, bytes: String }

#[derive(Serialize)]
struct Output { removed: Vec<String>, files: Vec<File> }

fn collect(root: &Path, rel: &str, out: &mut Vec<File>) {
    for entry in fs::read_dir(root.join(rel)).unwrap() {
        let entry = entry.unwrap();
        let child = format!("{rel}/{}", entry.file_name().to_string_lossy());
        if entry.file_type().unwrap().is_dir() {
            collect(root, &child, out);
        } else {
            out.push(File { path: child, bytes: fs::read_to_string(entry.path()).unwrap() });
        }
    }
}

fn main() {
    let file = env::args().nth(1).expect("input JSON path");
    let input: Input = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
    let root: PathBuf = env::temp_dir().join(format!("tine-og-rename-oracle-{}", std::process::id()));
    if root.exists() { fs::remove_dir_all(&root).unwrap(); }
    for dir in ["pages", "journals"] { fs::create_dir_all(root.join(dir)).unwrap(); }
    for entry in &input.files {
        let destination = root.join(&entry.path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, entry.bytes.as_bytes()).unwrap();
    }
    let graph = tine_core::model::Graph::open(&root);
    graph.rename_page(&input.old, &input.new).unwrap();
    let mut files = Vec::new();
    for dir in ["pages", "journals"] { collect(&root, dir, &mut files); }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut removed: Vec<String> = input.files.iter().filter(|entry| !root.join(&entry.path).exists())
        .map(|entry| entry.path.clone()).collect();
    removed.sort();
    println!("{}", serde_json::to_string_pretty(&Output { removed, files }).unwrap());
    drop(graph);
    fs::remove_dir_all(root).unwrap();
}
