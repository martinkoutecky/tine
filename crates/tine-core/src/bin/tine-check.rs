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
    // On-disk format profile (content-free).
    let mut disk_indent_tab = 0usize; // files whose nested lines use TAB indent
    let mut disk_indent_space = 0usize; // ...use SPACE indent
    let mut disk_trail_0 = 0usize; // files ending with no newline
    let mut disk_trail_1 = 0usize; // ...exactly one
    let mut disk_trail_2 = 0usize; // ...two or more (trailing blank line)

    walk(&root, &mut |path, content| {
        total += 1;
        match indent_style(content) {
            Some('\t') => disk_indent_tab += 1,
            Some(' ') => disk_indent_space += 1,
            _ => {}
        }
        match trailing_newlines(content) {
            0 => disk_trail_0 += 1,
            1 => disk_trail_1 += 1,
            _ => disk_trail_2 += 1,
        }
        let parsed = doc::parse(content);
        let out = doc::serialize(&parsed);
        let reparsed = doc::parse(&out);
        if reparsed != parsed {
            structural += 1;
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

    println!("\non-disk format profile (what your Logseq writes):");
    println!("  indentation    : TAB in {disk_indent_tab} files, SPACES in {disk_indent_space} files");
    println!("  trailing \\n    : none in {disk_trail_0}, one in {disk_trail_1}, two+ (blank end) in {disk_trail_2}");
    println!("  Tine emits     : TAB indent, single trailing \\n, blank line after page properties");

    if !cats.is_empty() {
        println!("\nprecise difference categories (file counts):");
        for (k, v) in &cats {
            println!("  {k:<22} {v}");
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
    println!("\nLegend: trailing-newline=final newline count differs · interior-blank-lines=blank");
    println!("lines between content differ · indent-char=tab/space mismatch · indent-width=same");
    println!("char different count · cont-indent=multi-line block continuation indent ·");
    println!("trailing-ws=trailing spaces · crlf=CRLF · content-change=REAL text change (report).");
}

/// Dominant indent character among a file's indented lines, if any.
fn indent_style(content: &str) -> Option<char> {
    let mut tab = 0usize;
    let mut space = 0usize;
    for l in content.split('\n') {
        match l.as_bytes().first() {
            Some(b'\t') => tab += 1,
            Some(b' ') => space += 1,
            _ => {}
        }
    }
    if tab == 0 && space == 0 {
        None
    } else if tab >= space {
        Some('\t')
    } else {
        Some(' ')
    }
}

fn trailing_newlines(content: &str) -> usize {
    content.bytes().rev().take_while(|b| *b == b'\n').count()
}

/// Classify the *kinds* of difference without revealing content.
fn classify(content: &str, out: &str) -> Vec<&'static str> {
    let mut tags = Vec::new();
    if content.contains('\r') {
        tags.push("crlf");
    }
    let c_lf = content.replace("\r\n", "\n");
    if c_lf == *out {
        return dedup(tags);
    }
    if trailing_newlines(&c_lf) != trailing_newlines(out) {
        tags.push("trailing-newline");
    }

    // Work on the lines with any trailing newline(s) removed so the trailing-\n
    // difference doesn't masquerade as a blank-line difference.
    let cl: Vec<&str> = c_lf.trim_end_matches('\n').split('\n').collect();
    let ol: Vec<&str> = out.trim_end_matches('\n').split('\n').collect();

    let c_blanks = cl.iter().filter(|l| l.trim().is_empty()).count();
    let o_blanks = ol.iter().filter(|l| l.trim().is_empty()).count();
    if c_blanks != o_blanks {
        tags.push("interior-blank-lines");
    }

    let cn: Vec<&str> = cl.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    let on: Vec<&str> = ol.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    if cn.iter().map(|l| l.trim()).ne(on.iter().map(|l| l.trim())) {
        tags.push("content-change");
        return dedup(tags);
    }
    // Same content lines (trimmed); diagnose the whitespace differences.
    for (a, b) in cn.iter().zip(&on) {
        if a.trim_end() != b.trim_end() && a.trim() == b.trim() {
            tags.push("trailing-ws");
        }
        let la = leading(a);
        let lb = leading(b);
        if la != lb {
            let ca = la.chars().next();
            let cb = lb.chars().next();
            if ca.is_some() && cb.is_some() && ca != cb {
                tags.push("indent-char");
            } else if la.len() != lb.len() {
                // bullet lines vs continuation lines indent differently
                tags.push(if a.trim_start().starts_with("- ") { "indent-width" } else { "cont-indent" });
            }
        }
    }
    dedup(tags)
}

fn leading(l: &str) -> &str {
    let n = l.len() - l.trim_start_matches([' ', '\t']).len();
    &l[..n]
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
