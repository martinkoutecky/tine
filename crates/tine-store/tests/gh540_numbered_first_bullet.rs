//! GH #540 (port of master 80fa6e225) — an empty numbered-list item as the
//! first bullet of a new page.
//!
//! The numbered-list command leaves an empty first bullet whose raw is exactly
//! `logseq.order-list-type:: number`. The save path took that for page-header
//! properties and wrote it as the unbulleted preamble; once the user typed the
//! item's text, the next save moved that "header" property back into the
//! outline, the GH #163 firewall refused it, and the page stopped saving.

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

fn scratch(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "tine-gh540-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    root
}

#[test]
fn an_empty_numbered_first_bullet_stays_a_list_item_and_the_page_keeps_saving() {
    let root = scratch("numbered");
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = PageId::from("pages/Dosa.md");
    let first = store.save(
        tine_store::EditKind::ReplacePage,
        &id,
        SaveBase::CreateNew,
        &page(
            "Dosa",
            vec![block("b1", "logseq.order-list-type:: number", vec![])],
        ),
    );
    assert!(
        matches!(first, SaveOutcome::Saved(_)),
        "first save: {first:?}"
    );
    let read = store.page(&id).unwrap();
    assert_eq!(
        read.doc.pre_block,
        None,
        "an empty numbered item must not become page properties: {:?}",
        fs::read_to_string(root.join("pages/Dosa.md")).unwrap()
    );

    // Then the item's text and a child, exactly as typed.
    let mut next = read.doc.clone();
    next.blocks = vec![block(
        "b1",
        "Dosa\nlogseq.order-list-type:: number",
        vec![block("c1", "crispy", vec![])],
    )];
    let second = store.save(
        tine_store::EditKind::ReplacePage,
        &id,
        SaveBase::Existing(read.rev),
        &next,
    );
    assert!(
        matches!(second, SaveOutcome::Saved(_)),
        "typing into the numbered item must keep saving: {second:?}"
    );
    let reread = store.page(&id).unwrap().doc;
    assert_eq!(reread.pre_block, None);
    assert_eq!(
        reread
            .blocks
            .first()
            .map(|b| (b.raw.as_str(), b.children.len())),
        Some(("Dosa\nlogseq.order-list-type:: number", 1)),
    );
    store.close();
    fs::remove_dir_all(root).unwrap();
}
