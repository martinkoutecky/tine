//! I-22: every production lsdoc parse goes through the bounded boundary in
//! `crates/tine-outline/src/lsdoc_block_parse.rs` (the exemplar: `parse_block`,
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
    "parse_format_without_property_page_refs",
    "property_references",
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
    (
        "crates/tine-outline/src/lsdoc_block_parse.rs",
        "the bounded boundary itself",
    ),
    (
        "crates/tine-core/src/query/conformance.rs",
        "a #[cfg(test)] module (asserted below)",
    ),
];

/// Functions allowed to call lsdoc directly, by file and name, each with its
/// reason. A call anywhere else in the same file is a hit.
const EXEMPT_FNS: &[(&str, &str, &str)] = &[(
    "crates/tine-text/src/bound.rs",
    "parse_outline_bounded",
    "the page outline parse under the same quote bound",
)];

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
        hits.extend(file_hits(&relative, &text));
    }
}

/// The raw calls in production file `relative`, outside its exempt functions
/// and its `#[cfg(test)]` items.
fn file_hits(relative: &str, text: &str) -> Vec<String> {
    let exempt: Vec<&str> = (EXEMPT_FNS.iter())
        .filter(|(file, ..)| *file == relative)
        .map(|(_, name, _)| *name)
        .collect();
    let (mut hits, mut depth) = (Vec::new(), 0usize);
    // An item not scanned (an exempt function, or the item a `#[cfg(test)]`
    // attribute applies to): the depth it began at, and whether its body
    // opened. It ends where its braces close back to that depth, or at a `;`
    // at that depth before any brace opens (an item with no body).
    let mut skipped: Option<(usize, bool)> = None;
    for (n, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let declares = |name: &&str| {
            let words: Vec<&str> = code
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .collect();
            words.windows(2).any(|w| w == ["fn", *name])
        };
        let test_item = line.trim_start().starts_with("#[cfg(test)]");
        if skipped.is_none() && (test_item || exempt.iter().any(declares)) {
            skipped = Some((depth, false));
        }
        if skipped.is_none() {
            for entry in raw_calls(code) {
                hits.push(format!(
                    "{relative}:{}: raw `lsdoc::{entry}` in `{}`",
                    n + 1,
                    line.trim()
                ));
            }
        }
        for c in code.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
            if let Some((at, opened)) = &mut skipped {
                *opened |= depth > *at;
                if (*opened && depth <= *at) || (!*opened && c == ';') {
                    skipped = None;
                }
            }
        }
    }
    hits
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
        "I-22: call lsdoc only through crates/tine-outline/src/lsdoc_block_parse.rs (exemplar: \
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

#[test]
fn an_exempt_function_exempts_only_its_own_body() {
    let file = "crates/tine-text/src/bound.rs";
    let bounded = "pub fn parse_outline_bounded(text: &str) -> Option<()> {\n    \
                   lsdoc::parse_outline(text, \"md\").ok().map(|_| ())\n}\n";
    assert!(file_hits(file, bounded).is_empty());
    let second =
        format!("{bounded}fn other(t: &str) {{\n    lsdoc::parse_outline(t, \"md\");\n}}\n");
    assert_eq!(file_hits(file, &second).len(), 1, "{second}");
    let before = format!("fn early(t: &str) {{ lsdoc::parse(t); }}\n{bounded}");
    assert_eq!(file_hits(file, &before).len(), 1, "{before}");
    assert_eq!(file_hits("crates/tine-text/src/lib.rs", bounded).len(), 1);
}

/// A one-line exempt function closes its exemption on its own line, and a
/// `#[cfg(test)]` item in the middle of a file exempts that item only: a raw
/// call after either is a hit.
#[test]
fn the_scanner_ends_each_exemption_where_its_item_ends() {
    let file = "crates/tine-text/src/bound.rs";
    let one_line = "pub fn parse_outline_bounded(t: &str) -> bool { lsdoc::parse_outline(t, \"md\").is_ok() }\n\
                    fn other(t: &str) {\n    lsdoc::parse(t);\n}\n";
    assert_eq!(file_hits(file, one_line).len(), 1, "{one_line}");
    let lib = "crates/tine-text/src/lib.rs";
    let middle = "#[cfg(test)]\nthread_local! {\n    static N: u8 = 0;\n}\n\
                  #[cfg(test)]\nstatic M: u8 = 0;\n\
                  impl Map {\n    fn live(t: &str) {\n        lsdoc::parse(t);\n    }\n}\n\
                  #[cfg(test)]\nmod tests {\n    fn probe() {\n        lsdoc::parse(\"\");\n    }\n}\n";
    assert_eq!(file_hits(lib, middle).len(), 1, "{middle}");
}
