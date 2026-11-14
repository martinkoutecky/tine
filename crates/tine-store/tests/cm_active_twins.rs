use crate::{
    Depth, InventoryScope, OpenOptions, QueryDialect, QueryResult, Resolved, Store, WatchMode,
};

fn rows(store: &Store, query: &str) -> Vec<String> {
    let result = store
        .whole_graph()
        .unwrap()
        .query(query, QueryDialect::Simple)
        .unwrap();
    let QueryResult::Simple(groups) = result else {
        panic!("simple query")
    };
    groups
        .iter()
        .flat_map(|g| g.blocks.iter().map(|b| b.raw.clone()))
        .collect()
}

#[test]
fn active_twins_mask_meaning_and_promote_unchanged_siblings() {
    let _guard = super::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    let first = dir.path().join("pages/A.org");
    let second = dir.path().join("pages/Z.md");
    let first_text = "#+title: Shared\n#+alias: VisibleAlias\n* visible [[VisibleTarget]]\n";
    let second_text = "title:: Shared\nalias:: HiddenAlias\n- hidden [[HiddenTarget]]\n";
    std::fs::write(&first, first_text).unwrap();
    std::fs::write(&second, second_text).unwrap();
    let cp = dir.path().join("launch.bin");
    let open = || {
        Store::open(
            dir.path(),
            OpenOptions {
                watch: WatchMode::Poll,
                launch_checkpoint: Some(cp.clone()),
                ..Default::default()
            },
        )
        .unwrap()
        .0
    };
    let check = |store: &Store, visible: bool| {
        let view = store.whole_graph().unwrap();
        assert_eq!(rows(store, "[[VisibleTarget]]").len(), usize::from(visible));
        assert_eq!(rows(store, "[[HiddenTarget]]").len(), usize::from(!visible));
        let all = rows(store, "(not [[Unrelated]])");
        assert!(all
            .iter()
            .any(|s| s.contains(if visible { "visible" } else { "hidden" })));
        assert!(!all
            .iter()
            .any(|s| s.contains(if visible { "hidden" } else { "visible" })));
        assert_eq!(
            view.backlinks("VisibleTarget").unwrap().len(),
            usize::from(visible)
        );
        assert_eq!(
            view.backlinks("HiddenTarget").unwrap().len(),
            usize::from(!visible)
        );
        let inventory = view.inventory(InventoryScope::All);
        for (name, present) in [
            ("VisibleAlias", visible),
            ("VisibleTarget", visible),
            ("HiddenAlias", !visible),
            ("HiddenTarget", !visible),
        ] {
            assert_eq!(
                inventory.0.iter().any(|e| e.name == name),
                present,
                "{name}: {inventory:?}"
            );
        }
        let active = view.corpus(Some("Shared"), true);
        assert_eq!(active.pages.len(), 1);
        let block = &active.pages[0].document.roots[0];
        let snapshot = view.test_read_snapshot();
        let baseline = snapshot.referenced_names_for_completion(Some((
            "Shared",
            &block.uuid.text(),
            "baseline [[BaselineOnly]]",
        )));
        assert!(baseline.iter().any(|name| name == "BaselineOnly"));
        assert!(!baseline
            .iter()
            .any(|name| name == "VisibleTarget" || name == "HiddenTarget"));
        assert_eq!(
            view.explicit_referrers(&["HiddenTarget".to_owned()]),
            [crate::PageId::from("pages/Z.md")]
        );
        // The physical corpus is the mutation input, including inactive sites.
        assert_eq!(view.corpus(None, false).pages.len(), 2);
        match view.resolve("Shared", false) {
            Resolved::Existing { id, others } => {
                assert_eq!(
                    id.as_str(),
                    if visible { "pages/A.org" } else { "pages/Z.md" }
                );
                assert_eq!(others.len(), usize::from(visible));
            }
            other => panic!("{other:?}"),
        }
    };
    let hold = dir.path().join(".tine-test-pause-load");
    std::fs::write(&hold, "").unwrap();
    let store = open();
    let cold = store
        .page_named("Shared", tine_core::model::PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(cold.id.as_str(), "pages/A.org");
    std::fs::remove_file(&hold).unwrap();
    check(&store, true);
    store.write_checkpoint_now();
    store.close();
    let store = open();
    check(&store, true);
    assert_eq!(
        store.diagnostics()["checkpoint"]["load"]["outcome"],
        "loaded"
    );
    // Retitling the winner activates an unchanged sibling; both files still exist.
    std::fs::write(&first, "#+title: Different\n* neutral\n").unwrap();
    store.refresh(Depth::Stamps).unwrap();
    check(&store, false);
    std::fs::write(&first, first_text).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    check(&store, true);
    std::fs::remove_file(&first).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(rows(&store, "[[HiddenTarget]]").len(), 1);
    std::fs::write(&first, first_text).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    check(&store, true);
    assert_eq!(std::fs::read_to_string(&second).unwrap(), second_text);
    store.close();
}

#[test]
fn inactive_namespace_files_supply_no_parents_or_completion_names() {
    let _guard = super::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    let winner = dir.path().join("pages/A%2Fx.md");
    let loser = dir.path().join("pages/a%2Fx.org");
    std::fs::write(&winner, "").unwrap();
    std::fs::write(&loser, "* hidden [[Ghost/child]]\n").unwrap();
    let store = Store::open(
        dir.path(),
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let check = |present: bool| {
        let view = store.whole_graph().unwrap();
        let inventory = view.inventory(InventoryScope::All);
        assert_eq!(
            inventory
                .0
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case("A") && e.namespace_parent),
            present
        );
        let names = view.complete_page_names("", 100, None, false, None);
        for name in ["Ghost", "Ghost/child"] {
            assert_eq!(
                names.iter().any(|(entry, _)| entry.name == name),
                present,
                "{names:?}"
            );
        }
    };
    check(false);
    // Editing only the inactive file cannot introduce a semantic name.
    std::fs::write(&loser, "* changed [[Ghost/child]] [[AnotherHidden]]\n").unwrap();
    store.refresh(Depth::Stamps).unwrap();
    check(false);
    std::fs::remove_file(&winner).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    check(true);
    std::fs::write(&winner, "").unwrap();
    store.refresh(Depth::Stamps).unwrap();
    check(false);
    store.close();
}

#[test]
fn cross_kind_claims_prefer_pages_and_keep_all_physical_files() {
    let _guard = super::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    for folder in ["pages", "journals"] {
        std::fs::create_dir(dir.path().join(folder)).unwrap();
    }
    let ordinary = dir.path().join("pages/A.org");
    std::fs::write(
        &ordinary,
        "#+title: Nov 6th, 2026\n* ordinary [[PageTarget]]\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("journals/2026_11_06.md"),
        "- hidden [[JournalTarget]]\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("journals/2026-11-06.org"),
        "* other hidden [[OtherTarget]]\n",
    )
    .unwrap();
    let store = Store::open(
        dir.path(),
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let view = store.whole_graph().unwrap();
    for is_journal in [false, true] {
        let Resolved::Existing { id, others } = view.resolve("Nov 6th, 2026", is_journal) else {
            panic!("real file")
        };
        assert_eq!(id.as_str(), "pages/A.org");
        assert_eq!(others.len(), 2);
    }
    assert!(!view.link_target("Nov 6th, 2026").is_journal);
    assert_eq!(view.corpus(None, false).pages.len(), 3);
    assert_eq!(rows(&store, "[[JournalTarget]]").len(), 0);
    assert_eq!(rows(&store, "[[PageTarget]]").len(), 1);
    drop(view);
    std::fs::remove_file(&ordinary).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    // Within journals, hyphen sorts before underscore, regardless of format.
    assert_eq!(rows(&store, "[[OtherTarget]]").len(), 1);
    assert_eq!(rows(&store, "[[JournalTarget]]").len(), 0);
    // KD: an ordinary date spelling is a distinct page, including when a
    // caller initially classified its spelling as a journal date.
    std::fs::write(
        &ordinary,
        "#+title: 2026-11-06\n* ordinary [[PageTarget]]\n",
    )
    .unwrap();
    store.refresh(Depth::Stamps).unwrap();
    let view = store.whole_graph().unwrap();
    for is_journal in [false, true] {
        let Resolved::Existing { id, .. } = view.resolve("2026-11-06", is_journal) else {
            panic!("ordinary date file")
        };
        assert_eq!(id.as_str(), "pages/A.org");
    }
    assert!(!view.link_target("2026-11-06").is_journal);
    assert_eq!(rows(&store, "[[OtherTarget]]").len(), 1);
    store.close();
}

#[test]
fn r13_checkpoint_rebuild_and_unit_cost() {
    let _guard = super::integration_lock();
    for blocks in [1, 60] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        let file = dir.path().join("pages/Cost.md");
        let text = "- line [[Target]]\n".repeat(blocks);
        std::fs::write(&file, &text).unwrap();
        let cp = dir.path().join("launch.bin");
        let open = || {
            Store::open(
                dir.path(),
                OpenOptions {
                    watch: WatchMode::Poll,
                    launch_checkpoint: Some(cp.clone()),
                    ..Default::default()
                },
            )
            .unwrap()
            .0
        };
        let store = open();
        assert_eq!(rows(&store, "[[Target]]").len(), blocks);
        let before = store.write_checkpoint_now().unwrap();
        std::fs::write(&file, text.replace("line", "edit")).unwrap();
        store.refresh(Depth::Stamps).unwrap();
        assert_eq!(rows(&store, "[[Target]]").len(), blocks);
        let after = store.write_checkpoint_now().unwrap();
        eprintln!("Unit cost: {blocks} blocks; source bytes={}; checkpoint before={before:?}; after={after:?}; checkpoint files=1; added graph/transport records=0", text.len());
        store.close();
        let mut bytes = std::fs::read(&cp).unwrap();
        bytes[8..12].copy_from_slice(&27u32.to_le_bytes());
        std::fs::write(&cp, bytes).unwrap();
        let store = open();
        assert_eq!(rows(&store, "[[Target]]").len(), blocks);
        assert_eq!(
            store.diagnostics()["checkpoint"]["load"]["outcome"],
            "format"
        );
        store.write_checkpoint_now();
        store.close();
        let store = open();
        assert_eq!(rows(&store, "[[Target]]").len(), blocks);
        assert_eq!(
            store.diagnostics()["checkpoint"]["load"]["outcome"],
            "loaded"
        );
        store.close();
    }
}
