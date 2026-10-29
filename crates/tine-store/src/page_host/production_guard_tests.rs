use super::*;

#[cfg(windows)]
#[test]
fn windows_junction_at_mkdir_recheck_keeps_cold_draft_then_retries() {
    let mut executed = 0;
    for swap_root in [true, false] {
        let mut f = Fixture::new();
        fs::create_dir(f.graph.join("pages")).unwrap();
        f.host.fs.spell("c.md", "pages/sub/c.md");
        let input = "- irreplaceable Windows nested page input\n";
        f.edit("c.md", input);
        let outside = f.root.path().join("outside");
        let detached = f.root.path().join("detached");
        fs::create_dir(&outside).unwrap();
        // The substituted target holds the parent the planned mkdir needs, so
        // without the recheck the nested folder would land outside the graph.
        let planted = if swap_root {
            fs::create_dir(outside.join("pages")).unwrap();
            1
        } else {
            0
        };
        let swap = if swap_root {
            f.graph.clone()
        } else {
            f.graph.join("pages")
        };
        let destination = outside.clone();
        let moved = detached.clone();
        let substituted = swap.clone();
        crate::directory_durability::BEFORE_CREATE_CHECK.with(|step| {
            *step.borrow_mut() = Some(Box::new(move || {
                fs::rename(&substituted, moved).unwrap();
                crate::page_host::conformance::junction(&destination, &substituted);
            }));
        });
        let events = f.host.events.len();
        assert_ne!(f.save("c.md"), Outcome::Published);
        assert!(
            crate::directory_durability::BEFORE_CREATE_CHECK.with(|step| step.borrow().is_none())
        );
        assert!(!f.host.events[events..]
            .iter()
            .any(|event| matches!(event, Event::Published { .. })));
        assert_eq!(f.host.pages["c.md"].buf, text(input));
        assert_eq!(f.host.begin_draft("c.md"), Disposition::Pending);
        f.drain();
        let mut f = f.cold_reopen();
        assert_eq!(f.host.pages["c.md"].buf, text(input));
        assert!(f.host.pages["c.md"].risk);
        assert_eq!(
            drafts::scan(draft_dir(&f).into_iter().collect()).logical["c.md"].bytes,
            text(input),
            "recover from physical files after dropping host custody (root={swap_root})"
        );
        assert_eq!(fs::read_dir(&outside).unwrap().count(), planted);
        assert!(!outside.join("pages/sub").exists() && !outside.join("sub").exists());
        // Remove only the junction, restore the original directory, then bind
        // a fresh adapter to that restored directory before retrying the input.
        fs::remove_dir(&swap).unwrap();
        fs::rename(&detached, &swap).unwrap();
        f = f.cold_reopen();
        f.host.fs.spell("c.md", "pages/sub/c.md");
        assert_eq!(f.save("c.md"), Outcome::Published);
        assert_eq!(
            fs::read(f.graph.join("pages/sub/c.md")).unwrap(),
            input.as_bytes()
        );
        assert!(f.host.pages["c.md"].clean());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), planted);
        assert!(!outside.join("pages/sub").exists() && !outside.join("sub").exists());
        executed += 1;
    }
    assert_eq!(
        executed, 2,
        "Windows root and parent junction witnesses must run"
    );
}

#[cfg(unix)]
#[test]
fn dircreate_swap_during_save_keeps_input_in_durable_device_draft() {
    for swap_root in [true, false] {
        if swap_root && cfg!(target_os = "android") {
            continue; // Existing vanish-only policy, including new folders.
        }
        let mut f = Fixture::new();
        fs::create_dir(f.graph.join("pages")).unwrap();
        f.host.fs.spell("c.md", "pages/sub/c.md");
        let input = "- irreplaceable nested page input\n";
        f.edit("c.md", input);
        let outside = f.root.path().join("outside");
        let detached = f.root.path().join("detached");
        fs::create_dir(&outside).unwrap();
        let swap = if swap_root {
            f.graph.clone()
        } else {
            f.graph.join("pages")
        };
        let destination = outside.clone();
        crate::directory_durability::BEFORE_CREATE_CHECK.with(|step| {
            *step.borrow_mut() = Some(Box::new(move || {
                fs::rename(&swap, detached).unwrap();
                if swap_root {
                    fs::create_dir(&swap).unwrap();
                } else {
                    std::os::unix::fs::symlink(destination, swap).unwrap();
                }
            }));
        });
        assert_ne!(f.save("c.md"), Outcome::Published);
        assert!(
            crate::directory_durability::BEFORE_CREATE_CHECK.with(|step| step.borrow().is_none())
        );
        assert_eq!(f.host.pages["c.md"].buf, text(input));
        assert_eq!(f.host.begin_draft("c.md"), Disposition::Pending);
        f.drain();
        assert_eq!(
            drafts::scan(f.host.fs.draft_files(true)).logical["c.md"].bytes,
            text(input)
        );
        // F-DC-2: discard the entire host and its durable census. Launch a
        // fresh adapter/host against the still-swapped graph and same app data.
        let f = f.cold_reopen();
        assert_eq!(f.host.pages["c.md"].buf, text(input));
        assert!(f.host.pages["c.md"].risk);
        assert_eq!(
            drafts::scan(draft_dir(&f).into_iter().collect()).logical["c.md"].bytes,
            text(input),
            "cold recovery must retain a physical draft survivor (swap_root={swap_root})"
        );
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        if swap_root {
            assert_eq!(fs::read_dir(&f.graph).unwrap().count(), 0);
        }
    }
}

