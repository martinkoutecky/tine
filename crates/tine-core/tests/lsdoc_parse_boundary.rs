//! I-22: every production lsdoc parse goes through the bounded boundary in
//! `crates/lsdoc-block-parse.rs` (the exemplar: `parse_block`,
//! `parse_projection`, `parse_text_bounded`, `parse_inline_bounded`). lsdoc
//! recurses while parsing a quote staircase and returns trees whose drop and
//! walk recurse, so a raw call on synced or imported text (a property value,
//! a whole file, a query atom) aborted the process on a deep `>>>…` value
//! (og C3 L03). A match below means a raw call crept back: call the boundary
//! through `tine_core::render` (or the included module) instead.

use std::path::Path;

/// lsdoc entry points that parse text.
const ENTRIES: &[&str] = &[
    "parse",
    "parse_format",
    "parse_outline",
    "refs",
    "inline",
    "parse_to_projection",
    "parse_org_to_projection",
    "__parse_org_streaming",
    "__parse_format_legacy",
    "__parse_format_v2",
    "__try_parse_format_v2",
    "__inline_v2",
];

/// Files allowed to call lsdoc directly, each with its reason.
const EXEMPT: &[(&str, &str)] = &[
    ("crates/lsdoc-block-parse.rs", "the bounded boundary itself"),
    (
        "crates/tine-core/src/query/conformance.rs",
        "a #[cfg(test)] module (asserted below)",
    ),
];

fn raw_calls(code: &str) -> Vec<&'static str> {
    ENTRIES
        .iter()
        .copied()
        .filter(|entry| {
            let call = format!("lsdoc::{entry}(");
            let import = code.contains("use ")
                && code.contains("lsdoc::")
                && code
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|word| word == *entry)
                && !code.contains("lsdoc::ast");
            code.contains(&call) || import
        })
        .collect()
}

fn scan(root: &Path, dir: &Path, hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            if !matches!(
                path.file_name().and_then(|n| n.to_str()),
                Some("tests" | "target")
            ) {
                scan(root, &path, hits);
            }
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") || EXEMPT.iter().any(|(f, _)| *f == relative)
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (n, line) in text.lines().enumerate() {
            // Unit tests sit at the end of a production file.
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            let code = line.split("//").next().unwrap_or("");
            for entry in raw_calls(code) {
                hits.push(format!(
                    "{relative}:{}: raw `lsdoc::{entry}` in `{}`",
                    n + 1,
                    line.trim()
                ));
            }
        }
    }
}

#[test]
fn every_production_lsdoc_parse_goes_through_the_bounded_boundary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap();
    let mut hits = Vec::new();
    scan(&root, &root.join("crates"), &mut hits);
    scan(&root, &root.join("src-tauri/src"), &mut hits);
    assert!(
        hits.is_empty(),
        "I-22: call lsdoc only through crates/lsdoc-block-parse.rs (exemplar: \
         `parse_text_bounded` / `parse_inline_bounded`, reached via tine_core::render), \
         which refuses a quote staircase before lsdoc recurses and drains too-deep \
         trees iteratively:\n{}",
        hits.join("\n")
    );
    let query = std::fs::read_to_string(root.join("crates/tine-core/src/query.rs")).unwrap();
    assert!(
        query.contains("#[cfg(test)]\nmod conformance;"),
        "conformance.rs is exempt only while it stays a #[cfg(test)] module"
    );
}

#[test]
fn the_scanner_sees_raw_calls_and_imports() {
    assert_eq!(
        raw_calls("let p = lsdoc::parse_format(text, fmt);"),
        ["parse_format"]
    );
    assert_eq!(
        raw_calls("let n = tine_core::lsdoc::inline(t, \"md\");"),
        ["inline"]
    );
    assert_eq!(
        raw_calls("use lsdoc::{inline, parse};"),
        ["parse", "inline"]
    );
    assert!(raw_calls("let p = crate::render::parse_text_bounded(v, false);").is_empty());
    assert!(raw_calls("use lsdoc::ast::{Block, Inline};").is_empty());
}
