//! GH #905: a clean held page follows external removal through its owner.
use super::*;

#[test]
fn a_fileless_held_journal_reads_a_late_sync_delivery_without_a_watcher_event() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    let core = live.store.watch.core_for_load();
    core.deaf.store(true, std::sync::atomic::Ordering::Release);
    let (key, _) = live.open("journals/2026_11_10.md");
    assert!(matches!(
        live.store.graph.source(&live.root.join(&key)),
        crate::model::Source::HeldAbsent
    ));
    let temp = live.root.join("journals/.syncthing.2026_11_10.md.tmp");
    let bytes = b"- Journal delivered by sync\n";
    fs::write(&temp, bytes).unwrap();
    let file = fs::OpenOptions::new().write(true).open(&temp).unwrap();
    file.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        .unwrap();
    drop(file);
    fs::rename(&temp, live.root.join(&key)).unwrap();
    assert_eq!(
        live.store
            .read(&crate::FileId::from(key.clone()), None)
            .unwrap()
            .0,
        bytes
    );
    let read = live
        .store
        .page(&PageId::from(key))
        .expect("the delivered file must open without a touch or watcher publication");
    assert_eq!(read.doc.blocks[0].raw, "Journal delivered by sync");
}

#[test]
fn a_fileless_held_journal_reads_a_backdated_sync_delivery_with_a_watcher_event() {
    for watch in [crate::WatchMode::Notify, crate::WatchMode::Poll] {
        let live = Live::build(&[], false, watch);
        let (key, _) = live.open("journals/2026_11_10.md");
        let temp = live.root.join("journals/.syncthing.2026_11_10.md.tmp");
        fs::write(&temp, "- Journal delivered by sync\n").unwrap();
        let file = fs::OpenOptions::new().write(true).open(&temp).unwrap();
        file.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
            .unwrap();
        drop(file);
        fs::rename(&temp, live.root.join(&key)).unwrap();
        live.wait(&key, "the host observes the delivered journal", |mail| {
            matches!(mail.page.as_ref().map(|page| &page.text),
                Some(MailText::Page { dto }) if dto.blocks[0].raw == "Journal delivered by sync")
        });
        let read = live.store.page(&PageId::from(key)).unwrap();
        assert_eq!(read.doc.blocks[0].raw, "Journal delivered by sync");
    }
}

#[test]
fn a_live_subfolder_move_of_a_clean_held_page_retires_its_old_claimant() {
    for watch in [crate::WatchMode::Notify, crate::WatchMode::Poll] {
        let live = Live::build(
            &[("pages/shared-elias/Survey test.md", "- real content\n")],
            false,
            watch,
        );
        let (key, _) = live.open("pages/shared-elias/Survey test.md");
        assert!(
            matches!(live.store.whole_graph().unwrap().resolve("Survey test", false), crate::Resolved::Existing { id, others } if id.as_str() == key && others.is_empty())
        );
        fs::create_dir_all(live.root.join("pages/live")).unwrap();
        fs::copy(
            live.root.join(&key),
            live.root.join("pages/live/Survey test.md"),
        )
        .unwrap();
        fs::remove_file(live.root.join(&key)).unwrap();
        live.store.refresh(crate::Depth::Stamps).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if matches!(live.store.whole_graph().unwrap().resolve("Survey test", false), crate::Resolved::Existing { id, others } if id.as_str() == "pages/live/Survey test.md" && others.is_empty())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the old live claimant was not retired"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn closing_a_stale_sidebar_path_does_not_create_a_phantom_file_claimant() {
    let live = Live::build(&[], true, crate::WatchMode::Poll);
    // Session restore can open the stale path before launch has found the
    // real file. Once found, that empty host page must not claim a disk file.
    let (stale, _) = live.open("pages/Survey test.md");
    fs::create_dir_all(live.root.join("pages/shared-elias")).unwrap();
    fs::write(
        live.root.join("pages/shared-elias/Survey test.md"),
        "- real content\n",
    )
    .unwrap();
    live.store.refresh(crate::Depth::Stamps).unwrap();
    let id = live.id();
    live.host.close(live.host.session(), id, &stale).unwrap();
    assert!(live.host.quiesce_for_tests(Duration::from_secs(10)));
    match live
        .store
        .whole_graph()
        .unwrap()
        .resolve("Survey test", false)
    {
        crate::Resolved::Existing { id, others } => {
            assert_eq!(id.as_str(), "pages/shared-elias/Survey test.md");
            assert!(others.is_empty());
        }
        _ => panic!("the real file must remain the unique claimant"),
    }
}
