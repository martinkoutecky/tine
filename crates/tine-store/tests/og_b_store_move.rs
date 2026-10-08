//! I-15: one namespace change syncs each affected parent directory once.
use std::{fs, sync::Mutex};
use tine_store::{cost_counters, Area, Store, TxOutcome};

static CASE_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn transaction_moves_sync_each_distinct_parent_once() {
    let _lock = CASE_LOCK.lock().unwrap();
    for (target, expected_syncs) in [("moved.bin", 1), ("other/moved.bin", 2)] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("assets/other")).unwrap();
        fs::write(dir.path().join("assets/source.bin"), b"kept bytes").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let source = store.file_id(Area::Assets, "source.bin").unwrap();
        let destination = store.file_id(Area::Assets, target).unwrap();
        let rev = store.read(&source, None).unwrap().1;
        cost_counters::reset();
        let mut tx = store.transaction(None);
        tx.move_file(&source, rev, &destination, None);
        let result = tx.commit();
        let counts = cost_counters::snapshot();
        assert!(matches!(result, TxOutcome::Committed { .. }), "{result:?}");
        assert_eq!(store.read(&destination, None).unwrap().0, b"kept bytes");
        assert!(!dir.path().join("assets/source.bin").exists());
        assert_eq!(counts.fsyncs, expected_syncs,
            "I-15: moves must sync each distinct parent once; exemplar transaction::io_helpers::sync_move_dirs");
        store.close();
    }
}
