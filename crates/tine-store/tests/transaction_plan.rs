//! DESIGN §3.1c: planning shares commit's preflight and writes nothing.
use std::fs;
use tine_store::{
    Area, Content, EditKind, HostEffect, HostExpected, PageHost, PageId, Refusal, SaveBase, Store,
    TrashIf, Why,
};

fn graph() -> (tempfile::TempDir, std::sync::Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    for area in ["pages", "journals", "assets"] {
        fs::create_dir(dir.path().join(area)).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (dir, store)
}

#[test]
fn a_save_plan_stages_the_same_bytes_as_commit_without_publishing() {
    let _integration_guard = crate::integration_lock();
    let (dir, store) = graph();
    let page = PageId::from("pages/A.md");
    fs::write(dir.path().join(page.as_str()), "- original\n").unwrap();
    store.refresh(tine_store::Depth::Bytes).unwrap();
    let read = store.page(&page).unwrap();
    let mut doc = read.doc;
    doc.blocks[0].raw = "edited".into();
    let transaction = || {
        let mut tx = store.transaction(Some(EditKind::SaveBlock));
        tx.save_page(
            &[EditKind::SaveBlock],
            &page,
            SaveBase::Existing(read.rev.clone()),
            &doc,
        );
        tx
    };
    let before = store.whole_graph().unwrap().rev();
    let op = transaction().plan().unwrap();
    assert_eq!(store.whole_graph().unwrap().rev(), before);
    assert_eq!(
        fs::read(dir.path().join(page.as_str())).unwrap(),
        b"- original\n"
    );
    assert_eq!(
        op.pages[0].2,
        HostExpected::Bytes(b"- original\n".as_slice().into())
    );
    assert!(matches!(
        transaction().commit(),
        tine_store::TxOutcome::Committed { .. }
    ));
    assert_eq!(
        op.pages[0].1,
        HostEffect::Write(fs::read(dir.path().join(page.as_str())).unwrap().into())
    );
    assert_eq!(op.kinds, [EditKind::SaveBlock]);
}

#[test]
fn a_non_page_step_refuses_without_creating_either_file() {
    let _integration_guard = crate::integration_lock();
    let (dir, store) = graph();
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.create(
        &store.file_id(Area::Pages, "A.md").unwrap(),
        Content::Bytes(b"- a\n".to_vec()),
    );
    tx.create(
        &store.file_id(Area::Assets, "a.bin").unwrap(),
        Content::Bytes(vec![1]),
    );
    assert!(matches!(
        tx.plan(),
        Err(Why::Refused(Refusal::InvalidTarget(_)))
    ));
    assert!(!dir.path().join("pages/A.md").exists());
    assert!(!dir.path().join("assets/a.bin").exists());
}

#[test]
fn a_move_plan_keeps_the_source_exception_and_publication_order() {
    let _integration_guard = crate::integration_lock();
    let (dir, store) = graph();
    fs::write(dir.path().join("journals/Jun 19th, 2026.md"), "- source\n").unwrap();
    store.refresh(tine_store::Depth::Bytes).unwrap();
    let src = store.file_id(Area::Journals, "Jun 19th, 2026.md").unwrap();
    let dst = store.file_id(Area::Journals, "2026_06_19.md").unwrap();
    let (_, rev) = store.read(&src, None).unwrap();
    let mut tx = store.transaction(Some(EditKind::RenamePage));
    tx.move_file(&src, rev, &dst, None);
    let op = tx.plan().unwrap();
    assert_eq!(
        op.moved,
        Some((PageId::from(dst.as_str()), PageId::from(src.as_str())))
    );
    assert!(dir.path().join(src.as_str()).exists());
    assert!(!dir.path().join(dst.as_str()).exists());
    let app = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app.path()).unwrap();
    host.apply(op).unwrap();
    assert!(!dir.path().join(src.as_str()).exists());
    assert_eq!(
        fs::read(dir.path().join(dst.as_str())).unwrap(),
        b"- source\n"
    );
}

