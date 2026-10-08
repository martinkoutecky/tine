//! I-12/I-25: every publisher uses the same bounded native answer signal.
use serde_json::{json, Value};
use std::fs;
use tine_store::{Area, EditKind, PageId, SaveBase, SavePagesOutcome, Store, TxOutcome};
const ONE: &str = "00000000-0000-4000-8000-000000000001";
const TWO: &str = "00000000-0000-4000-8000-000000000002";
fn fixture() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/A.md"), "- before\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    (dir, store)
}
fn save(store: &Store, raw: &str) -> Value {
    let id = PageId::from("pages/A.md");
    let mut read = store.page(&id).unwrap();
    read.doc.blocks[0].raw = raw.into();
    let SavePagesOutcome::Ok { change, .. } = store.save_pages(&[(
        id,
        SaveBase::Existing(read.rev),
        read.doc,
        vec![EditKind::ReplacePage],
    )]) else {
        panic!("save refused");
    };
    serde_json::to_value(change.expect("changed save publishes")).unwrap()
}
/// External edits reach the store through its file watcher as well as through
/// `scan_refresh`; under load the watcher may publish part of a bulk edit first.
/// Each publication carries final counts for the targets it changed, so read
/// publications until `target` reaches `count` and report whether any of them
/// changed the inventory.
fn settled(sub: &tine_store::Subscription, target: &str, count: u64) -> (bool, Value) {
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
    let (_dir, store) = fixture();
    let sub = store.subscribe();
    let text = save(&store, "after");
    assert_eq!(text["inventoryChanged"], false);
    counts(&text, json!({}));
    assert_eq!(text, serde_json::to_value(sub.recv().unwrap()).unwrap());
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
        let SavePagesOutcome::Ok { change, .. } = store.save_pages(&[(
            id.clone(),
            SaveBase::Existing(read.rev),
            read.doc,
            vec![EditKind::SaveBlock],
        )]) else {
            panic!("property refused");
        };
        assert_eq!(
            serde_json::to_value(change).unwrap()["inventoryChanged"],
            true
        );
    }
    store.close();
}
#[test]
fn create_delete_and_rename_publish_both_answers() {
    let (_dir, store) = fixture();
    let new = PageId::from("pages/New.md");
    let mut dto = store.page(&PageId::from("pages/A.md")).unwrap().doc;
    dto.blocks[0].raw = format!("(({ONE}))");
    let SavePagesOutcome::Ok { change, .. } = store.save_pages(&[(
        new.clone(),
        SaveBase::CreateNew,
        dto,
        vec![EditKind::CreatePage],
    )]) else {
        panic!("create refused");
    };
    let created = serde_json::to_value(change).unwrap();
    assert_eq!(created["inventoryChanged"], true);
    counts(&created, json!({ONE:1}));
    let renamed = store.file_id(Area::Pages, "Renamed.md").unwrap();
    let mut tx = store.transaction(Some(EditKind::RenamePage));
    tx.move_file(&new.file(), store.page(&new).unwrap().rev, &renamed, None);
    let TxOutcome::Committed { change, .. } = tx.commit() else {
        panic!("rename refused");
    };
    let moved = serde_json::to_value(change).unwrap();
    assert_eq!(moved["inventoryChanged"], true);
    counts(&moved, json!({}));
    let mut tx = store.transaction(Some(EditKind::DeletePage));
    tx.trash(
        &renamed,
        store.page(&PageId::from(renamed.as_str())).unwrap().rev,
    );
    let TxOutcome::Committed { change, .. } = tx.commit() else {
        panic!("delete refused");
    };
    let deleted = serde_json::to_value(change).unwrap();
    assert_eq!(deleted["inventoryChanged"], true);
    counts(&deleted, json!({ONE:0}));
    store.close();
}
#[test]
fn external_single_and_bulk_changes_publish_final_target_counts() {
    let (dir, store) = fixture();
    let sub = store.subscribe();
    fs::write(
        dir.path().join("pages/A.md"),
        format!("alias:: Outside\n- (({ONE}))\n"),
    )
    .unwrap();
    store.scan_refresh().unwrap();
    let (inventory, single) = settled(&sub, ONE, 1);
    assert!(inventory);
    counts(&single, json!({ONE:1}));
    for i in 0..40 {
        fs::write(
            dir.path().join(format!("pages/B{i}.md")),
            format!("- (({TWO}))\n"),
        )
        .unwrap();
    }
    store.scan_refresh().unwrap();
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
    store.scan_refresh().unwrap();
    let (_, removed) = settled(&sub, TWO, 0);
    assert_eq!(removed["inventoryChanged"], false);
    counts(&removed, json!({TWO:0}));
    store.close();
}
