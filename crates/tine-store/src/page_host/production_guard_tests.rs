use super::*;

#[test]
fn l8_02_raw_creation_rules_survive_failure_and_new_input_revokes_them() {
    let mut f = Fixture::new();
    fs::write(
        f.graph.join("Other.md"),
        b"title:: Hidden\nbroken:: \xff\n\n- imported\n",
    )
    .unwrap();
    let store = Arc::new(crate::Store::open(&f.graph, Default::default()).unwrap().0);
    store.whole_graph_reconciled().unwrap();
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
