//! Build the public Guide demo with the same live exporter as a user graph.
//! The graph is temporary, all-pages selection applies to this run only, and
//! the output is an external create-only directory. `dist/` must be built first.
//!
//! Sheets are computed by the app's own TS evaluator, never here (I-12), so the docs
//! build is two-phase (`scripts/build-guide-demo.mjs`): `--dump-sheet-inputs <file>`
//! writes the Guide's sheet blocks and exits; `scripts/sheet-export-cli.ts` (vite-node,
//! the same `src/sheet/staticExport.ts` the app calls) turns them into exports; then
//! `--sheets <file>` publishes with them. With neither flag sheets keep their outline.

use std::fs;
use std::path::{Path, PathBuf};

use tine_graph_features::guide::create_demo_graph;
use tine_graph_features::publish::sheet_export_inputs;
use tine_graph_features::publish_query::publish_live_with_sheets;
use tine_graph_features::SheetExport;
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
    let mut args = std::env::args().skip(1);
    let (mut out, mut dump, mut sheets_file) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dump-sheet-inputs" => dump = args.next().map(PathBuf::from),
            "--sheets" => sheets_file = args.next().map(PathBuf::from),
            _ => out = Some(PathBuf::from(arg)),
        }
    }
    let temp = tempfile::tempdir().expect("temporary Guide graph");
    create_demo_graph(temp.path()).expect("scaffold Guide graph");
    let (store, _, _) = Store::open(temp.path(), Default::default()).expect("open Guide graph");
    if let Some(dump) = dump {
        let inputs = sheet_export_inputs(&store, None, None).expect("collect sheet inputs");
        fs::write(
            &dump,
            serde_json::to_vec(&inputs).expect("encode sheet inputs"),
        )
        .expect("write sheet inputs");
        println!("dumped {} sheet blocks -> {}", inputs.len(), dump.display());
        return;
    }
    let out = out.expect("usage: build-demo-site <out_dir> [--sheets <exports.json>]");
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let leaf = out
        .file_name()
        .and_then(|s| s.to_str())
        .expect("output leaf");
    assert_eq!(leaf, "demo", "Guide output must be named demo");
    let sheets: Vec<SheetExport> = match sheets_file {
        Some(file) => serde_json::from_slice(&fs::read(file).expect("read sheet exports"))
            .expect("decode sheet exports"),
        None => Vec::new(),
    };
    let mut files = Vec::new();
    bundle(Path::new("dist"), "", &mut files).expect("read built frontend");
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let receipt = publish_live_with_sheets(&store, parent, "demo", true, &files, sheets)
        .expect("publish Guide demo");
    println!("published {} pages -> {}", receipt.pages, receipt.path);
}
