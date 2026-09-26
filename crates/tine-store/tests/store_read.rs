use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tine_core::model::{PageDto, PageKind};
use tine_store::{Area, OpenOptions, PageId, Resolved, Store, StoreError};

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-store-read-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("pages/Note.md"), "- before\n").unwrap();
        std::fs::write(root.join("pages/Bad.md"), [0xff]).unwrap();
        std::fs::write(root.join("pages/Org.org"), "* a\n*** c\n").unwrap();
        std::fs::write(root.join("assets/pic.bin"), b"abcdef").unwrap();
        Self(root)
    }

    fn store(&self) -> Store {
        Store::open(&self.0, OpenOptions::default()).unwrap().0
    }

    fn put(&self, rel: &str, bytes: &[u8]) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp = self.0.join(format!(
            ".read-fixture-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&temp, bytes).unwrap();
        std::fs::rename(temp, self.0.join(rel)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn page_reads_and_publishes_external_edit() {
    let f = Fixture::new();
    let store = f.store();
    let id = PageId::from("pages/Note.md");
    let first = store.page(&id).unwrap();
    assert_eq!(first.id, id);
    assert_eq!(first.doc.blocks[0].raw, "before");
    assert_eq!(first.doc.kind, PageKind::Page);
    assert!(first.read_only.is_none());
    assert_eq!(
        serde_json::to_string(&first.rev).unwrap(),
        serde_json::to_string(&store.read(&id.file(), None).unwrap().1).unwrap()
    );
    let _ = store.whole_graph().unwrap().complete_page_names("Note", 10);
    f.put("pages/Note.md", b"- after edit\n");
    store.scan_refresh().unwrap();
    let second = store.page(&id).unwrap();
    assert_eq!(second.doc.blocks[0].raw, "after edit");
    assert_ne!(first.rev, second.rev);
    assert!(
        store
            .whole_graph()
            .unwrap()
            .search(
                &tine_store::SearchRequest {
                    text: "after edit".into(),
                    within: None,
                    page_limit: 10,
                    block_limit: 10,
                    explain: false,
                },
                &tine_store::Cancel(Arc::new(false.into()))
            )
            .unwrap()
            .hits
            .len()
            > 0
    );
}

#[test]
fn page_reports_invalid_missing_and_undecodable() {
    let f = Fixture::new();
    let store = f.store();
    assert!(matches!(
        store.page(&PageId::from("pages/missing.md")),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.page(&PageId::from("pages/../Bad.md")),
        Err(StoreError::InvalidTarget(_))
    ));
    assert!(matches!(
        store.page(&PageId::from("pages/Bad.md")),
        Err(StoreError::Undecodable)
    ));
    let org = store.page(&PageId::from("pages/Org.org")).unwrap();
    assert_eq!(org.doc.format, tine_core::model::Format::Org);
    assert!(org.read_only.is_some());
    assert!(org.doc.read_only);
}

#[cfg(unix)]
#[test]
fn page_symlinks_are_not_pages() {
    // v0.6.5's walker skips symlinked page files; neither route reads one.
    let f = Fixture::new();
    let outside = f.0.with_extension("outside.md");
    std::fs::write(&outside, "- outside graph\n").unwrap();
    std::os::unix::fs::symlink(&outside, f.0.join("pages/Outside.md")).unwrap();
    std::os::unix::fs::symlink(f.0.join("pages/Note.md"), f.0.join("pages/Linked.md")).unwrap();
    let store = f.store();
    for name in ["Outside", "Linked"] {
        let id = PageId::from(format!("pages/{name}.md"));
        assert!(matches!(store.page(&id), Err(StoreError::InvalidTarget(_))));
        assert!(matches!(
            store.whole_graph().unwrap().resolve(name, false),
            Resolved::Absent { .. }
        ));
    }
    std::fs::remove_file(outside).unwrap();
}

#[test]
fn byte_reads_streaming_and_os_handoff() {
    let f = Fixture::new();
    let store = f.store();
    let id = store.file_id(Area::Assets, "pic.bin").unwrap();
    assert_eq!(store.read(&id, None).unwrap().0, b"abcdef");
    assert_eq!(store.read(&id, Some(6)).unwrap().0, b"abcdef");
    assert!(matches!(
        store.read(&id, Some(5)),
        Err(StoreError::TooLarge { limit: 5, len: 6 })
    ));
    assert_eq!(store.open_read(&id).unwrap().1, 6);
    // Hand-off paths live under the canonical root `Store::open` binds (on
    // Windows a `\\?\` long-name path; on macOS `/private/var`).
    let root = std::fs::canonicalize(&f.0).unwrap();
    assert_eq!(
        store.path_for_os_handoff(&id, false).unwrap(),
        root.join("assets").join("pic.bin")
    );
    let future = store.file_id(Area::Pages, "future.md").unwrap();
    assert_eq!(
        store.path_for_os_handoff(&future, false).unwrap(),
        root.join("pages").join("future.md")
    );
    assert!(matches!(
        store.path_for_os_handoff(&PageId::from("pages/../escape.md").file(), false),
        Err(StoreError::InvalidTarget(_))
    ));
}

#[cfg(unix)]
#[test]
fn streaming_refuses_symlinked_asset() {
    let f = Fixture::new();
    std::os::unix::fs::symlink(f.0.join("assets/pic.bin"), f.0.join("assets/link.bin")).unwrap();
    let store = f.store();
    let id = store.file_id(Area::Assets, "link.bin").unwrap();
    assert!(matches!(
        store.open_read(&id),
        Err(StoreError::InvalidTarget(_))
    ));
    std::os::unix::fs::symlink(f.0.parent().unwrap(), f.0.join("pages/outside")).unwrap();
    let escaping = PageId::from("pages/outside/next.md");
    assert!(matches!(
        store.path_for_os_handoff(&escaping.file(), false),
        Err(StoreError::InvalidTarget(_))
    ));
}

#[test]
fn page_id_keeps_string_wire_form_beside_a_pathless_dto() {
    // The file identity travels as `PageRead.id` (a plain string on the wire),
    // not inside the DTO: the DTO no longer carries a `path`.
    let f = Fixture::new();
    let store = f.store();
    let read = store.page(&PageId::from("pages/Note.md")).unwrap();
    assert_eq!(
        serde_json::to_string(&read.id).unwrap(),
        "\"pages/Note.md\""
    );
    assert_eq!(
        serde_json::from_str::<PageId>("\"pages/Note.md\"").unwrap(),
        read.id
    );
    let real = serde_json::to_string(&read.doc).unwrap();
    assert!(!real.contains("\"path\""), "{real}");
    assert_eq!(
        serde_json::to_string(&serde_json::from_str::<PageDto>(&real).unwrap()).unwrap(),
        real
    );
}

#[test]
fn graph_meta_omits_layout_directories() {
    let f = Fixture::new();
    let (_, meta, _) = Store::open(&f.0, OpenOptions::default()).unwrap();
    let value = serde_json::to_value(meta).unwrap();
    assert!(value.get("pages_dir").is_none(), "{value}");
    assert!(value.get("journals_dir").is_none(), "{value}");
}

#[test]
fn page_mtime_remains_the_observed_snapshot_value() {
    let f = Fixture::new();
    let store = f.store();
    let id = PageId::from("pages/Note.md");
    let view = store.whole_graph().unwrap();
    let observed = view.page_mtime(&id).unwrap();
    let later = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    // Setting times needs write access on Windows (FILE_WRITE_ATTRIBUTES).
    std::fs::File::options()
        .write(true)
        .open(f.0.join("pages/Note.md"))
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert_ne!(observed, later);
    assert_eq!(view.page_mtime(&id), Some(observed));
    store.scan_refresh().unwrap();
    assert_eq!(store.whole_graph().unwrap().page_mtime(&id), Some(later));
}

#[test]
fn unreadable_files_reports_skipped_non_utf8_page() {
    let f = Fixture::new();
    let store = f.store();
    let view = store.whole_graph().unwrap();
    assert!(view
        .unreadable_files()
        .iter()
        .any(|(id, reason)| id.as_str() == "pages/Bad.md" && !reason.is_empty()));
    assert!(view
        .corpus()
        .pages
        .iter()
        .all(|page| page.id != "pages/Bad.md"));
    f.put("pages/Bad.md", b"- now readable\n");
    store.scan_refresh().unwrap();
    assert!(store
        .whole_graph()
        .unwrap()
        .unreadable_files()
        .iter()
        .all(|(id, _)| id.as_str() != "pages/Bad.md"));
    assert!(view
        .unreadable_files()
        .iter()
        .any(|(id, _)| id.as_str() == "pages/Bad.md"));
}

#[cfg(unix)]
#[test]
fn unreadable_files_reports_unlistable_page_directory() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let nested = f.0.join("pages/nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0)).unwrap();
    let store = f.store();
    let view = store.whole_graph().unwrap();
    assert!(view
        .unreadable_files()
        .iter()
        .any(|(id, reason)| { id.as_str() == "pages/nested" && !reason.is_empty() }));
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
    store.close();
}

#[test]
fn resolve_then_page_covers_titles_aliases_namespaces_and_journals() {
    let f = Fixture::new();
    std::fs::write(
        f.0.join("pages/Title.md"),
        "title:: Display Title\n\n- body\n",
    )
    .unwrap();
    std::fs::write(f.0.join("pages/Owner.md"), "alias:: Also Owner\n\n- body\n").unwrap();
    std::fs::write(f.0.join("pages/Parent%2FChild.md"), "- nested name\n").unwrap();
    std::fs::write(f.0.join("journals/2026_09_25.md"), "- journal body\n").unwrap();
    let store = f.store();
    let view = store.whole_graph().unwrap();
    for (name, journal, expected) in [
        ("Title", false, "pages/Title.md"),
        ("Also Owner", false, "pages/Owner.md"),
        ("Parent/Child", false, "pages/Parent%2FChild.md"),
        ("Sep 25th, 2026", true, "journals/2026_09_25.md"),
    ] {
        let id = match view.resolve(name, journal) {
            Resolved::Existing { id, .. } => id,
            Resolved::Alias { owners } => owners.into_iter().next().unwrap(),
            Resolved::Absent { .. } => panic!("{name} did not resolve"),
        };
        assert_eq!(id.as_str(), expected);
        assert_eq!(store.page(&id).unwrap().id, id);
    }
}

#[test]
fn configured_nested_page_directory_keeps_page_identity() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.0.join("logseq")).unwrap();
    std::fs::create_dir_all(f.0.join("archive/pages")).unwrap();
    std::fs::create_dir_all(f.0.join("diary")).unwrap();
    std::fs::write(
        f.0.join("logseq/config.edn"),
        "{:pages-directory \"archive/pages\" :journals-directory \"diary\"}\n",
    )
    .unwrap();
    std::fs::write(f.0.join("archive/pages/Nested.md"), "- nested\n").unwrap();
    let store = f.store();
    let id = store.file_id(Area::Pages, "Nested.md").unwrap();
    let page = store.page(&store.as_page(&id).unwrap()).unwrap();
    assert_eq!(page.id.as_str(), "archive/pages/Nested.md");
    assert_eq!(page.doc.blocks[0].raw, "nested");
}

#[cfg(unix)]
#[test]
fn approved_external_assets_reject_retarget() {
    let f = Fixture::new();
    let approved = f.0.with_extension("approved-assets");
    let other = f.0.with_extension("other-assets");
    std::fs::create_dir_all(&approved).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(approved.join("asset.bin"), b"approved").unwrap();
    std::fs::write(other.join("asset.bin"), b"unapproved").unwrap();
    std::fs::remove_file(f.0.join("assets/pic.bin")).unwrap();
    std::fs::remove_dir(f.0.join("assets")).unwrap();
    std::os::unix::fs::symlink(&approved, f.0.join("assets")).unwrap();
    let store = Store::open(
        &f.0,
        OpenOptions {
            approved_external_assets: Some(approved.clone()),
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let id = store.file_id(Area::Assets, "asset.bin").unwrap();
    assert_eq!(store.read(&id, None).unwrap().0, b"approved");
    std::fs::remove_file(f.0.join("assets")).unwrap();
    std::os::unix::fs::symlink(&other, f.0.join("assets")).unwrap();
    assert!(matches!(
        store.read(&id, None),
        Err(StoreError::InvalidTarget(_))
    ));
    std::fs::remove_dir_all(approved).unwrap();
    std::fs::remove_dir_all(other).unwrap();
}
