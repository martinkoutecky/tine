//! Read-only acceptance gate; run against a temporary copy of an anonymized graph.
use std::path::Path;
use tine_core::{
    doc,
    editor_text::{self, HidePolicy},
};

fn check(block: &doc::DocBlock, counts: &mut [usize; 3]) {
    counts[1] += 1;
    let org = block.is_org();
    let regions = tine_core::block_regions::parse(block.raw(), org);
    let native_logbook = regions
        .drawers
        .iter()
        .any(|d| d.name.eq_ignore_ascii_case("LOGBOOK"));
    if !native_logbook && regions.header.marker.is_none() {
        for child in &block.children {
            check(child, counts);
        }
        return;
    }
    // The anonymizer currently removes clock history. Exercise its real task
    // bodies with a deterministic clock added in memory; never alter the copy.
    let clocked;
    let raw = if native_logbook {
        block.raw()
    } else {
        clocked = regions
            .insert_drawer(
                block.raw(),
                org,
                ":LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00:00]\n:END:",
            )
            .unwrap();
        &clocked
    };
    counts[2] += 1;
    let parts = editor_text::split(raw, org, &HidePolicy::Builtin);
    assert_eq!(
        editor_text::set_text(raw, org, &parts.visible).unwrap(),
        raw,
        "LOGBOOK block {} roundtrip",
        counts[2]
    );
    // Insert visible text at the title's start, edit it back, and require every
    // pre-existing byte (including the hidden clock history) to survive.
    let edited = editor_text::set_text(raw, org, &format!("edited {}", parts.visible)).unwrap();
    assert_eq!(
        editor_text::set_text(&edited, org, &parts.visible).unwrap(),
        raw,
        "LOGBOOK block {} edit",
        counts[2]
    );
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
        .expect("usage: logbook-editor-corpus <graph-copy>");
    let mut counts = [0; 3];
    walk(Path::new(&path), &mut counts);
    assert!(counts[2] > 0, "corpus must exercise LOGBOOKs");
    println!("files={} blocks={} clocked_task_blocks={} byte_exact=PASS (clock history injected in memory where anonymized away)", counts[0], counts[1], counts[2]);
}
