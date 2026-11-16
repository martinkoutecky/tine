//! GH #905: an external move must retire the checkpoint's old path.
use super::*;

#[test]
fn a_late_publication_for_a_missing_path_does_not_create_a_file_claimant() {
    let root = graph();
    fs::create_dir_all(root.path().join("pages/sub")).unwrap();
    fs::rename(
        root.path().join("pages/A.md"),
        root.path().join("pages/sub/A.md"),
    )
    .unwrap();
    let store = open_cp(root.path(), &root.path().join("launch.bin"));
    store.whole_graph_reconciled().unwrap();
    {
        let _writer = store.writer.lock().unwrap();
        // Retired/late paths are named at publication time. Such a notice is
        // not evidence that the old file still exists (GH #905).
        store.changes.publish(
            Origin::External,
            vec![(
                FileId::from("pages/A.md".to_owned()),
                ChangeKind::Modified,
                None,
            )],
            false,
            vec![],
        );
    }
    match store.whole_graph().unwrap().resolve("A", false) {
        Resolved::Existing { id, others } => {
            assert_eq!(id.as_str(), "pages/sub/A.md");
            assert!(others.is_empty());
        }
        _ => panic!("the actual file must open"),
    }
    store.close();
}

#[test]
fn an_external_move_while_closed_retires_every_old_path() {
    let root = graph();
    let cp = root.path().join("launch.bin");
    let old_path = "pages/Survey test.md";
    let new_path = "pages/shared-elias/Survey test.md";
    let block = "aaaaaaaa-0000-4000-8000-000000000905";
    fs::write(
        root.path().join(old_path),
        format!("- real content\n  id:: {block}\n"),
    )
    .unwrap();
    let store = open_cp(root.path(), &cp);
    store.whole_graph_reconciled().unwrap();
    let read = store
        .page_named("Survey test", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(read.id.as_str(), old_path);
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(CheckpointWrite::Written { .. })
    ));
    store.close();
    fs::create_dir_all(root.path().join("pages/shared-elias")).unwrap();
    fs::rename(root.path().join(old_path), root.path().join(new_path)).unwrap();
    let store = open_cp(root.path(), &cp);
    let view = store.whole_graph_reconciled().unwrap();
    assert_eq!(load_outcome(&store), "loaded");
    match view.resolve("Survey test", false) {
        Resolved::Existing { id, others, .. } => {
            assert_eq!(id.as_str(), new_path);
            assert!(others.is_empty(), "stale claimant: {others:?}");
        }
        _ => panic!("moved page must resolve to its real file"),
    }
    assert_eq!(
        store
            .page_named("Survey test", PageKind::Page)
            .unwrap()
            .unwrap()
            .id
            .as_str(),
        new_path
    );
    assert_eq!(
        view.graph
            .block_owner_page(block)
            .unwrap()
            .1
             .0
            .rel_path_str(),
        new_path
    );
    let entries = view.inventory(crate::InventoryScope::All);
    assert!(!entries.0.iter().any(|entry| match &entry.target {
        Resolved::Existing { id, others } =>
            id.as_str() == old_path || others.iter().any(|id| id.as_str() == old_path),
        _ => false,
    }));
    store.close();
}

#[test]
fn a_checkpoint_path_without_a_stamp_cannot_outlive_an_external_move() {
    let root = graph();
    let cp = root.path().join("launch.bin");
    write_checkpoint(root.path(), &cp);
    // A disposable checkpoint can carry a page whose watcher baseline has
    // already forgotten the old location. Missing coverage must force recovery.
    let key = config_key(&tine_core::config::Config::default());
    let mut loaded = load(&cp, &stored_root(root.path()), &key, &|| None).unwrap();
    let missing = stored_path(root.path(), "pages/A.md");
    loaded.body.stamps.retain(|(path, _)| *path != missing);
    let body = loaded.body;
    let body = Body {
        graph: body
            .graph
            .map_pages(|pages| PagesOut(pages.into_pages(&stored_root(root.path())))),
        claimants: body.claimants,
        name_by_path: body.name_by_path,
        stamps: body.stamps,
        racy: body.racy,
    };
    fs::write(
        &cp,
        encode(&stored_root(root.path()), key, &body).unwrap().0,
    )
    .unwrap();
    fs::create_dir_all(root.path().join("pages/sub")).unwrap();
    fs::rename(&missing, root.path().join("pages/sub/A.md")).unwrap();
    let store = open_cp(root.path(), &cp);
    let view = store.whole_graph_reconciled().unwrap();
    assert_eq!(load_outcome(&store), "decode");
    match view.resolve("A", false) {
        Resolved::Existing { id, others } => {
            assert_eq!(id.as_str(), "pages/sub/A.md");
            assert!(
                others.is_empty(),
                "the absent cache row is not a physical twin"
            );
        }
        _ => panic!("real file must open"),
    }
    assert_eq!(
        store
            .page_named("A", PageKind::Page)
            .unwrap()
            .unwrap()
            .id
            .as_str(),
        "pages/sub/A.md"
    );
    store.close();
}

