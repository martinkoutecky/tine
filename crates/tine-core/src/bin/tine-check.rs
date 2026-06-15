//! Privacy-safe round-trip checker for a Logseq/Tine graph.
//!
//! Parses then re-serializes every `.md` file and reports, in aggregate, where
//! Tine's canonical form would differ from what's on disk — WITHOUT printing any
//! of your note content or page names. The summary is safe to share.
//!
//!   tine-check <graph-dir>            # counts + categories only (shareable)
//!   tine-check <graph-dir> --paths    # also list relative paths (local use)
//!
//! "structural" = a real round-trip bug (re-parsing the output yields a
//! different document → potential data loss). Everything else is cosmetic
//! canonicalization (tabs, trailing newline, …) but is still listed because it
//! causes diff churn with Syncthing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tine_core::doc;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = match args.next() {
        Some(d) => d,
        None => {
            eprintln!("usage: tine-check <graph-dir> [--paths]");
            std::process::exit(2);
        }
    };
    let show_paths = args.any(|a| a == "--paths");
    let root = PathBuf::from(&dir);

    let mut total = 0usize;
    let mut byte_identical = 0usize;
    let mut canonicalized = 0usize;
    let mut structural = 0usize;
    let mut cats: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut structural_paths: Vec<String> = Vec::new();

    walk(&root, &mut |path, content| {
        total += 1;
        let parsed = doc::parse(content);
        let out = doc::serialize(&parsed);
        let reparsed = doc::parse(&out);
        if reparsed != parsed {
            structural += 1;
            for t in classify(content, &out) {
                *cats.entry(t).or_default() += 1;
            }
            *cats.entry("STRUCTURAL").or_default() += 1;
            if let Ok(rel) = path.strip_prefix(&root) {
                structural_paths.push(rel.display().to_string());
            }
        } else if out != *content {
            canonicalized += 1;
            for t in classify(content, &out) {
                *cats.entry(t).or_default() += 1;
            }
        } else {
            byte_identical += 1;
        }
    });

    println!("Tine round-trip check  ({} .md files)", total);
    println!("  byte-identical : {byte_identical}");
    println!("  canonicalized  : {canonicalized}   (cosmetic, but causes Syncthing diff churn)");
    println!("  STRUCTURAL     : {structural}   (round-trip bugs — investigate)");
    if !cats.is_empty() {
        println!("\ndifference categories (file counts):");
        for (k, v) in &cats {
            println!("  {k:<18} {v}");
        }
    }
    if structural > 0 {
        if show_paths {
            println!("\nstructural-bug files:");
            for p in &structural_paths {
                println!("  {p}");
            }
        } else {
            println!("\n(run again with --paths to list the {structural} structural-bug files locally)");
        }
    }
    println!("\nCategory legend: crlf=CRLF line endings · trailing-ws=trailing spaces ·");
    println!("blank-lines=blank-line spacing · leading-indent=tabs/spaces indentation ·");
    println!("content-change=a non-whitespace line changed (report this one to me).");
}

/// Classify the *kind* of difference without revealing content.
fn classify(content: &str, out: &str) -> Vec<&'static str> {
    let mut tags = Vec::new();
    if content.contains('\r') {
        tags.push("crlf");
    }
    // Strip CRLF and compare; if equal, the only diff was line endings.
    let c_lf = content.replace("\r\n", "\n");
    if c_lf == *out {
        return dedup(tags);
    }
    let cl: Vec<&str> = c_lf.split('\n').collect();
    let ol: Vec<&str> = out.split('\n').collect();

    let c_blanks = cl.iter().filter(|l| l.trim().is_empty()).count();
    let o_blanks = ol.iter().filter(|l| l.trim().is_empty()).count();
    if c_blanks != o_blanks || cl.len() != ol.len() {
        tags.push("blank-lines");
    }

    // Compare the non-blank lines, normalized for leading/trailing whitespace.
    let cn: Vec<String> = cl.iter().filter(|l| !l.trim().is_empty()).map(norm).collect();
    let on: Vec<String> = ol.iter().filter(|l| !l.trim().is_empty()).map(norm).collect();
    if cn == on {
        // Only whitespace differences among content lines.
        if cl.iter().zip(&ol).any(|(a, b)| a.trim_end() != b.trim_end() && a.trim() == b.trim()) {
            tags.push("trailing-ws");
        }
        tags.push("leading-indent");
    } else {
        // A real content line changed — the important one.
        tags.push("content-change");
    }
    dedup(tags)
}

fn norm(l: &&str) -> String {
    l.trim().to_string()
}
fn dedup(mut v: Vec<&'static str>) -> Vec<&'static str> {
    v.sort_unstable();
    v.dedup();
    v
}

fn walk(dir: &Path, f: &mut impl FnMut(&Path, &String)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip Tine/Logseq output + VCS dirs.
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, ".git" | "publish" | ".obsidian") {
                continue;
            }
            walk(&path, f);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                f(&path, &content);
            }
        }
    }
}
