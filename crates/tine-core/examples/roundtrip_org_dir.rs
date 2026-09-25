//! Org counterpart of `roundtrip_dir`: walk a directory, parse+serialize every
//! `.org`, report round-trip status per the same gate the app uses
//! (`org_round_trips` — a page that fails it is read-only in Tine).
//! Usage: cargo run -p tine-core --example roundtrip_org_dir -- <dir>

use std::path::Path;
use tine_core::org;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: roundtrip_org_dir <dir>");
    let mut total = 0usize;
    let mut read_only = 0usize;
    walk(Path::new(&dir), &mut |path, content| {
        total += 1;
        if !org::org_round_trips(content) {
            read_only += 1;
            println!("NOT ROUND-TRIPPING (read-only in Tine): {}", path.display());
        }
    });
    println!(
        "\n{total} org files | {read_only} read-only | {} round-trip clean",
        total - read_only
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
        } else if path.extension().and_then(|e| e.to_str()) == Some("org") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                f(&path, &content);
            }
        }
    }
}