#[cfg(not(target_os = "android"))]
#[test]
fn l9_replaced_root_refuses_creating_save_and_keeps_input() {
    let mut f = Fixture::new();
    f.edit("c.md", "- irreplaceable input\n");
    fs::rename(&f.graph, f.root.path().join("moved")).unwrap();
    fs::create_dir(&f.graph).unwrap();
    assert_ne!(f.save("c.md"), Outcome::Published);
    assert!(!f.graph.join("c.md").exists());
    assert_eq!(f.host.pages["c.md"].buf, text("- irreplaceable input\n"));
    assert_eq!(f.host.begin_draft("c.md"), Disposition::Pending);
    f.drain();
    let recovered = drafts::scan(f.host.fs.draft_files(true));
    assert_eq!(
        recovered.logical["c.md"].bytes,
        text("- irreplaceable input\n")
    );
}

#[test]
fn l9_vanished_root_fails_with_named_reason_and_durable_draft() {
    let mut f = Fixture::new();
    f.edit("a.md", "- my existing page edit\n");
    fs::rename(&f.graph, f.root.path().join("moved")).unwrap();
    let error = f
        .host
        .fs
        .page_temp("a.md", &text("- my existing page edit\n"))
        .unwrap_err();
    assert_eq!(error.operation, Some("GraphRootMoved"));
    assert_ne!(f.save("a.md"), Outcome::Published);
    assert!(!f.graph.exists());
    assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(
        drafts::scan(f.host.fs.draft_files(true)).logical["a.md"].bytes,
        text("- my existing page edit\n")
    );
}

#[test]
fn l9_root_binding_cost_is_metadata_only_and_independent_of_page_size() {
    let mut measured = Vec::new();
    for blocks in [1, 60] {
        let mut f = Fixture::new();
        let body = "- typed\n".repeat(blocks);
        f.edit("a.md", &body);
        crate::root_binding::CHECKS.with(|count| count.set(0));
        crate::watch::STAMPS_BY_PATH.with(|count| count.set(0));
        crate::atomic_file::TEMP_WRITES.with(|counts| counts.set((0, 0)));
        assert_eq!(f.save("a.md"), Outcome::Published);
        let checks = crate::root_binding::CHECKS.with(|count| count.get());
        let stamps = crate::watch::STAMPS_BY_PATH.with(|count| count.get());
        let writes = crate::atomic_file::TEMP_WRITES.with(|counts| counts.get());
        assert_eq!(writes, (1, body.len() as u64));
        // M3-1: six root probes plus one post-sync publication metadata probe,
        // formerly six plus zero, for both 1 and 60 blocks; no payload reread.
        assert_eq!((checks, stamps), (6, 1));
        eprintln!("L9 I-25 blocks={blocks}: root identity checks={checks}, post-sync metadata probes={stamps}, total={}, payload writes={writes:?}; added files=0 bytes=0 transport=0", checks + stamps as u64);
        measured.push(checks);
    }
    assert_eq!(measured[0], measured[1]);
}

#[cfg(not(target_os = "android"))]
#[test]
fn l9_replaced_root_refuses_every_host_graph_io_boundary() {
    for phase in [
        Phase::Read,
        Phase::PageTemp,
        Phase::PageRename,
        Phase::TrashMove,
    ] {
        let mut f = Fixture::new();
        if phase == Phase::PageRename {
            f.host.fs.page_temp("c.md", &text("input")).unwrap();
        }
        fs::rename(&f.graph, f.root.path().join("moved")).unwrap();
        fs::create_dir(&f.graph).unwrap();
        fs::write(f.graph.join("a.md"), b"replacement occupant").unwrap();
        let failed = match phase {
            Phase::Read => f.host.fs.read_page("a.md").is_err(),
            Phase::PageTemp => f.host.fs.page_temp("c.md", &text("input")).is_err(),
            Phase::PageRename => f.host.fs.page_rename("c.md").is_err(),
            Phase::TrashMove => f.host.fs.trash_move("a.md", "payload").result.is_err(),
            _ => unreachable!(),
        };
        assert!(failed, "{phase:?} used a replacement root");
        assert_eq!(
            fs::read(f.graph.join("a.md")).unwrap(),
            b"replacement occupant"
        );
        f.host.fs.page_finish("c.md");
    }
}

