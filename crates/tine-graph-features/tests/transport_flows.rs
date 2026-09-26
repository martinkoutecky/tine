use std::sync::atomic::{AtomicU64, Ordering};
use tine_core::model::PageKind;
use tine_graph_features::{assets, pages};
use tine_store::{OpenOptions, PageId, SaveOutcome, Store};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-feature-transport-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for area in ["pages", "journals", "assets"] {
            std::fs::create_dir_all(root.join(area)).unwrap();
        }
        std::fs::write(root.join("pages/Note.md"), "alias:: Shortcut\n- before\n").unwrap();
        Self(root)
    }
    fn store(&self) -> Store {
        Store::open(&self.0, OpenOptions::default()).unwrap().0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn page_resolution_and_force_save_preserve_the_live_revision_guard() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let alias = pages::get_page(&store, "Shortcut", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(alias.id, PageId::from("pages/Note.md"));
    assert!(pages::get_page(&store, "Missing", PageKind::Page)
        .unwrap()
        .is_none());

    let mut doc = alias.doc;
    doc.blocks[0].raw = "kept by force".into();
    std::fs::write(
        fixture.0.join("pages/Note.md"),
        "alias:: Shortcut\n- changed outside\n",
    )
    .unwrap();
    let outcome = pages::save_page(&store, &alias.id, &doc, None, true).unwrap();
    assert!(matches!(outcome, SaveOutcome::Saved(_)));
    assert!(std::fs::read_to_string(fixture.0.join("pages/Note.md"))
        .unwrap()
        .contains("kept by force"));
    std::fs::write(fixture.0.join("pages/Note.md"), [0xff]).unwrap();
    assert!(pages::save_page(&store, &alias.id, &doc, None, true).is_err());
    assert_eq!(
        std::fs::read(fixture.0.join("pages/Note.md")).unwrap(),
        [0xff]
    );
}

#[test]
fn asset_path_import_read_and_trash_summary_keep_one_file_semantics() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let source = fixture.0.with_extension("source.bin");
    std::fs::write(&source, b"media").unwrap();
    let name = assets::choose_import_name(
        source.file_name().and_then(|name| name.to_str()),
        Some("kept.bin"),
    )
    .unwrap();
    let name = assets::import_asset(
        &store,
        &name,
        tine_store::Content::Stream {
            source: std::fs::File::open(&source).unwrap(),
            max_bytes: u64::MAX,
        },
    )
    .unwrap();
    assert_eq!(name, "kept.bin");
    assert_eq!(assets::read_asset(&store, &name, None).unwrap(), b"media");
    assert_eq!(
        assets::path_for_os_handoff(&store, &name).unwrap(),
        fixture.0.join("assets/kept.bin")
    );
    assert_eq!(
        assets::choose_import_name(
            source.file_name().and_then(|name| name.to_str()),
            Some("../bad")
        )
        .unwrap_err(),
        "bad asset name"
    );
    assets::trash_asset(&store, &name).unwrap();
    let stats = assets::asset_trash_stats(&store).unwrap();
    assert_eq!((stats.count, stats.bytes), (1, 5));
    std::fs::remove_file(source).unwrap();
}
