//! Regression: full-text search reflects a marker toggle once the edited page is
//! saved back (the path a {{query}}-result edit takes).
use tine_core::{Graph, PageKind};

fn mk(tag: &str) -> std::path::PathBuf {
    // Unique per test (pid + tag) so parallel tests don't share a dir.
    let root = std::env::temp_dir().join(format!("tine-se-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    root
}

#[test]
fn search_reflects_toggle_on_named_page() {
    let root = mk("named");
    std::fs::write(root.join("pages").join("Tasks.md"), "- TODO ship the thing\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let mut dto = g.load_named("Tasks", PageKind::Page).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DOING");
    g.save_page(&dto).expect("save");
    assert!(!g.search("DOING", 20).is_empty(), "named: DOING found after save");
    assert!(g.search("TODO", 20).is_empty(), "named: TODO gone after toggle");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_reflects_toggle_on_journal_page() {
    let root = mk("journal");
    std::fs::write(root.join("journals").join("2026_06_16.md"), "- TODO ship the thing\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let title = g.run_query("(task TODO)")[0].page.clone();
    let mut dto = g.load_named(&title, PageKind::Journal).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DOING");
    g.save_page(&dto).expect("journal save");
    assert!(!g.search("DOING", 20).is_empty(), "journal: DOING found after save");
    assert!(g.search("TODO", 20).is_empty(), "journal: TODO gone after toggle");
    let _ = std::fs::remove_dir_all(&root);
}

// The watcher must recognize Tine's own writes: after save_page, sync_file on
// that file returns None (no phantom external-change → no false conflict).
#[test]
fn own_write_is_suppressed_by_watcher() {
    let root = mk("selfwrite");
    let path = root.join("pages").join("Notes.md");
    // A block WITHOUT id:: (the common case): cache uuid is generated, disk has none.
    std::fs::write(&path, "- TODO ship the thing\n- another line\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();

    // Before any edit, an unchanged file is already suppressed.
    assert!(g.sync_file(&path).is_none(), "unchanged file → suppressed");

    // Edit + save through the normal path.
    let mut dto = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DOING");
    g.save_page(&dto).expect("save");

    // The watcher polling this file must see it as OUR write, not external.
    assert!(g.sync_file(&path).is_none(), "own write → suppressed (no phantom graph-changed)");

    // A genuine external change is still detected.
    std::fs::write(&path, "- DOING ship the thing\n- edited by hand\n").unwrap();
    assert!(g.sync_file(&path).is_some(), "external edit → detected");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn journal_content_days_distinguishes_empty() {
    let root = mk("contentdays");
    // Non-empty journal, an empty one (placeholder bullet), and a props-only one.
    std::fs::write(root.join("journals").join("2026_06_16.md"), "- went for a walk\n").unwrap();
    std::fs::write(root.join("journals").join("2026_06_15.md"), "- \n").unwrap();
    std::fs::write(root.join("journals").join("2026_06_14.md"), "title:: x\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let days = g.journal_content_days();
    assert!(days.contains(&20260616), "non-empty day present: {days:?}");
    assert!(!days.contains(&20260615), "empty bullet day absent");
    assert!(!days.contains(&20260614), "props-only day absent");
    let _ = std::fs::remove_dir_all(&root);
}
