//! I-12/I-25: every publisher uses the same bounded native answer signal.
use crate::{Area, EditKind, PageId, SaveBase, Store};
use serde_json::{json, Value};
use std::fs;
const ONE: &str = "00000000-0000-4000-8000-000000000001";
const TWO: &str = "00000000-0000-4000-8000-000000000002";
fn fixture() -> (tempfile::TempDir, std::sync::Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/A.md"), "- before\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    (dir, std::sync::Arc::new(store))
}
/// One operation through the page host, the published change as a value.
fn apply_published(store: &std::sync::Arc<Store>, tx: crate::PagePlan<'_>) -> Value {
    let sub = store.subscribe();
    let op = tx.plan().expect("planned");
    crate::test_host_writer::HostWriter::for_store(store)
        .apply(op)
        .expect("applied");
    // Published before the reply, page by page; a new hold's retired rows
    // publish too and name no file.
    let mut own = Vec::new();
    while let Some(change) = sub.try_recv().unwrap() {
        if change.origin == crate::Origin::Own {
            own.push(serde_json::to_value(change).unwrap());
        } else {
            assert!(
                change.files.is_empty(),
                "no outside echo: {:?}",
                change.files
            );
        }
    }
    assert!(!own.is_empty(), "the operation published");
    crate::test_host_writer::merged_changes(own)
}
fn save(store: &std::sync::Arc<Store>, raw: &str) -> Value {
    let id = PageId::from("pages/A.md");
    let mut read = store.page(&id).unwrap();
    read.doc.blocks[0].raw = raw.into();
    let mut tx = store.page_plan(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::Existing(read.rev),
        &read.doc,
    );
    apply_published(store, tx)
}
/// External edits reach the store through its file watcher as well as through
/// `refresh`; under load the watcher may publish part of a bulk edit first.
/// Each publication carries final counts for the targets it changed, so read
/// publications until `target` reaches `count` and report whether any of them
/// changed the inventory.
fn settled(sub: &crate::Subscription, target: &str, count: u64) -> (bool, Value) {
    let mut inventory = false;
    for _ in 0..64 {
        let signal = serde_json::to_value(sub.recv().unwrap()).unwrap();
        inventory |= signal["inventoryChanged"] == true;
        if signal["blockRefCounts"][target] == json!(count) {
            return (inventory, signal);
        }
    }
    panic!("I-12: {target} never reached its final count {count}");
}
fn counts(signal: &Value, expected: Value) {
    assert_eq!(signal["blockRefCounts"], expected, "I-12: native publication supplies final changed-target counts; exemplar store/answer_changes.rs");
}
#[test]
fn save_signals_are_bounded_and_match_the_publication() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture();
    let text = save(&store, "after");
    assert_eq!(text["summary"]["full"], false);
    assert_eq!(text["summary"]["pages"], json!(["a"]));
    assert_eq!(text["summary"]["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(text["summary"]["rev"], text["rev"]);
    assert_eq!(text["inventoryChanged"], false);
    counts(&text, json!({}));
    let refs = save(&store, &format!("(({ONE})) (({ONE}))"));
    counts(&refs, json!({ONE:1}));
    assert_eq!(refs["inventoryChanged"], false);
    counts(&save(&store, &format!("(({TWO}))")), json!({ONE:0,TWO:1}));
    let names = save(&store, "[[Reference Only]]");
    assert_eq!(names["inventoryChanged"], true);
    counts(&names, json!({TWO:0}));
    // Name and alias properties, including title removal, use native identities.
    let id = PageId::from("pages/A.md");
    for pre in [Some("alias:: Shortcut"), Some("title:: Retitled"), None] {
        let mut read = store.page(&id).unwrap();
        read.doc.pre_block = pre.map(str::to_owned);
        let mut tx = store.page_plan(Some(EditKind::SaveBlock));
        tx.save_page(
            &[EditKind::SaveBlock],
            &id,
            SaveBase::Existing(read.rev),
            &read.doc,
        );
        assert_eq!(apply_published(&store, tx)["inventoryChanged"], true);
    }
    store.close();
}
#[test]
fn create_delete_and_rename_publish_both_answers() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture();
    let new = PageId::from("pages/New.md");
    let mut dto = store.page(&PageId::from("pages/A.md")).unwrap().doc;
    dto.blocks[0].raw = format!("(({ONE}))");
    let mut tx = store.page_plan(Some(EditKind::CreatePage));
    tx.save_page(&[EditKind::CreatePage], &new, SaveBase::CreateNew, &dto);
    let created = apply_published(&store, tx);
    // D-20261102-PERF row 3: a create, rename or delete is summarised
    // precisely (it was `full`); the summary names the moved page keys and
    // the referenced block, so its readers still refresh.
    let precise = |signal: &Value, pages: &[&str]| {
        assert_eq!(signal["summary"]["full"], false, "{signal}");
        let mut named: Vec<_> = signal["summary"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap())
            .collect();
        named.sort_unstable();
        assert_eq!(named, pages, "{signal}");
        assert!(
            signal["summary"]["refsTo"]
                .as_array()
                .unwrap()
                .contains(&json!(ONE)),
            "{signal}"
        );
    };
    precise(&created, &["new"]);
    assert_eq!(created["inventoryChanged"], true);
    counts(&created, json!({ONE:1}));
    let renamed = store.file_id(Area::Pages, "Renamed.md").unwrap();
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    tx.move_file(&new.file(), store.page(&new).unwrap().rev, &renamed, None);
    let moved = apply_published(&store, tx);
    precise(&moved, &["new", "renamed"]);
    assert_eq!(moved["inventoryChanged"], true);
    // The destination publishes first and the source's removal last, so the
    // merged value carries the final count.
    counts(&moved, json!({ONE:1}));
    let mut tx = store.page_plan(Some(EditKind::DeletePage));
    tx.trash(
        &renamed,
        store.page(&PageId::from(renamed.as_str())).unwrap().rev,
        crate::TrashIf::Any,
    );
    let deleted = apply_published(&store, tx);
    precise(&deleted, &["renamed"]);
    assert_eq!(deleted["inventoryChanged"], true);
    counts(&deleted, json!({ONE:0}));
    store.close();
}
#[test]
fn external_single_and_bulk_changes_publish_final_target_counts() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (dir, store) = fixture();
    let sub = store.subscribe();
    fs::write(
        dir.path().join("pages/A.md"),
        format!("alias:: Outside\n- (({ONE}))\n"),
    )
    .unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    let (inventory, single) = settled(&sub, ONE, 1);
    assert_eq!(single["summary"]["full"], true, "alias changes are full");
    assert!(inventory);
    counts(&single, json!({ONE:1}));
    for i in 0..40 {
        fs::write(
            dir.path().join(format!("pages/B{i}.md")),
            format!("- (({TWO}))\n"),
        )
        .unwrap();
    }
    store.refresh(crate::Depth::Stamps).unwrap();
    let (inventory, bulk) = settled(&sub, TWO, 40);
    assert!(inventory);
    counts(&bulk, json!({TWO:40}));
    for i in 0..40 {
        fs::write(
            dir.path().join(format!("pages/B{i}.md")),
            "- ordinary text\n",
        )
        .unwrap();
    }
    store.refresh(crate::Depth::Stamps).unwrap();
    let (_, removed) = settled(&sub, TWO, 0);
    assert_eq!(removed["inventoryChanged"], false);
    counts(&removed, json!({TWO:0}));
    store.close();
}