#[test]
fn a_stale_checkpoint_claimant_or_name_map_recovers_without_touching_the_real_file() {
    for site in ["claimant", "name-map"] {
        let root = graph();
        let cp = root.path().join("launch.bin");
        write_checkpoint(root.path(), &cp);
        let key = config_key(&tine_core::config::Config::default());
        let old = load(&cp, &stored_root(root.path()), &key, &|| None).unwrap();
        let ghost = old.body.claimants.get(&(false, "a".into())).unwrap()[0].clone();
        fs::create_dir_all(root.path().join("pages/sub")).unwrap();
        let real = root.path().join("pages/sub/A.md");
        fs::rename(root.path().join("pages/A.md"), &real).unwrap();
        let bytes = fs::read(&real).unwrap();
        write_checkpoint(root.path(), &cp);
        let mut body = load(&cp, &stored_root(root.path()), &key, &|| None)
            .unwrap()
            .body;
        // Older checkpoints could retain a claimant after its removal left
        // the baseline. It need not have a corresponding parsed page row.
        if site == "claimant" {
            Arc::make_mut(&mut body.claimants)
                .get_mut(&(false, "a".into()))
                .unwrap()
                .push(ghost);
        } else {
            Arc::make_mut(&mut body.name_by_path).insert(
                stored_path(root.path(), "pages/A.md"),
                (PageKind::Page, "A".into()),
            );
        }
        let body = Body {
            graph: body
                .graph
                .map_pages(|pages| PagesOut(pages.into_pages(&stored_root(root.path())))),
            claimants: body.claimants,
            name_by_path: body.name_by_path,
            stamps: body.stamps,
            racy: body.racy,
        };
        fs::write(
            &cp,
            encode(&stored_root(root.path()), key, &body).unwrap().0,
        )
        .unwrap();
        let store = open_cp(root.path(), &cp);
        let view = store.whole_graph_reconciled().unwrap();
        assert_eq!(load_outcome(&store), "decode", "{site}");
        assert!(
            matches!(view.resolve("A", false), Resolved::Existing { id, others } if id.as_str() == "pages/sub/A.md" && others.is_empty())
        );
        assert_eq!(fs::read(&real).unwrap(), bytes);
        assert!(!root.path().join("pages/A.md").exists());
        assert!(matches!(
            store.write_checkpoint_now(),
            Some(CheckpointWrite::Written { .. })
        ));
        store.close();
        let store = open_cp(root.path(), &cp);
        store.whole_graph_reconciled().unwrap();
        assert_eq!(
            load_outcome(&store),
            "loaded",
            "recovery writes a usable cache"
        );
        store.close();
    }
}

#[test]
fn a_sync_style_move_while_running_retires_the_old_path() {
    let root = graph();
    let store = open_cp(root.path(), &root.path().join("launch.bin"));
    store.whole_graph_reconciled().unwrap();
    fs::create_dir_all(root.path().join("pages/sub")).unwrap();
    fs::copy(
        root.path().join("pages/A.md"),
        root.path().join("pages/sub/A.md"),
    )
    .unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert!(
        matches!(store.whole_graph().unwrap().resolve("A", false), Resolved::Existing { others, .. } if others.len() == 1)
    );
    fs::remove_file(root.path().join("pages/A.md")).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert!(
        matches!(store.whole_graph().unwrap().resolve("A", false), Resolved::Existing { id, others } if id.as_str() == "pages/sub/A.md" && others.is_empty())
    );
    store.close();
}

/// The root as a checkpoint header records it: `Store::open` canonicalizes it
/// (on Windows a `\\?\` path), so `load`/`encode` must be given the same form.
fn stored_root(root: &Path) -> PathBuf {
    fs::canonicalize(root).unwrap()
}