#[test]
fn l8_02_raw_creation_rules_survive_failure_and_new_input_revokes_them() {
    let mut f = Fixture::new();
    // An equivalent noncanonical spelling exposes the fixture mismatch on
    // Unix too; on Windows canonicalization also introduces a verbatim prefix.
    f.graph = f.graph.join("..").join("graph");
    fs::write(
        f.graph.join("Other.md"),
        b"title:: Hidden\nbroken:: \xff\n\n- imported\n",
    )
    .unwrap();
    let store = Arc::new(crate::Store::open(&f.graph, Default::default()).unwrap().0);
    store.whole_graph_reconciled().unwrap();
    // Match PageHost::launch: its physical adapter uses the Store's root,
    // so lexical page eligibility and the inventory share one spelling.
    f.host.fs = ProductionIo::attach(&store.graph.root, &f.app, "test-graph", &f.trash);
    f.host.fs.store = Arc::downgrade(&store);
    let raw = text("title:: Hidden\n\n- raw\n");
    f.host.fs.note_creation("c.md", raw.clone(), None, None);
    f.host
        .fs
        .faults
        .insert(Phase::PageTemp, [io::ErrorKind::Other].into());
    assert!(f.host.fs.page_temp("c.md", &raw).is_err());
    assert_eq!(f.host.fs.page_owner("c.md", &raw, None).unwrap(), None);
    let editor = text("title:: Hidden\n\n- editor\n");
    f.host.fs.page_temp("c.md", &editor).unwrap();
    assert_eq!(
        f.host
            .fs
            .page_owner("c.md", &editor, None)
            .unwrap()
            .as_deref(),
        Some("Other.md")
    );
    f.host.fs.page_finish("c.md");
    store.close();
}

#[test]
fn l8_01_metadata_cost_is_independent_of_page_size() {
    use crate::model::write_target::TARGET_PROBES;
    let mut measured = Vec::new();
    for blocks in [1, 60] {
        let mut f = Fixture::new();
        let body = "- typed\n".repeat(blocks);
        f.edit("a.md", &body);
        TARGET_PROBES.with(|counts| counts.set((0, 0, 0)));
        crate::atomic_file::TEMP_WRITES.with(|counts| counts.set((0, 0)));
        assert_eq!(f.save("a.md"), Outcome::Published);
        let probes = TARGET_PROBES.with(|counts| counts.get());
        let writes = crate::atomic_file::TEMP_WRITES.with(|counts| counts.get());
        assert_eq!(
            writes,
            (1, body.len() as u64),
            "no extra persisted bytes or files"
        );
        eprintln!("L8 I-25 blocks={blocks}: containment metadata API calls (root, lstat, resolution)={probes:?}; persisted writes={writes:?}; added bytes=0 files=0 transport=0");
        measured.push(probes);
    }
    assert_eq!(measured[0], measured[1]);
}

#[cfg(unix)]
#[test]
fn l8_01_actual_io_rechecks_after_admission_and_after_temp() {
    for phase in [
        Phase::Read,
        Phase::PageTemp,
        Phase::PageRename,
        Phase::TrashMove,
    ] {
        let mut f = Fixture::new();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(f.graph.join("pages/sub")).unwrap();
        fs::write(f.graph.join("pages/sub/a.md"), b"old").unwrap();
        f.host.fs.spell("a.md", "pages/sub/a.md");
        if phase == Phase::PageRename {
            f.host.fs.page_temp("a.md", &text("typed")).unwrap();
        }
        fs::rename(f.graph.join("pages/sub"), outside.path().join("sub")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("sub"), f.graph.join("pages/sub")).unwrap();
        let refused = match phase {
            Phase::Read => f.host.fs.read_page("a.md").is_err(),
            Phase::PageTemp => f.host.fs.page_temp("a.md", &text("typed")).is_err(),
            Phase::PageRename => f.host.fs.page_rename("a.md").is_err(),
            Phase::TrashMove => f.host.fs.trash_move("a.md", "payload").result.is_err(),
            _ => unreachable!(),
        };
        assert!(refused, "{phase:?} followed the retargeted folder");
        f.host.fs.page_finish("a.md");
        assert_eq!(fs::read(outside.path().join("sub/a.md")).unwrap(), b"old");
        assert_eq!(
            fs::read_dir(outside.path().join("sub")).unwrap().count(),
            1,
            "no outside temp"
        );
        assert!(!f.trash.join("payload").exists());
    }
}

#[cfg(unix)]
#[test]
fn l8_01_trash_destination_is_checked_too() {
    let mut f = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    fs::rename(&f.trash, outside.path().join("pages")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("pages"), &f.trash).unwrap();
    assert!(f.host.fs.trash_move("a.md", "payload").result.is_err());
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"A");
    assert_eq!(
        fs::read_dir(outside.path().join("pages")).unwrap().count(),
        0
    );
}
