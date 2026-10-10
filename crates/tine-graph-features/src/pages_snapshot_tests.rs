use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs as disk, sync::Arc};

#[test]
fn rename_plan_keeps_its_view_during_concurrent_referrer_write() {
    let root = std::env::temp_dir().join(format!(
        "tine-d3-rename-plan-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    disk::write(root.join("pages/Old.md"), "- source\n").unwrap();
    disk::write(root.join("pages/Referrer.md"), "- [[Old]] before\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let written = AtomicBool::new(false);
    rename_page_after_inventory(&store, None, "Old", "New", None, None, &[], || {
        if written.swap(true, Ordering::AcqRel) {
            return;
        }
        let writer = Arc::clone(&store);
        std::thread::spawn(move || {
            let id = PageId::from("pages/Referrer.md");
            let read = writer.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = "[[Old]] concurrent".into();
            assert!(matches!(
                writer.save(
                    tine_store::EditKind::ReplacePage,
                    &id,
                    SaveBase::Existing(read.rev),
                    &doc
                ),
                SaveOutcome::Saved(_)
            ));
        })
        .join()
        .unwrap();
    })
    .unwrap();
    assert!(root.join("pages/New.md").is_file());
    assert!(!root.join("pages/Old.md").exists());
    assert!(disk::read_to_string(root.join("pages/Referrer.md"))
        .unwrap()
        .contains("[[New]] concurrent"));
    store.close();
    disk::remove_dir_all(root).unwrap();
}

#[test]
fn delete_detects_an_external_twin_before_selecting_a_file() {
    let root = std::env::temp_dir().join(format!(
        "tine-d3-delete-external-twin-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    disk::write(root.join("pages/Old.md"), "- markdown\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let _ = store.whole_graph().unwrap();
    disk::write(root.join("pages/Old.org"), "* org\n").unwrap();

    assert!(delete_page_expected(
        &store,
        None,
        "Old",
        tine_core::model::PageKind::Page,
        None,
        None
    )
    .is_err());
    assert!(root.join("pages/Old.md").exists());
    assert!(root.join("pages/Old.org").exists());
    store.close();
    disk::remove_dir_all(root).unwrap();
}

#[test]
fn force_save_writes_only_the_named_twin_on_production_path() {
    // A loaded page carries its file path, so keep-mine targets that file.
    let root = std::env::temp_dir().join(format!(
        "tine-b22-force-save-twin-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    let md = root.join("pages/Foo.md");
    let org = root.join("pages/Foo.org");
    disk::write(&md, "- md body\n").unwrap();
    disk::write(&org, "* org body\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = PageId::from("pages/Foo.md");
    let read = store.page(&id).unwrap();
    let mut page = read.doc;
    page.blocks[0].raw = "edited".into();

    let outcome = save_page(
        &store,
        tine_store::EditKind::ReplacePage,
        &id,
        &page,
        None,
        true,
    );
    let md_after = disk::read_to_string(&md).unwrap();
    let org_after = disk::read_to_string(&org).unwrap();
    store.close();
    disk::remove_dir_all(root).unwrap();
    assert!(
        matches!(outcome, Ok(SaveOutcome::Saved(_))),
        "force-save must write the named twin: {outcome:?}"
    );
    assert_eq!(md_after, "- edited\n");
    assert_eq!(org_after, "* org body\n");
}

#[test]
fn creating_a_named_page_refuses_an_existing_twin() {
    // The old unpinned DTO has no production equivalent. A create by explicit
    // file name still refuses the competing physical claimant.
    let root = std::env::temp_dir().join(format!(
        "tine-b22-create-twin-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    let md = root.join("pages/Foo.md");
    let org = root.join("pages/Foo.org");
    disk::write(&org, "* org body\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = PageId::from("pages/Foo.md");
    let mut page = store.page(&PageId::from("pages/Foo.org")).unwrap().doc;
    page.format = tine_core::model::Format::Md;
    page.blocks[0].raw = "edited".into();
    let outcome = save_page(
        &store,
        tine_store::EditKind::ReplacePage,
        &id,
        &page,
        None,
        false,
    )
    .unwrap();
    assert!(matches!(outcome, SaveOutcome::Twin { .. }), "{outcome:?}");
    assert!(!md.exists());
    assert_eq!(disk::read_to_string(&org).unwrap(), "* org body\n");
    store.close();
    disk::remove_dir_all(root).unwrap();
}

#[test]
fn force_save_keeps_guide_ephemeral_and_refuses_unknown_bytes() {
    let root = std::env::temp_dir().join(format!(
        "tine-b22-force-basics-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages/A.md");
    disk::write(&path, "- original\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = PageId::from("pages/A.md");
    let mut page = store.page(&id).unwrap().doc;
    page.guide = true;
    assert!(matches!(
        save_page(
            &store,
            tine_store::EditKind::ReplacePage,
            &id,
            &page,
            None,
            true
        ),
        Ok(SaveOutcome::GuideEphemeral)
    ));
    assert_eq!(disk::read_to_string(&path).unwrap(), "- original\n");
    page.guide = false;
    page.blocks[0].raw = "replacement".into();
    let unknown = b"\xff\xfeunknown on-disk bytes";
    disk::write(&path, unknown).unwrap();
    assert!(matches!(
        save_page(
            &store,
            tine_store::EditKind::ReplacePage,
            &id,
            &page,
            None,
            true
        ),
        Err(StoreError::Undecodable)
    ));
    assert_eq!(disk::read(&path).unwrap(), unknown);
    store.close();
    disk::remove_dir_all(root).unwrap();
}

#[test]
fn force_save_refuses_non_round_trip_org_and_header_reclassification() {
    use tine_core::model::BlockDto;

    for (label, original) in [
        ("org", "* a\n*** c\n"),
        ("lf", "A:: XX\nB:: XX\nC:: XX\n"),
        ("crlf", "A:: XX\r\nB:: XX\r\nC:: XX\r\n"),
        ("unicode", "A:: XX\nklíč:: hodnota\nC:: XX\n"),
    ] {
        let root = std::env::temp_dir().join(format!(
            "tine-b22-force-header-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        disk::create_dir_all(root.join("pages")).unwrap();
        let rel = if label == "org" {
            "Weird.org"
        } else {
            "Property.md"
        };
        let path = root.join("pages").join(rel);
        disk::write(&path, original).unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let id = PageId::from(format!("pages/{rel}"));
        let mut page = store.page(&id).unwrap().doc;
        if label == "org" {
            assert!(page.read_only);
            assert!(matches!(
                save_page(
                    &store,
                    tine_store::EditKind::ReplacePage,
                    &id,
                    &page,
                    None,
                    true
                ),
                Ok(SaveOutcome::ReadOnly(_))
            ));
        } else {
            let normalized = original.replace("\r\n", "\n");
            let normalized = normalized.trim_end_matches('\n');
            assert_eq!(page.pre_block.as_deref(), Some(normalized));
            assert!(page.blocks.is_empty());
            let (kept, moved) = normalized.split_once('\n').unwrap();
            page.pre_block = Some(kept.into());
            page.blocks = vec![BlockDto {
                id: "corrupt-shape".into(),
                raw: moved.into(),
                ..Default::default()
            }];
            assert!(matches!(
                save_page(&store, tine_store::EditKind::ReplacePage, &id, &page, None, true),
                Ok(SaveOutcome::Io(error)) if error.kind() == std::io::ErrorKind::InvalidData
            ));
        }
        assert_eq!(disk::read_to_string(&path).unwrap(), original);
        store.close();
        disk::remove_dir_all(root).unwrap();
    }
}

#[test]
fn force_save_refuses_changed_header_properties_and_preamble_loss() {
    use tine_core::model::BlockDto;

    for (shape, original, kept, moved, childful) in [
        (
            "partial-value",
            "A:: old\nB:: old\n",
            Some("A:: old"),
            "B:: changed",
            false,
        ),
        (
            "partial-key",
            "A:: old\nB:: old\n",
            Some("A:: old"),
            "Renamed:: old",
            false,
        ),
        (
            "whole-key-value",
            "A:: old\nB:: old\n",
            None,
            "Renamed:: changed\nC:: newer",
            true,
        ),
        (
            "crlf",
            "A:: old\r\nB:: old\r\n",
            Some("A:: old"),
            "B:: changed",
            false,
        ),
        (
            "unicode-plugin",
            "A:: old\n插件/键:: old\n",
            Some("A:: old"),
            "插件/新:: changed",
            false,
        ),
    ] {
        let root = std::env::temp_dir().join(format!(
            "tine-b22-force-changed-{shape}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        disk::create_dir_all(root.join("pages")).unwrap();
        let path = root.join("pages/Property.md");
        disk::write(&path, original).unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let id = PageId::from("pages/Property.md");
        let before = store.page(&id).unwrap().doc;
        let mut page = store.page(&id).unwrap().doc;
        page.pre_block = kept.map(str::to_string);
        page.blocks = vec![BlockDto {
            id: "reclassified-header".into(),
            raw: moved.into(),
            children: childful
                .then(|| BlockDto {
                    id: "body".into(),
                    raw: "Body".into(),
                    ..Default::default()
                })
                .into_iter()
                .collect(),
            ..Default::default()
        }];
        assert!(matches!(
            save_page(&store, tine_store::EditKind::ReplacePage, &id, &page, None, true),
            Ok(SaveOutcome::Io(error)) if error.kind() == std::io::ErrorKind::InvalidData
        ));
        assert_eq!(disk::read_to_string(&path).unwrap(), original);
        let after = store.page(&id).unwrap().doc;
        assert_eq!(after.pre_block, before.pre_block);
        assert_eq!(after.blocks.len(), before.blocks.len());
        assert_eq!(after.rev, before.rev);
        store.close();
        disk::remove_dir_all(root).unwrap();
    }

    let root = std::env::temp_dir().join(format!(
        "tine-b22-force-preamble-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages/Imported.md");
    let original = "Intro before outline\n\n- Body\n";
    disk::write(&path, original).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = PageId::from("pages/Imported.md");
    let mut page = store.page(&id).unwrap().doc;
    page.pre_block = None;
    page.blocks.insert(
        0,
        BlockDto {
            id: "candidate".into(),
            raw: "alias:: book".into(),
            ..Default::default()
        },
    );
    let outcome = save_page(
        &store,
        tine_store::EditKind::ReplacePage,
        &id,
        &page,
        None,
        true,
    )
    .unwrap();
    assert!(matches!(
        outcome,
        SaveOutcome::Io(error) if error.kind() == std::io::ErrorKind::InvalidData
            && error.to_string().contains("existing page preamble")
    ));
    assert_eq!(disk::read_to_string(&path).unwrap(), original);
    let cached = store.page(&id).unwrap().doc;
    assert_eq!(cached.pre_block.as_deref(), Some("Intro before outline"));
    assert_eq!(cached.blocks.len(), 1);
    store.close();
    disk::remove_dir_all(root).unwrap();
}

/// STEP3 §7, F10, E17: with a host the command routes a single page's
/// rename, with or without a file, to the host's own operation; a namespace
/// rename or a merge to a retained transaction that flushes unsaved input
/// first; and an interrupted title completion to one that refuses it.
#[test]
fn a_hosted_rename_routes_by_the_plan_shape() {
    let root = std::env::temp_dir().join(format!(
        "tine-rename-routing-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = disk::remove_dir_all(&root);
    disk::create_dir_all(root.join("pages")).unwrap();
    disk::create_dir_all(root.join("logseq")).unwrap();
    disk::write(
        root.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    for (name, text) in [
        ("Start.md", "- [[Ghost]]\n"),
        ("Ns.md", "- ns\n"),
        ("Ns___child.md", "- child\n"),
        ("Into.md", "- into\n"),
        ("Merged.md", "- merged\n"),
        ("Done.md", "title:: Half\n\n- interrupted\n"),
    ] {
        disk::write(root.join("pages").join(name), text).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    let shape = |old: &str, new: &str, into: Option<&str>| {
        let plan = plan_rename(&store, old, new, None, into, &|| {}).unwrap();
        let single = plan.single(&store).unwrap();
        let single =
            single.map(|(source, target)| (source.as_str().to_owned(), target.as_str().to_owned()));
        (single, plan.input())
    };
    let pair = |source: &str, target: &str| Some((source.to_owned(), target.to_owned()));
    assert_eq!(
        shape("Start", "Begin", None),
        (pair("pages/Start.md", "pages/Begin.md"), Input::Flush)
    );
    assert_eq!(
        shape("Ghost", "Spirit", None),
        (pair("pages/Ghost.md", "pages/Spirit.md"), Input::Flush)
    );
    assert_eq!(shape("Ns", "Space", None), (None, Input::Flush));
    assert_eq!(
        shape("Merged", "Into", Some("pages/Into.md")),
        (None, Input::Flush)
    );
    assert_eq!(shape("Half", "Done", None), (None, Input::Refuse));
    store.close();
    let _ = disk::remove_dir_all(root);
}

/// A graph under a temp dir: `files` relative to the graph root.
fn b2_graph(files: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf, Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    for sub in ["pages", "logseq"] {
        disk::create_dir_all(root.join(sub)).unwrap();
    }
    for (path, text) in files {
        disk::write(root.join(path), text).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (dir, root, store)
}

/// Deliver a `config.edn` the way the watcher does: write it, then refresh.
fn b2_deliver(store: &Store, root: &std::path::Path, config: &str) {
    disk::write(root.join("logseq/config.edn"), config).unwrap();
    store.refresh(tine_store::Depth::Bytes).unwrap();
}

fn b2_tree(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    disk::read_dir(root.join("pages"))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            (name, disk::read_to_string(entry.path()).unwrap())
        })
        .collect()
}

const B2_LEGACY: &str = "{}\n";
const B2_LOWBAR: &str = "{:file/name-format :triple-lowbar}\n";

/// H-Q3 (A-W1, B2): every write kind commits only in the context it was
/// planned in. A config change between plan and commit, for a namespace
/// move whose destinations the old format spelled, a title rebind and a
/// merge's survivor alike, commits nothing: the attempt replans.
#[test]
fn a_rename_planned_before_a_config_change_commits_nothing() {
    for (case, old, new, into) in [
        ("move", "Ns", "Space", None),
        ("rebind", "Half", "Done", None),
        ("survivor", "Merged", "Into", Some("pages/Into.md")),
    ] {
        let (_dir, root, store) = b2_graph(&[
            ("logseq/config.edn", B2_LEGACY),
            ("pages/Ns.md", "- ns [[Merged]] [[Half]]\n"),
            ("pages/Ns%2Fchild.md", "- child\n"),
            ("pages/Into.md", "- into\n"),
            ("pages/Merged.md", "- merged\n"),
            ("pages/Done.md", "title:: Half\n\n- interrupted\n"),
        ]);
        let plan = plan_rename(&store, old, new, None, into, &|| {}).unwrap();
        b2_deliver(&store, &root, B2_LOWBAR);
        let before = b2_tree(&root);
        assert!(
            commit_rename(&store, old, plan, &[]).unwrap().is_none(),
            "{case}: a stale plan replans"
        );
        assert_eq!(b2_tree(&root), before, "{case}: nothing written");
        store.close();
    }
}

/// H-Q3 (A-W1, B2): the pause between building the targets and capturing
/// the write set. The write set comes from the stale plan, but the commit
/// still refuses it; a full rerun spells the destinations in the new format.
#[test]
fn a_config_change_between_targets_and_write_set_replans() {
    let (_dir, root, store) = b2_graph(&[
        ("logseq/config.edn", B2_LEGACY),
        ("pages/Ns.md", "- ns\n"),
        ("pages/Ns%2Fchild.md", "- child\n"),
    ]);
    let mut plan = plan_rename(&store, "Ns", "Space", None, None, &|| {}).unwrap();
    b2_deliver(&store, &root, B2_LOWBAR);
    let pages = plan.pages(&store).unwrap();
    assert!(
        pages
            .iter()
            .any(|id| id.as_str() == "pages/Space%2Fchild.md"),
        "the stale plan spells the old format: {pages:?}"
    );
    let before = b2_tree(&root);
    assert!(commit_rename(&store, "Ns", plan, &[]).unwrap().is_none());
    assert_eq!(b2_tree(&root), before);
    rename_page_expected(&store, None, "Ns", "Space", None).unwrap();
    let after = b2_tree(&root);
    assert!(after.contains_key("Space___child.md"), "{after:?}");
    assert!(!after.contains_key("Space%2Fchild.md"), "{after:?}");
    store.close();
}

/// H-Q3 (A-W1, B2), the retry bound: a config that changes during every
/// plan exhausts the existing four attempts with no write; once it is
/// stable the rename goes through, in the format it was planned in.
#[test]
fn a_config_changing_during_every_plan_exhausts_the_retries() {
    let (_dir, root, store) = b2_graph(&[
        ("logseq/config.edn", B2_LEGACY),
        ("pages/Ns.md", "- ns\n"),
        ("pages/Ns%2Fchild.md", "- child\n"),
        ("pages/Ref.md", "- see [[Ns/child]]\n"),
    ]);
    let before = b2_tree(&root);
    let flips = std::cell::Cell::new(0);
    let toggle = || {
        flips.set(flips.get() + 1);
        let config = if flips.get() % 2 == 1 {
            B2_LOWBAR
        } else {
            B2_LEGACY
        };
        b2_deliver(&store, &root, config);
    };
    let err = rename_page_after_inventory(&store, None, "Ns", "Space", None, None, &[], toggle)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(err.to_string(), "page changed repeatedly during rename");
    assert_eq!(flips.get(), 4, "the existing bound: four plans");
    assert_eq!(b2_tree(&root), before, "no write");
    rename_page_after_inventory(&store, None, "Ns", "Space", None, None, &[], || {}).unwrap();
    let after = b2_tree(&root);
    assert!(after.contains_key("Space%2Fchild.md"), "{after:?}");
    assert_eq!(after["Ref.md"], "- see [[Space/child]]\n");
    store.close();
}

/// H-Q3 (A-W1, B2), the host path: a config change during the first plan
/// fails the host's admission check before anything is written; the
/// replan's single-page rename uses the new format.
#[test]
fn a_hosted_rename_planned_before_a_config_change_replans() {
    let (dir, root, store) = b2_graph(&[
        ("logseq/config.edn", B2_LEGACY),
        ("pages/Start.md", "- start\n"),
        ("pages/Ref.md", "- see [[Start]]\n"),
    ]);
    let app = dir.path().join("app");
    disk::create_dir_all(&app).unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, &app).unwrap();
    let delivered = AtomicBool::new(false);
    rename_page_after_inventory(
        &store,
        Some(&host),
        "Start",
        "Begin/x",
        None,
        None,
        &[],
        || {
            if !delivered.swap(true, Ordering::AcqRel) {
                b2_deliver(&store, &root, B2_LOWBAR);
            }
        },
    )
    .unwrap();
    let after = b2_tree(&root);
    assert!(after.contains_key("Begin___x.md"), "{after:?}");
    assert!(!after.contains_key("Begin%2Fx.md"), "{after:?}");
    assert!(!after.contains_key("Start.md"), "{after:?}");
    assert_eq!(after["Ref.md"], "- see [[Begin/x]]\n");
    drop(host);
    store.close();
}
