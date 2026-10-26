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
use tine_store::{PageId, SaveBase, SaveOutcome, Store};

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
    let root = std::env::temp_dir().join(format!(
        "tine-gh210-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = PageId::from("pages/Tabled.md");
    let config = "tine.view:: table\ntine.table-widths:: prop%3Atest=171;title=341";
    let first = store.save(
        tine_store::EditKind::ReplacePage,
        &id,
        SaveBase::CreateNew,
        &page("Tabled", vec![block("b1", config, vec![])]),
    );
    assert!(
        matches!(first, SaveOutcome::Saved(_)),
        "first save: {first:?}"
    );
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
    let second = store.save(
        tine_store::EditKind::ReplacePage,
        &id,
        SaveBase::Existing(read.rev),
        &next,
    );
    assert!(
        matches!(second, SaveOutcome::Saved(_)),
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
