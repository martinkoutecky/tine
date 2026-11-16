use tine_store::{Area, Content, FileRev, Refusal, Store, TrashIf, TxOutcome, Why};

#[test]
fn public_transactions_refuse_dynamic_page_effects_before_any_write() {
    let _guard = crate::integration_lock();
    for effect in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("pages/A.md"), b"- original\n").unwrap();
        std::fs::write(dir.path().join("assets/source.bin"), b"- source\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let page = store.file_id(Area::Pages, "A.md").unwrap();
        let new = store.file_id(Area::Pages, "B.md").unwrap();
        let asset = store.file_id(Area::Assets, "first.bin").unwrap();
        let source = store.file_id(Area::Assets, "source.bin").unwrap();
        let rev = FileRev::from_bytes(b"- original\n");
        let mut tx = store.transaction(None);
        // This earlier non-page step must not escape a later page refusal.
        tx.create(&asset, Content::Bytes(b"first".to_vec()));
        match effect {
            0 => {
                tx.create(&new, Content::Bytes(b"- new\n".to_vec()));
            }
            1 => {
                tx.create_unique(Area::Pages, "B", ".md", Content::Bytes(b"- new\n".to_vec()));
            }
            2 => {
                tx.trash(&page, rev, TrashIf::Any);
            }
            3 => {
                tx.move_file(&page, rev, &new, None);
            }
            _ => {
                tx.move_file(&source, FileRev::from_bytes(b"- source\n"), &new, None);
            }
        }
        assert!(
            matches!(
                tx.commit(),
                TxOutcome::NotCommitted {
                    why: Why::Refused(Refusal::InvalidTarget(_)),
                    ..
                }
            ),
            "effect {effect}"
        );
        assert!(!dir.path().join("assets/first.bin").exists());
        assert!(!dir.path().join("pages/B.md").exists());
        assert_eq!(
            std::fs::read(dir.path().join("pages/A.md")).unwrap(),
            b"- original\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("assets/source.bin")).unwrap(),
            b"- source\n"
        );
        store.close();
    }
}

#[test]
fn non_page_commit_can_depend_on_a_read_only_page_expectation() {
    let _guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("pages/A.md"), b"- original\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let page = store.file_id(Area::Pages, "A.md").unwrap();
    let asset = store.file_id(Area::Assets, "derived.bin").unwrap();
    let mut tx = store.transaction(None);
    tx.expect(&page, FileRev::from_bytes(b"- original\n"));
    tx.create(&asset, Content::Bytes(b"derived".to_vec()));
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    assert_eq!(
        std::fs::read(dir.path().join("assets/derived.bin")).unwrap(),
        b"derived"
    );
    store.close();
}
