use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tine_store::{model::Graph, Area, Day, Store};

fn fixture() -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-scan-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    (
        root.clone(),
        Store::from_legacy(Arc::new(Graph::open(&root))),
    )
}

#[test]
fn scan_is_sorted_skips_hidden_reports_unreadable_and_limits_meta() {
    let (root, store) = fixture();
    fs::create_dir_all(root.join("assets/nested")).unwrap();
    for name in ["z.png", "a.png", ".hidden", "nested/b.png"] {
        fs::write(root.join("assets").join(name), b"x").unwrap();
    }
    let listing = store.scan_area(Area::Assets, None).unwrap();
    assert_eq!(
        listing
            .files
            .iter()
            .map(|f| f.rel.as_str())
            .collect::<Vec<_>>(),
        vec!["a.png", "nested/b.png", "z.png"]
    );
    assert_eq!(listing.files[0].meta.as_ref().unwrap().len, 1);
    assert!(store
        .scan_area(Area::Assets, Some("absent"))
        .unwrap()
        .files
        .is_empty());
    assert!(store.scan_area(Area::Assets, Some("../escape")).is_err());
    assert_eq!(
        store
            .scan_area(Area::Assets, Some("a.png"))
            .unwrap()
            .unreadable
            .len(),
        1
    );
    fs::write(root.join("logseq/config.edn"), b"{}").unwrap();
    fs::write(root.join("logseq/custom.css"), b"body{}").unwrap();
    fs::write(root.join("logseq/other.txt"), b"x").unwrap();
    assert_eq!(
        store
            .scan_area(Area::Meta, None)
            .unwrap()
            .files
            .iter()
            .map(|f| f.rel.as_str())
            .collect::<Vec<_>>(),
        vec!["config.edn", "custom.css"]
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("assets/nested"), root.join("assets/linked")).unwrap();
        assert!(!store
            .scan_area(Area::Assets, None)
            .unwrap()
            .files
            .iter()
            .any(|f| f.rel.starts_with("linked/")));
        assert!(store
            .scan_area(Area::Assets, Some("linked"))
            .unwrap()
            .files
            .is_empty());
    }
}

#[test]
fn referenced_assets_keeps_raw_decoded_and_nested_first_segment() {
    let (root, store) = fixture();
    fs::write(
        root.join("pages/Refs.md"),
        "- ![](../assets/my%20file.png)\n- ![](../assets/pdfkey/crop.png)\n",
    )
    .unwrap();
    let names = store.whole_graph().unwrap().referenced_assets();
    for name in ["my%20file.png", "my file.png", "pdfkey", "pdfkey/crop.png"] {
        assert!(names.contains(name), "{name}");
    }
}

#[test]
fn journal_scan_and_canonical_id_follow_configured_format() {
    let (root, store) = fixture();
    for name in ["2026_06_18.md", "Jun 18th, 2026.org", "notes.txt"] {
        fs::write(root.join("journals").join(name), b"- body\n").unwrap();
    }
    fs::write(root.join("pages/2026_06_18.md"), b"- page\n").unwrap();
    let listing = store.scan_area(Area::Journals, None).unwrap();
    let dated = listing
        .files
        .iter()
        .find(|entry| entry.rel == "2026_06_18.md")
        .unwrap();
    assert_eq!(dated.day, Some(Day(20260618)));
    assert!(dated.date_stem);
    let titled = listing
        .files
        .iter()
        .find(|entry| entry.rel == "Jun 18th, 2026.org")
        .unwrap();
    assert_eq!(titled.day, Some(Day(20260618)));
    assert!(!titled.date_stem);
    assert_eq!(
        listing
            .files
            .iter()
            .find(|entry| entry.rel == "notes.txt")
            .unwrap()
            .day,
        None
    );
    assert_eq!(
        store.scan_area(Area::Pages, None).unwrap().files[0].day,
        None
    );
    assert_eq!(
        store.journal_id(Day(20260618)).as_str(),
        "journals/2026_06_18.md"
    );

    fs::write(
        root.join("logseq/config.edn"),
        b"{:journal/file-name-format \"yyyy-MM-dd\"}",
    )
    .unwrap();
    fs::write(root.join("journals/2026-06-19.org"), b"- custom\n").unwrap();
    fs::write(root.join("journals/Jun 19th, 2026.md"), b"- title\n").unwrap();
    let custom = Store::from_legacy(Arc::new(Graph::open(&root)));
    let listing = custom.scan_area(Area::Journals, None).unwrap();
    assert!(
        listing
            .files
            .iter()
            .find(|entry| entry.rel == "2026-06-19.org")
            .unwrap()
            .date_stem
    );
    assert!(
        !listing
            .files
            .iter()
            .find(|entry| entry.rel == "Jun 19th, 2026.md")
            .unwrap()
            .date_stem
    );
    assert_eq!(
        custom.journal_id(Day(20260619)).as_str(),
        "journals/2026-06-19.org"
    );
}
