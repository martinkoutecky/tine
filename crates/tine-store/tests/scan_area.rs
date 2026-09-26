use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_store::{Area, Day, Store};

fn put(root: &std::path::Path, rel: &str, bytes: &[u8]) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let temp = root.join(format!(
        ".scan-fixture-{}",
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temp, bytes).unwrap();
    fs::rename(temp, root.join(rel)).unwrap();
}

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
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

#[test]
fn scan_is_sorted_skips_hidden_reports_unreadable_and_limits_meta() {
    let (root, store) = fixture();
    fs::create_dir_all(root.join("assets/nested")).unwrap();
    for name in ["z.png", "a.png", ".hidden", "nested/b.png"] {
        put(&root, &format!("assets/{name}"), b"x");
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
    put(&root, "logseq/config.edn", b"{}");
    put(&root, "logseq/custom.css", b"body{}");
    put(&root, "logseq/other.txt", b"x");
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
    put(
        &root,
        "pages/Refs.md",
        b"- ![](../assets/my%20file.png)\n- ![](../assets/pdfkey/crop.png)\n",
    );
    store.scan_refresh().unwrap();
    let names = store.whole_graph().unwrap().referenced_assets();
    for name in ["my%20file.png", "my file.png", "pdfkey", "pdfkey/crop.png"] {
        assert!(names.contains(name), "{name}");
    }
}

#[test]
fn journal_scan_and_canonical_id_follow_configured_format() {
    let (root, store) = fixture();
    for name in ["2026_06_18.md", "Jun 18th, 2026.org", "notes.txt"] {
        put(&root, &format!("journals/{name}"), b"- body\n");
    }
    put(&root, "pages/2026_06_18.md", b"- page\n");
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

    put(
        &root,
        "logseq/config.edn",
        b"{:journal/file-name-format \"yyyy-MM-dd\"}",
    );
    put(&root, "journals/2026-06-19.org", b"- custom\n");
    put(&root, "journals/Jun 19th, 2026.md", b"- title\n");
    let custom = Store::open(&root, Default::default()).unwrap().0;
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

#[test]
fn journal_equal_rank_uses_extension_then_byte_order() {
    let (root, early) = fixture();
    drop(early);
    for name in ["Jun 18th, 2026.org", "Jun 18th, 2026.md", "2026-06-18.md"] {
        put(&root, &format!("journals/{name}"), b"- body\n");
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(
        store.journal_id(Day(20260618)).as_str(),
        "journals/2026-06-18.md"
    );
    fs::remove_file(root.join("journals/2026-06-18.md")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(
        store.journal_id(Day(20260618)).as_str(),
        "journals/Jun 18th, 2026.md"
    );
}

#[cfg(unix)]
#[test]
fn source_area_scan_does_not_follow_linked_directories() {
    let (root, store) = fixture();
    let outside = root.with_extension("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("Secret.md"), b"- outside\n").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("pages/linked")).unwrap();
    let listing = store.scan_area(Area::Pages, None).unwrap();
    assert!(listing
        .files
        .iter()
        .all(|file| !file.rel.starts_with("linked/")));
    fs::remove_dir_all(outside).unwrap();
}
