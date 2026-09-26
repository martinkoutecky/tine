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
    rename_page_after_inventory(&store, "Old", "New", None, || {
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
                writer.save(&id, SaveBase::Existing(read.rev), &doc),
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

    assert!(
        delete_page_expected(&store, "Old", tine_core::model::PageKind::Page, None, None).is_err()
    );
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

    let outcome = save_page(&store, &id, &page, None, true);
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
    let outcome = save_page(&store, &id, &page, None, false).unwrap();
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
        save_page(&store, &id, &page, None, true),
        Ok(SaveOutcome::GuideEphemeral)
    ));
    assert_eq!(disk::read_to_string(&path).unwrap(), "- original\n");
    page.guide = false;
    page.blocks[0].raw = "replacement".into();
    let unknown = b"\xff\xfeunknown on-disk bytes";
    disk::write(&path, unknown).unwrap();
    assert!(matches!(
        save_page(&store, &id, &page, None, true),
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
                save_page(&store, &id, &page, None, true),
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
                save_page(&store, &id, &page, None, true),
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
            save_page(&store, &id, &page, None, true),
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
    let outcome = save_page(&store, &id, &page, None, true).unwrap();
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