#[test]
fn a_repeated_file_has_the_same_preflight_refusal() {
    let _integration_guard = crate::integration_lock();
    let (_dir, store) = graph();
    let file = store.file_id(Area::Pages, "A.md").unwrap();
    let transaction = || {
        let mut tx = store.transaction(Some(EditKind::ReplacePage));
        tx.create(&file, Content::Bytes(b"- a\n".to_vec()));
        tx.create(&file, Content::Bytes(b"- b\n".to_vec()));
        tx
    };
    assert!(
        matches!(transaction().plan(), Err(Why::Refused(Refusal::RepeatedFile(id))) if id == file)
    );
    assert!(
        matches!(transaction().commit(), tine_store::TxOutcome::NotCommitted {
        why: Why::Refused(Refusal::RepeatedFile(id)), .. } if id == file)
    );
}

#[test]
fn a_merge_declares_its_move_after_planning_and_keeps_both_kinds() {
    let _integration_guard = crate::integration_lock();
    let (dir, store) = graph();
    for name in ["A", "B"] {
        fs::write(
            dir.path().join(format!("pages/{name}.md")),
            format!("- {name}\n"),
        )
        .unwrap();
    }
    store.refresh(tine_store::Depth::Bytes).unwrap();
    let receiver = PageId::from("pages/A.md");
    let source = PageId::from("pages/B.md");
    let mut read = store.page(&receiver).unwrap();
    let other = store.page(&source).unwrap();
    read.doc.blocks.extend(other.doc.blocks);
    let mut tx = store.transaction(Some(EditKind::InsertBlocks));
    tx.save_page(
        &[EditKind::InsertBlocks, EditKind::DeletePage],
        &receiver,
        SaveBase::Existing(read.rev),
        &read.doc,
    );
    tx.trash(&source.file(), other.rev, TrashIf::Any);
    tx.create(
        &PageId::from("pages/Referrer.md").file(),
        Content::Bytes(b"- referrer\n".to_vec()),
    );
    let mut op = tx.plan().unwrap();
    assert_eq!(op.moved, None);
    op.moved = Some((receiver.clone(), source.clone()));
    assert_eq!(op.moved, Some((receiver, source)));
    assert_eq!(op.kinds, [EditKind::InsertBlocks, EditKind::DeletePage]);
    let app = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app.path()).unwrap();
    host.apply(op).unwrap();
}

#[test]
fn admission_retains_the_raw_create_and_dto_name_distinction() {
    let _integration_guard = crate::integration_lock();
    for structured in [false, true] {
        let (dir, store) = graph();
        let page = PageId::from("pages/A.md");
        let raw = b"title:: Hidden\n\n- created\n";
        let mut tx = store.transaction(Some(EditKind::ReplacePage));
        if structured {
            let doc = store.page_of(&page, raw).unwrap().doc;
            tx.save_page(&[EditKind::ReplacePage], &page, SaveBase::CreateNew, &doc);
        } else {
            tx.create(&page.file(), Content::Bytes(raw.to_vec()));
        }
        let op = tx.plan().unwrap();
        let mut owner = b"title:: Hidden\nbroken:: ".to_vec();
        owner.extend_from_slice(b"\xff\n\n- imported\n");
        fs::write(dir.path().join("pages/Other.md"), &owner).unwrap();
        store.refresh(tine_store::Depth::Bytes).unwrap();
        store.whole_graph().unwrap();
        let app = tempfile::tempdir().unwrap();
        let host = PageHost::start_for_tests(&store, app.path()).unwrap();
        let applied = host.apply(op);
        if structured {
            assert_eq!(applied, Err(tine_store::ApplyRefusal::Changed));
            assert!(!dir.path().join(page.as_str()).exists());
            assert!(
                host.quiescent_for_tests(),
                "a binding refusal records no operation"
            );
        } else {
            applied.unwrap();
            assert_eq!(fs::read(dir.path().join(page.as_str())).unwrap(), raw);
        }
        assert_eq!(fs::read(dir.path().join("pages/Other.md")).unwrap(), owner);
    }
}

