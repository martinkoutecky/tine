//! Read-only acceptance probe. Reports counts, never graph content.
use std::path::Path;
use tine_core::{
    block_regions, doc,
    editor_text::{self, HidePolicy},
};

fn check(block: &doc::DocBlock, counts: &mut [usize; 3]) {
    counts[1] += 1;
    let org = block.is_org();
    let parts = editor_text::split(block.raw(), org, &HidePolicy::Builtin);
    if !parts.hidden.is_empty() {
        counts[2] += 1;
        for text in [format!("Edited {}", parts.visible), String::new()] {
            let joined = editor_text::join_preserving(
                &text,
                &parts.hidden,
                org,
                block.raw(),
                &HidePolicy::Builtin,
            )
            .expect("accepted hidden metadata must survive an edit");
            assert!(
                editor_text::split(&joined, org, &HidePolicy::Builtin).hidden == parts.hidden,
                "hidden metadata bytes changed in corpus block {}",
                counts[1]
            );
            // Review F5: the rows must stay where OG reads them. An edit may not
            // split the block's properties into more groups (OG keeps the first).
            let groups = |raw: &str| block_regions::parse(raw, org).property_regions.len();
            assert!(
                groups(&joined) <= groups(block.raw()).max(1),
                "an edit split the property group in corpus block {}",
                counts[1]
            );
            if !text.is_empty() {
                // A visible prefix edit must splice into the first visible
                // source position, leaving every hidden row in its original
                // group and order. Group counts alone admitted 2 -> 2 moves.
                let raw = block.raw();
                let at = editor_text::visible_to_raw_with(
                    raw,
                    org,
                    &block_regions::parse(raw, org),
                    &HidePolicy::Builtin,
                    0,
                );
                let expected = format!("{}Edited {}", &raw[..at], &raw[at..]);
                assert!(
                    joined == expected,
                    "visible edit moved metadata in corpus block {}",
                    counts[1]
                );
            }
        }
    }
    for child in &block.children {
        check(child, counts);
    }
}

fn walk(path: &Path, counts: &mut [usize; 3]) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, counts);
            continue;
        }
        let org = match path.extension().and_then(|s| s.to_str()) {
            Some("md") => false,
            Some("org") => true,
            _ => continue,
        };
        let raw = std::fs::read_to_string(path).unwrap();
        let page = if org {
            tine_core::org::parse_org(&raw)
        } else {
            doc::parse(&raw)
        };
        counts[0] += 1;
        for block in &page.roots {
            check(block, counts);
        }
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("property-hiding-corpus <graph-copy>");
    let mut counts = [0; 3];
    walk(Path::new(&path), &mut counts);
    assert!(counts[2] > 0, "corpus must exercise hidden properties");
    println!(
        "files={} blocks={} hidden_blocks={} edit_and_clear_bytes=PASS",
        counts[0], counts[1], counts[2]
    );
}
