use crate::test_host_writer::HostWriter;
use std::sync::Arc;
use tine_core::model::{BlockDto, PageDto, PageKind};
use tine_graph_features::{assets, pages};
use tine_store::{OpenOptions, PageId, Resolved, SaveBase, Store};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = crate::test_support::fresh_dir("tine-feature-transport-");
        for area in ["pages", "journals", "assets"] {
            std::fs::create_dir_all(root.join(area)).unwrap();
        }
        std::fs::write(root.join("pages/Note.md"), "alias:: Shortcut\n- before\n").unwrap();
        Self(root)
    }
    fn store(&self) -> Arc<Store> {
        Arc::new(Store::open(&self.0, OpenOptions::default()).unwrap().0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn journal_template_save_matches_master_bytes() {
    let _integration_guard = crate::integration_lock();
    // The same DTO/action as master's journal_template_bytes_survive_reopen_and_idempotent_resave.
    let fixture = Fixture::new();
    let store = fixture.store();
    let name = "Jul 30th, 2026";
    let id = match store.whole_graph().unwrap().resolve(name, true) {
        Resolved::Absent { id } => id,
        _ => panic!("expected absent journal"),
    };
    let page = PageDto {
        name: name.into(),
        kind: PageKind::Journal,
        title: name.into(),
        pre_block: None,
        blocks: ["### Meetings", "### Notes", "### Tasks"]
            .into_iter()
            .map(|raw| BlockDto {
                raw: raw.into(),
                ..Default::default()
            })
            .collect(),
        rev: None,
        format: Default::default(),
        read_only: false,
        guide: false,
    };
    let writer = HostWriter::start(&store);
    writer
        .save(
            tine_store::EditKind::CreatePage,
            &id,
            SaveBase::CreateNew,
            &page,
        )
        .unwrap();
    assert_eq!(
        std::fs::read(fixture.0.join("journals/2026_07_30.md")).unwrap(),
        b"- ### Meetings\n- ### Notes\n- ### Tasks\n",
    );
    drop(writer);
    drop(store);
    let reopened = fixture.store();
    let loaded = pages::get_page(&reopened, name, PageKind::Journal)
        .unwrap()
        .expect("templated journal survives reopening");
    assert_eq!(
        loaded
            .doc
            .blocks
            .iter()
            .map(|block| block.raw.as_str())
            .collect::<Vec<_>>(),
        ["### Meetings", "### Notes", "### Tasks"],
    );
    let rev = HostWriter::start(&reopened)
        .save(
            tine_store::EditKind::ReplacePage,
            &loaded.id,
            SaveBase::Existing(loaded.rev.clone()),
            &loaded.doc,
        )
        .unwrap();
    assert_eq!(rev, loaded.rev, "the same bytes keep their revision");
    assert_eq!(
        std::fs::read(fixture.0.join("journals/2026_07_30.md")).unwrap(),
        b"- ### Meetings\n- ### Notes\n- ### Tasks\n",
    );
}

#[test]
fn page_resolution_and_force_save_preserve_the_live_revision_guard() {
    let _integration_guard = crate::integration_lock();
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
    // Keep mine, through the host as the window sends it: the save guards on
    // the disk bytes the user reviewed, not the editor's base.
    let reviewed = std::fs::read(fixture.0.join("pages/Note.md")).unwrap();
    let reviewed = String::from(tine_store::FileRev::from_bytes(&reviewed));
    crate::l9_live_safety::save_guarded(&store, &alias.id, &doc, &reviewed).unwrap();
    assert!(std::fs::read_to_string(fixture.0.join("pages/Note.md"))
        .unwrap()
        .contains("kept by force"));
    std::fs::write(fixture.0.join("pages/Note.md"), [0xff]).unwrap();
    // Keep mine over bytes that are not text refuses: nothing to keep them by.
    let undecodable = String::from(tine_store::FileRev::from_bytes(&[0xff]));
    assert!(matches!(
        crate::l9_live_safety::save_guarded(&store, &alias.id, &doc, &undecodable),
        Err(tine_store::PageRefusal::Undecodable)
    ));
    assert_eq!(
        std::fs::read(fixture.0.join("pages/Note.md")).unwrap(),
        [0xff]
    );
}

#[test]
fn asset_path_import_read_and_trash_summary_keep_one_file_semantics() {
    let _integration_guard = crate::integration_lock();
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
    const TEST_ASSET_MAX_BYTES: u64 = 5;
    assert_eq!(
        assets::read_asset(&store, &name, TEST_ASSET_MAX_BYTES).unwrap(),
        b"media"
    );
    // Under the canonical root `Store::open` binds (Windows: `\\?\` path).
    assert_eq!(
        assets::path_for_os_handoff(&store, &name).unwrap(),
        std::fs::canonicalize(&fixture.0)
            .unwrap()
            .join("assets")
            .join("kept.bin")
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
