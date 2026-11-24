//! GH #210 (0.8.0-beta.3) — a table block as the first bullet of a page with no
//! page properties.
//!
//! Choosing a table view before typing a title leaves a first bullet whose raw is
//! exactly `tine.view:: table` (plus the widths Tine writes itself). The save path
//! took that for page-header properties and wrote it as the unbulleted preamble;
//! the rows and title typed next then made the live "header" invalid, so the page
//! stopped saving and every exit from the block raised the page-header toast.
//! The same family as GH #540: the key itself says the bullet is a block.

use std::fs;
use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::{PageId, SaveBase, Store};

fn block(id: &str, raw: &str, children: Vec<BlockDto>) -> BlockDto {
    BlockDto {
        id: id.into(),
        raw: raw.into(),
        children,
        ..Default::default()
    }
}

fn page(name: &str, blocks: Vec<BlockDto>) -> PageDto {
    PageDto {
        name: name.into(),
        kind: PageKind::Page,
        title: name.into(),
        pre_block: None,
        blocks,
        rev: None,
        format: Format::Md,
        read_only: false,
        guide: false,
    }
}

#[test]
fn a_titleless_table_first_bullet_stays_a_block_and_the_page_keeps_saving() {
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-gh210-");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let store = std::sync::Arc::new(store);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
    let id = PageId::from("pages/Tabled.md");
    let config = "tine.view:: table\ntine.table-widths:: prop%3Atest=171;title=341";
    let first = host_writer.save(
        tine_store::EditKind::ReplacePage,
        &id,
        SaveBase::CreateNew,
        &page("Tabled", vec![block("b1", config, vec![])]),
    );
    assert!(matches!(first, Ok(_)), "first save: {first:?}");
    let read = store.page(&id).unwrap();
    assert_eq!(
        read.doc.pre_block,
        None,
        "a table block must not become page properties: {:?}",
        fs::read_to_string(root.join("pages/Tabled.md")).unwrap()
    );

    // Then the title and a row, exactly as typed.
    let mut next = read.doc.clone();
    let titled = format!("test table\n{config}");
    next.blocks = vec![block("b1", &titled, vec![block("c1", "blah", vec![])])];
    let second = host_writer.save(
        tine_store::EditKind::ReplacePage,
        &id,
        SaveBase::Existing(read.rev),
        &next,
    );
    assert!(
        matches!(second, Ok(_)),
        "typing into the table block must keep saving: {second:?}"
    );
    let reread = store.page(&id).unwrap().doc;
    assert_eq!(reread.pre_block, None);
    assert_eq!(
        reread
            .blocks
            .first()
            .map(|b| (b.raw.as_str(), b.children.len())),
        Some((titled.as_str(), 1)),
    );
    store.close();
    fs::remove_dir_all(root).unwrap();
}

// CHARACTERIZATION: current behaviour, not a ruling (latent-disagreements core-2).
// HARM: a page `pages/x.md` whose first bullet is `- title:: Foo` is named `x`
// until its first save; that save promotes the bullet to the unbulleted page
// header, the page is then named `Foo`, and `[[x]]` stops resolving to it.
#[test]
fn characterization_a_first_bullet_title_becomes_the_page_name_on_first_save() {
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-core2-title-bullet-");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("pages/x.md"), "- title:: Foo\n- body\n").unwrap();
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let named = |name: &str| {
        store
            .page_named(name, PageKind::Page)
            .unwrap()
            .map(|read| read.id.as_str().to_string())
    };
    assert_eq!(named("x").as_deref(), Some("pages/x.md"));
    assert_eq!(named("Foo"), None);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
    let id = PageId::from("pages/x.md");
    let read = store.page(&id).unwrap();
    let mut next = read.doc.clone();
    let body = next
        .blocks
        .iter_mut()
        .find(|block| block.raw == "body")
        .expect("body block");
    body.raw = "body edited".into();
    let saved = host_writer.save(
        tine_store::EditKind::ReplacePage,
        &id,
        SaveBase::Existing(read.rev),
        &next,
    );
    assert!(matches!(saved, Ok(_)), "save: {saved:?}");
    assert_eq!(
        fs::read_to_string(root.join("pages/x.md")).unwrap(),
        "title:: Foo\n\n- body edited\n"
    );
    store.refresh(tine_store::Depth::Bytes).unwrap();
    assert_eq!(named("x"), None);
    assert_eq!(named("Foo").as_deref(), Some("pages/x.md"));
    store.close();
    fs::remove_dir_all(root).unwrap();
}
