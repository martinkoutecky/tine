//! Build the public Guide demo with the same live exporter as a user graph.
//! The graph is temporary, all-pages selection applies to this run only, and
//! the output is an external create-only directory. `dist/` must be built first.

use std::fs;
use std::path::{Path, PathBuf};

use tine_graph_features::guide::create_demo_graph;
use tine_graph_features::publish_query::publish_live;
use tine_store::Store;

fn bundle(dir: &Path, prefix: &str, files: &mut Vec<(String, Vec<u8>)>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if entry.file_type()?.is_dir() {
            bundle(&path, &relative, files)?;
        } else if relative == "index.html" || relative.starts_with("assets/") {
            files.push((relative, fs::read(path)?));
        }
    }
    Ok(())
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: build-demo-site <out_dir>"),
    );
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let leaf = out
        .file_name()
        .and_then(|s| s.to_str())
        .expect("output leaf");
    assert_eq!(leaf, "demo", "Guide output must be named demo");
    let temp = tempfile::tempdir().expect("temporary Guide graph");
    create_demo_graph(temp.path()).expect("scaffold Guide graph");
    let (store, _, _) = Store::open(temp.path(), Default::default()).expect("open Guide graph");
    let mut files = Vec::new();
    bundle(Path::new("dist"), "", &mut files).expect("read built frontend");
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let receipt = publish_live(&store, parent, "demo", true, &files).expect("publish Guide demo");
    println!("published {} pages -> {}", receipt.pages, receipt.path);
}