#[test]
fn two_unique_creates_allocate_the_same_names_as_commit() {
    let _integration_guard = crate::integration_lock();
    let (dir, store) = graph();
    let transaction = || {
        let mut tx = store.transaction(Some(EditKind::ReplacePage));
        for _ in 0..2 {
            tx.create_unique(Area::Pages, "A", ".md", Content::Bytes(b"- a\n".to_vec()));
        }
        tx
    };
    let op = transaction().plan().unwrap();
    let names: Vec<_> = op.pages.iter().map(|(id, ..)| id.as_str()).collect();
    assert_eq!(names, ["pages/A.md", "pages/A_1.md"]);
    assert!(matches!(
        transaction().commit(),
        tine_store::TxOutcome::Committed { .. }
    ));
    for (id, effect, _) in op.pages {
        assert_eq!(
            effect,
            HostEffect::Write(fs::read(dir.path().join(id.as_str())).unwrap().into())
        );
    }
    let (host_dir, host_store) = graph();
    let mut tx = host_store.transaction(Some(EditKind::ReplacePage));
    for _ in 0..2 {
        tx.create_unique(Area::Pages, "A", ".md", Content::Bytes(b"- a\n".to_vec()));
    }
    let app = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&host_store, app.path()).unwrap();
    host.apply(tx.plan().unwrap()).unwrap();
    for name in ["pages/A.md", "pages/A_1.md"] {
        assert_eq!(fs::read(host_dir.path().join(name)).unwrap(), b"- a\n");
    }
}

#[test]
fn a_declared_transfer_keeps_create_news_dto_name_check() {
    let _integration_guard = crate::integration_lock();
    let (dir, store) = graph();
    let src = PageId::from("pages/Src.md");
    let dst = PageId::from("pages/A.md");
    fs::write(dir.path().join(src.as_str()), b"- source\n").unwrap();
    store.refresh(tine_store::Depth::Bytes).unwrap();
    let source = store.page(&src).unwrap();
    let doc = store
        .page_of(&dst, b"title:: Hidden\n\n- source\n")
        .unwrap()
        .doc;
    let transaction = || {
        let mut tx = store.transaction(Some(EditKind::ReplacePage));
        tx.save_page(
            &[EditKind::ReplacePage, EditKind::DeletePage],
            &dst,
            SaveBase::CreateNew,
            &doc,
        );
        tx.trash(&src.file(), source.rev.clone(), TrashIf::Any);
        tx
    };
    let mut op = transaction().plan().unwrap();
    op.moved = Some((dst.clone(), src.clone()));
    let owner = b"title:: Hidden\nbroken:: \xff\n\n- imported\n";
    fs::write(dir.path().join("pages/Other.md"), owner).unwrap();
    store.refresh(tine_store::Depth::Bytes).unwrap();
    store.whole_graph().unwrap();
    assert!(matches!(
        transaction().commit(),
        tine_store::TxOutcome::NotCommitted {
            why: Why::Refused(Refusal::UnreadableOwner { .. }),
            ..
        }
    ));
    let app = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app.path()).unwrap();
    assert_eq!(host.apply(op), Err(tine_store::ApplyRefusal::Changed));
    assert!(!dir.path().join(dst.as_str()).exists());
    assert_eq!(
        fs::read(dir.path().join(src.as_str())).unwrap(),
        b"- source\n"
    );
    assert!(host.quiescent_for_tests());
}

#[test]
fn independent_save_and_trash_do_not_infer_a_transfer() {
    let _integration_guard = crate::integration_lock();
    let (dir, store) = graph();
    let src = PageId::from("pages/Src.md");
    fs::write(dir.path().join(src.as_str()), b"- source\n").unwrap();
    store.refresh(tine_store::Depth::Bytes).unwrap();
    let source = store.page(&src).unwrap();
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.create(
        &PageId::from("pages/A.md").file(),
        Content::Bytes(b"- independent\n".to_vec()),
    );
    tx.trash(&src.file(), source.rev, TrashIf::Any);
    assert_eq!(tx.plan().unwrap().moved, None);
}
