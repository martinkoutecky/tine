//! Walk a directory, parse+serialize every `.md`, and report any byte-level
//! mismatches. Usage: cargo run -p tine-core --example roundtrip_dir -- <dir>

use std::path::Path;
use tine_core::doc;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: roundtrip_dir <dir>");
    let mut total = 0usize;
    let mut byte_diff = 0usize;
    let mut structural_bugs = 0usize;
    walk(Path::new(&dir), &mut |path, content| {
        total += 1;
        let parsed = doc::parse(content);
        let out = doc::serialize(&parsed);
        // The real correctness bar: no structure/data lost. Serialization is
        // the canonical form, so re-parsing it must yield the same Document.
        let reparsed = doc::parse(&out);
        if reparsed != parsed {
            structural_bugs += 1;
            println!("STRUCTURAL BUG: {}", path.display());
            print_first_diff(content, &out);
        } else if out != *content {
            // Acceptable canonicalization (tabs, trailing newline, etc.).
            byte_diff += 1;
        }
    });
    println!(
        "\n{total} files | {structural_bugs} structural bugs | {byte_diff} canonicalized (acceptable) | {} byte-identical",
        total - byte_diff - structural_bugs
    );
}

fn walk(dir: &Path, f: &mut impl FnMut(&Path, &String)) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, f);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                f(&path, &content);
            }
        }
    }
}

fn print_first_diff(a: &str, b: &str) {
    for (i, (la, lb)) in a.lines().zip(b.lines()).enumerate() {
        if la != lb {
            println!("  line {}: IN  {:?}", i + 1, la);
            println!("  line {}: OUT {:?}", i + 1, lb);
            return;
        }
    }
    if a.lines().count() != b.lines().count() {
        println!(
            "  line count differs: in={} out={}",
            a.lines().count(),
            b.lines().count()
        );
    }
}
