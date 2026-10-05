//! Checkpoint-5 packet Q (REG-OG-C5-Q-REFUSAL1): the page-header firewall in
//! `prepare_page_content` must ask the parser what a property line is, not scan
//! raw bytes for `::`. A fenced code example that merely contains `beta:: x` is
//! literal prose, so adding one while deleting a header property is a legitimate
//! save, not a header property "moved into outline content".
//!
//! The firewall keeps refusing the real shape (a header property reclassified as
//! an outline block); that is pinned by the existing
//! `save_refuses_page_header_properties_reclassified_as_outline` tests.

use std::fs;

use tine_core::model::BlockDto;
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

fn block(id: &str, raw: &str) -> BlockDto {
    BlockDto {
        id: id.into(),
        raw: raw.into(),
        ..Default::default()
    }
}

#[test]
fn removing_a_header_property_beside_a_fenced_example_is_not_a_reclassification() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    let path = dir.path().join("pages").join("Doc.md");
    fs::write(&path, "alpha:: value\n\n- item\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let id = PageId::from("pages/Doc.md");
    let read = store.page(&id).unwrap();
    let mut dto = read.doc.clone();
    assert_eq!(dto.pre_block.as_deref(), Some("alpha:: value"));
    // The user deletes the `alpha` header property and adds a block that shows
    // property syntax inside a code fence.
    dto.pre_block = None;
    dto.blocks
        .push(block("example", "```text\nbeta:: example\n```"));
    match store.save(
        EditKind::ReplacePage,
        &id,
        SaveBase::Existing(read.rev),
        &dto,
    ) {
        SaveOutcome::Saved(_) => {}
        other => panic!("a fenced example is not a header property: {other:?}"),
    }
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("alpha::"), "{text:?}");
    assert!(text.contains("```text\n  beta:: example") || text.contains("beta:: example"));
    store.close();
}

#[test]
fn a_real_property_block_moved_out_of_the_header_is_still_refused() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    let path = dir.path().join("pages").join("Doc.md");
    fs::write(&path, "alpha:: value\nbeta:: two\n\n- item\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let id = PageId::from("pages/Doc.md");
    let read = store.page(&id).unwrap();
    let mut dto = read.doc.clone();
    dto.pre_block = Some("alpha:: value".into());
    dto.blocks.insert(0, block("moved", "beta:: two"));
    match store.save(
        EditKind::ReplacePage,
        &id,
        SaveBase::Existing(read.rev),
        &dto,
    ) {
        SaveOutcome::Io(error) => assert!(error.to_string().contains("page-header property")),
        other => panic!("moving a header property into the outline must be refused: {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "alpha:: value\nbeta:: two\n\n- item\n"
    );
    store.close();
}
