//! Export every block's `raw` body across one or more graph directories to a
//! `block-raws.json` at each graph's root — for lsdoc's `realmut` differential
//! (see lsdoc/FOR-TINE.md). Each entry is `{ raw, format }` where `raw` is the
//! de-bulleted, de-indented `:block/content` exactly as Tine stores it (lsdoc
//! re-bullets it itself, the OG way — do NOT re-bullet here).
//!
//! Usage: export-block-raws <graphdir> [<graphdir> ...]
//!   writes <graphdir>/block-raws.json for each dir.
use std::fs;
use std::path::{Path, PathBuf};
use tine_core::doc;
use tine_core::org;
use tine_core::DocBlock;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            walk(&p, out);
        } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            if ext == "md" || ext == "org" {
                out.push(p);
            }
        }
    }
}

fn collect(blocks: &[DocBlock], fmt: &str, out: &mut Vec<serde_json::Value>) {
    for b in blocks {
        out.push(serde_json::json!({ "raw": b.raw, "format": fmt }));
        collect(&b.children, fmt, out);
    }
}

fn main() {
    let dirs: Vec<String> = std::env::args().skip(1).collect();
    if dirs.is_empty() {
        eprintln!("usage: export-block-raws <graphdir> [<graphdir> ...]");
        std::process::exit(2);
    }
    for dir in &dirs {
        let mut files = Vec::new();
        walk(Path::new(dir), &mut files);
        files.sort();
        let mut records: Vec<serde_json::Value> = Vec::new();
        for f in &files {
            let Ok(content) = fs::read_to_string(f) else {
                continue;
            };
            let is_org = f.extension().and_then(|s| s.to_str()) == Some("org");
            let (document, fmt) = if is_org {
                (org::parse_org(&content), "org")
            } else {
                (doc::parse(&content), "md")
            };
            if let Some(pre) = &document.pre_block {
                // The page-property pre-block is real content lsdoc should see too.
                records.push(serde_json::json!({ "raw": pre, "format": fmt }));
            }
            collect(&document.roots, fmt, &mut records);
        }
        let out = Path::new(dir).join("block-raws.json");
        let json = serde_json::to_string_pretty(&records).unwrap();
        if let Err(e) = fs::write(&out, json) {
            eprintln!("{}: write failed: {e}", out.display());
            continue;
        }
        eprintln!(
            "{}: {} blocks from {} files -> {}",
            dir,
            records.len(),
            files.len(),
            out.display()
        );
    }
}
