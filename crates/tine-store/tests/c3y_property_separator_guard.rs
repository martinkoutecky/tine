//! og C3Y guard (C3W W3 class): OG's `sep-by-comma` splits linkable property
//! values (`alias::`, `tags::`, references) on `,` and the full-width `，`.
//! Five readers each spelled that set by hand and one (`page_facets` tags)
//! forgot `，`, so `tags:: A，B` became one tag there and two everywhere else.
//! The set now has one definition: `tine_core::refs::is_linkable_property_separator`.
use std::fs;
use std::path::{Path, PathBuf};

const EXEMPLAR: &str = "crates/tine-core/src/refs.rs";

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn full_width_comma_separator_has_one_definition() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["crates", "src-tauri/src"] {
        sources(&root.join(dir), &mut files);
    }
    let offenders: Vec<String> = files
        .iter()
        .filter(|path| !path.ends_with("tests/c3y_property_separator_guard.rs"))
        .filter(|path| !path.components().any(|c| c.as_os_str() == "target"))
        .flat_map(|path| {
            let text = fs::read_to_string(path).unwrap_or_default();
            let rel = path.strip_prefix(&root).unwrap_or(path).display().to_string();
            text.lines()
                .enumerate()
                .filter(|(_, line)| line.contains("'，'"))
                .map(|(n, _)| format!("{rel}:{}", n + 1))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        offenders,
        vec![format!("{EXEMPLAR}:{}", exemplar_line(&root))],
        "the linkable-property separator set is spelled once, in {EXEMPLAR} \
         (`is_linkable_property_separator`); call it instead of listing `，` again"
    );
}

fn exemplar_line(root: &Path) -> usize {
    let text = fs::read_to_string(root.join(EXEMPLAR)).unwrap();
    text.lines().position(|line| line.contains("'，'")).unwrap() + 1
}
