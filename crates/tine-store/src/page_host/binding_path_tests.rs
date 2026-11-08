//! GH #905: a clean held page follows external removal through its owner.
use super::*;

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
