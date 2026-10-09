//! Focused native/filesystem target: cargo test -p tine-store --lib page_host::production_tests
//! Runs unchanged on Linux, Windows/NTFS and macOS/APFS. Native power cuts are
//! deliberately excluded; ModelFs owns those claims.
use super::drafts::{self, Stage, Vehicle};
use super::io::{HostIo, Phase, Witness};
use super::production::ProductionIo;
use super::*;
use std::fs;
use std::io;
use std::path::PathBuf;

struct Fixture {
    root: tempfile::TempDir,
    graph: PathBuf,
    app: PathBuf,
    trash: PathBuf,
    host: Host<ProductionIo>,
}

impl Fixture {
    fn new() -> Self {
        let root = if let Some(base) = std::env::var_os("TINE_HOST_FS_ROOT") {
            tempfile::tempdir_in(base).unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let graph = root.path().join("graph");
        let app = root.path().join("app");
        let trash = graph.join("logseq/.tine-trash/pages");
        fs::create_dir_all(&graph).unwrap();
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&trash).unwrap();
        fs::write(graph.join("a.md"), b"A").unwrap();
        fs::write(graph.join("b.md"), b"B").unwrap();
        let io = ProductionIo::new(&graph, &app, "test-graph", &trash).unwrap();
        let locks = ["a.md", "b.md", "c.md"]
            .into_iter()
            .map(|key| (key.into(), Arc::new(Mutex::new(()))))
            .collect();
        Self {
            root,
            graph,
            app,
            trash,
            host: Host::new(io, locks),
        }
    }

    fn send(&mut self, page: &str, kind: RequestKind) {
        let request = Request {
            id: self.host.last_admitted + 1,
            generation: self.host.generation,
            page: page.into(),
            kind,
        };
        assert_eq!(self.host.admit(request), Disposition::Applied);
        assert_eq!(self.host.dequeue(), Disposition::Pending);
        let disposition = self.host.apply_request();
        assert!(matches!(
            disposition,
            Disposition::Applied | Disposition::Pending
        ));
    }

    fn edit(&mut self, page: &str, bytes: &str) {
        self.send(page, RequestKind::Open);
        let version = self.host.pages[page].version;
        self.send(
            page,
            RequestKind::Submit {
                bytes: Some(Arc::from(bytes.as_bytes())),
                version,
                resolve: None,
            },
        );
    }

    fn save(&mut self, page: &str) -> Outcome {
        assert_eq!(self.host.start_save(page), Disposition::Pending);
        for _ in 0..15 {
            self.host.advance_save(0);
            if self.host.job.is_none() {
                return self
                    .host
                    .events
                    .iter()
                    .rev()
                    .find_map(|event| {
                        if let Event::SaveOutcome { outcome, .. } = event {
                            Some(*outcome)
                        } else {
                            None
                        }
                    })
                    .unwrap();
            }
        }
        panic!("save did not finish");
    }

    fn drain(&mut self) {
        for _ in 0..100 {
            if self.host.worker.is_none() {
                return;
            }
            self.host.advance_draft();
        }
        panic!("draft worker did not finish");
    }

    fn restart(&mut self) {
        self.host.stop();
        self.host.fs =
            ProductionIo::new(&self.graph, &self.app, "test-graph", &self.trash).unwrap();
        assert!(matches!(
            self.host.launch(),
            Disposition::Applied | Disposition::Pending
        ));
        self.drain();
    }
}

#[test]
fn replace_and_create_publish_through_the_real_host() {
    let mut f = Fixture::new();
    f.edit("a.md", "replacement");
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"replacement");
    assert!(f.host.pages["a.md"].clean());
    f.edit("c.md", "created");
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(fs::read(f.graph.join("c.md")).unwrap(), b"created");
    assert!(f.host.fs.draft_files(true).is_empty());
}

#[test]
fn launch_sync_error_is_reported_best_effort_without_refusing_graph_saves() {
    let mut f = Fixture::new();
    crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(io::ErrorKind::Other)));
    f.host.fs.graph_launch(&BTreeSet::from(["a.md".into()]));
    assert_eq!(
        f.host.fs.launch_warnings,
        vec![(f.graph.clone(), io::ErrorKind::Other)]
    );
    f.edit("a.md", "mine");
    assert_eq!(f.save("a.md"), Outcome::Published);
}

#[test]
fn failed_deletion_installs_release_only_their_unapplied_name_custody() {
    let mut f = Fixture::new();
    assert_eq!(f.host.delete("b.md"), Disposition::Pending);
    f.drain();
    let b = f.host.logical_drafts()["b.md"].trash.unwrap();
    for _ in 0..20 {
        f.host
            .fs
            .faults
            .insert(Phase::DraftTemp, [io::ErrorKind::Other].into());
        assert_eq!(f.host.delete("a.md"), Disposition::Pending);
        f.drain();
        assert!(f.host.pages["a.md"].clean());
        assert_eq!(f.host.fresh_trash, BTreeSet::from([b]));
        assert_eq!(f.host.trash.len(), 1);
        assert_eq!(f.host.logical_drafts()["b.md"].trash, Some(b));
    }
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    f.edit("a.md", "undo before the move");
    assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.fresh_trash.len(), 2);
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(f.host.fresh_trash, BTreeSet::from([b]));
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert!(f.host.fresh_trash.is_empty());
    assert!(f.host.trash.is_empty());
}

#[test]
fn real_page_read_error_is_not_absence_and_cannot_publish_over_it() {
    let mut f = Fixture::new();
    fs::create_dir(f.graph.join("c.md")).unwrap();
    f.send("c.md", RequestKind::Open);
    assert!(!f.host.pages.contains_key("c.md"));
    assert!(f.graph.join("c.md").is_dir());
    assert_eq!(f.host.start_save("c.md"), Disposition::Disabled);
}

#[test]
fn absent_vehicle_unlink_requires_sync_and_other_real_unlink_errors_keep_custody() {
    let mut f = Fixture::new();
    let name = "p-absent.draft";
    let mut vehicle = Vehicle::remove(name.into());
    vehicle.advance(&mut f.host.fs);
    assert_eq!(vehicle.stage, Stage::UnlinkSync);
    f.host
        .fs
        .faults
        .insert(Phase::DraftSync, [io::ErrorKind::Other].into());
    vehicle.advance(&mut f.host.fs);
    assert_eq!(vehicle.stage, Stage::UnlinkSync);
    vehicle.advance(&mut f.host.fs);
    assert_eq!(vehicle.stage, Stage::Absent);
    let blocked = "p-blocked.draft";
    let path = f.app.join("drafts-v2/test-graph").join(blocked);
    fs::create_dir(&path).unwrap();
    let mut vehicle = Vehicle::remove(blocked.into());
    vehicle.advance(&mut f.host.fs);
    assert_eq!(vehicle.stage, Stage::Unlink);
    assert_eq!(vehicle.failures, 1);
    assert!(path.is_dir());
}

#[test]
fn quarantine_retries_only_collision_and_preserves_source_on_other_move_errors() {
    let mut f = Fixture::new();
    let name = "p-unreadable.draft";
    let root = f.app.join("drafts-v2/test-graph");
    let source = root.join(name);
    fs::create_dir(root.join("unreadable")).unwrap();
    fs::write(&source, b"unrecoverable bytes").unwrap();
    crate::no_replace::MOVE_ERRORS.with(|errors| {
        *errors.borrow_mut() = [
            io::ErrorKind::AlreadyExists,
            io::ErrorKind::PermissionDenied,
        ]
        .into()
    });
    let error = f.host.fs.quarantine(name).unwrap_err();
    assert_eq!(error.kind, super::io::ErrorKind::Io);
    assert_eq!(fs::read(&source).unwrap(), b"unrecoverable bytes");
    assert_eq!(fs::read_dir(root.join("unreadable")).unwrap().count(), 0);
    assert!(crate::no_replace::MOVE_ERRORS.with(|errors| errors.borrow().is_empty()));
    f.host.fs.quarantine(name).unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read_dir(root.join("unreadable")).unwrap().count(), 1);
}

#[test]
fn quarantine_destination_sync_precedes_source_sync_and_is_retried_after_error() {
    let mut f = Fixture::new();
    let name = "p-unreadable.draft";
    let root = f.app.join("drafts-v2/test-graph");
    let unreadable = root.join("unreadable");
    fs::create_dir(&unreadable).unwrap();
    fs::write(root.join(name), b"preserved bytes").unwrap();
    #[cfg(feature = "test-faults")]
    crate::directory_durability::take_synced_directories();
    crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(io::ErrorKind::Other)));
    assert!(f.host.fs.quarantine(name).is_err());
    #[cfg(feature = "test-faults")]
    assert_eq!(
        crate::directory_durability::take_synced_directories(),
        vec![unreadable.clone()]
    );
    assert!(!root.join(name).exists());
    assert_eq!(fs::read_dir(&unreadable).unwrap().count(), 1);
    f.host.fs.quarantine(name).unwrap();
    #[cfg(feature = "test-faults")]
    assert_eq!(
        crate::directory_durability::take_synced_directories(),
        vec![unreadable, root]
    );
}

#[test]
fn failed_fsync_and_failed_rename_leave_original_bytes_and_report_failed() {
    for fsync in [true, false] {
        let mut f = Fixture::new();
        f.edit("a.md", "mine");
        if fsync {
            crate::atomic_file::FAIL_FILE_SYNC.with(|flag| flag.set(true));
        } else {
            f.host
                .fs
                .faults
                .insert(Phase::PageRename, [io::ErrorKind::PermissionDenied].into());
        }
        assert_eq!(f.save("a.md"), Outcome::Failed);
        assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"A");
        assert!(f.host.pages["a.md"].risk);
    }
}

#[test]
fn create_race_after_guard_is_no_replace_and_then_observes_conflict() {
    let mut f = Fixture::new();
    f.edit("c.md", "mine");
    assert_eq!(f.host.start_save("c.md"), Disposition::Pending);
    f.host.advance_save(0); // synced temp
    f.host.advance_save(0); // absent base guard
    fs::write(f.graph.join("c.md"), b"other").unwrap();
    f.host.advance_save(0); // real EEXIST
    assert_eq!(fs::read(f.graph.join("c.md")).unwrap(), b"other");
    assert!(f.host.events.contains(&Event::SaveOutcome {
        page: "c.md".into(),
        outcome: Outcome::Failed
    }));
    assert_eq!(f.host.observe("c.md"), Disposition::Applied);
    assert!(f.host.pages["c.md"].conflict);
    assert_eq!(
        f.host.pages["c.md"].buf.as_deref(),
        Some(b"mine".as_slice())
    );
}

#[test]
fn graph_unsupported_is_distinct_and_does_not_refuse_betas_tolerated_set() {
    for kind in [
        io::ErrorKind::Unsupported,
        io::ErrorKind::InvalidInput,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::NotFound,
    ] {
        let mut f = Fixture::new();
        crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(kind)));
        assert_eq!(f.host.fs.page_sync("a.md"), Ok(Witness::Unsupported));
        f.edit("a.md", "mine");
        assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
        for _ in 0..3 {
            f.host.advance_save(0);
        }
        crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(kind)));
        f.host.advance_save(0);
        assert!(f.host.pages["a.md"].clean());
        assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"mine");
    }
}

#[test]
fn post_rename_sync_error_is_tagged_uncertain_and_retry_preserves_input() {
    let mut f = Fixture::new();
    f.edit("a.md", "mine");
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    for _ in 0..3 {
        f.host.advance_save(0);
    }
    crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(io::ErrorKind::Other)));
    let error = f.host.fs.page_sync("a.md").unwrap_err();
    assert!(error.completed);
    crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(io::ErrorKind::Other)));
    f.host.advance_save(0);
    assert!(f.host.events.contains(&Event::SaveOutcome {
        page: "a.md".into(),
        outcome: Outcome::Uncertain
    }));
    assert!(f.host.pages["a.md"].risk);
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"mine");
    assert_eq!(f.host.observe("a.md"), Disposition::Applied);
    assert_eq!(f.save("a.md"), Outcome::Published);
}

#[test]
fn delete_retries_real_trash_collision_and_syncs_trash_before_source() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0);
    let name = f.host.job.as_ref().unwrap().trash_name.clone();
    let occupied = f.trash.join(format!("{name}__a.md"));
    fs::write(&occupied, b"occupied").unwrap();
    f.host.advance_save(0);
    assert_ne!(f.host.job.as_ref().unwrap().trash_name, name);
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"A");
    assert_eq!(f.host.advance_save(0), Disposition::Waiting);
    f.drain();
    assert_eq!(
        super::trash_name(f.host.logical_drafts()["a.md"].trash.unwrap()),
        f.host.job.as_ref().unwrap().trash_name
    );
    assert_eq!(f.host.logical_drafts()["a.md"].pending_trash.len(), 1);
    f.host.advance_save(0);
    assert!(!f.graph.join("a.md").exists());
    #[cfg(feature = "test-faults")]
    crate::directory_durability::take_synced_directories();
    f.host.advance_save(0);
    f.host.advance_save(0);
    #[cfg(feature = "test-faults")]
    assert_eq!(
        crate::directory_durability::take_synced_directories(),
        vec![f.trash.clone(), f.graph.clone()]
    );
    assert_eq!(fs::read(occupied).unwrap(), b"occupied");
    assert!(fs::read_dir(&f.trash)
        .unwrap()
        .any(|entry| fs::read(entry.unwrap().path()).unwrap() == b"A"));
    assert!(f.host.pages["a.md"].clean());
}

#[test]
fn trash_sync_failure_stops_delete_before_source_sync() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    f.host
        .fs
        .faults
        .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
    assert_eq!(f.save("a.md"), Outcome::Uncertain);
    assert!(f.host.pages["a.md"].risk);
    assert!(!f
        .host
        .events
        .iter()
        .any(|event| matches!(event, Event::DeleteDurable { .. })));
    assert!(fs::read_dir(&f.trash)
        .unwrap()
        .any(|entry| fs::read(entry.unwrap().path()).unwrap() == b"A"));
}

#[test]
fn weak_trash_and_strong_source_sync_do_not_claim_strong_delete_durability() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0);
    f.host.advance_save(0);
    crate::directory_durability::SYNC_ERROR
        .with(|error| error.set(Some(io::ErrorKind::InvalidInput)));
    f.host.advance_save(0);
    f.host.advance_save(0);
    assert!(f.host.pages["a.md"].clean());
    assert!(!f
        .host
        .events
        .iter()
        .any(|event| matches!(event, Event::DeleteDurable { .. })));
}

#[test]
fn trashed_external_payload_is_file_synced_before_namespace_witness() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0); // guard
    fs::write(f.graph.join("a.md"), b"unflushed-external").unwrap(); // R1
    f.host.advance_save(0); // moves actual external bytes
    crate::atomic_file::FAIL_FILE_SYNC.with(|flag| flag.set(true));
    f.host.advance_save(0);
    assert!(f.host.pages["a.md"].risk);
    assert!(f.host.events.contains(&Event::SaveOutcome {
        page: "a.md".into(),
        outcome: Outcome::Uncertain
    }));
    assert!(!f
        .host
        .events
        .iter()
        .any(|event| matches!(event, Event::DeleteDurable { .. })));
    assert!(f.host.events.contains(&Event::Removed {
        page: "a.md".into(),
        bytes: Some(Arc::from(b"unflushed-external".as_slice()))
    }));
    assert!(fs::read_dir(&f.trash)
        .unwrap()
        .any(|entry| fs::read(entry.unwrap().path()).unwrap() == b"unflushed-external"));
    // A retry whose source is already absent still needs to finish the previous
    // payload witness, otherwise it could report a clean deletion too early.
    assert_eq!(f.host.observe("a.md"), Disposition::Applied);
    crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get), 1);
}

#[test]
fn guarded_save_abandonment_cleans_only_its_own_unpublished_temp() {
    let mut f = Fixture::new();
    f.edit("a.md", "mine");
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0);
    fs::write(f.graph.join("a.md"), b"external").unwrap();
    f.host.advance_save(0);
    assert!(f.host.job.is_none());
    assert!(f.host.pages["a.md"].conflict);
    assert!(fs::read_dir(&f.graph).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"external");
}

#[test]
fn restart_must_not_forget_unflushed_trash_payload() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0);
    fs::write(f.graph.join("a.md"), b"unflushed-external").unwrap();
    f.host.advance_save(0);
    crate::atomic_file::FAIL_FILE_SYNC.with(|flag| flag.set(true));
    f.host.advance_save(0);
    f.restart();
    assert_eq!(f.host.observe("a.md"), Disposition::Applied);
    crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert!(
        crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get) >= 1,
        "Published after restart without flushing the readable trash payload"
    );
}

#[test]
fn restart_before_move_drops_missing_name_when_recreated() {
    let mut f = Fixture::new();
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    let before = f.host.logical_drafts()["a.md"].clone();
    assert_eq!(before.pending_trash, vec![before.trash.unwrap()]);
    f.restart();
    assert_eq!(f.host.observe("a.md"), Disposition::Applied);
    f.edit("a.md", "recreated");
    assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(
        f.host.logical_drafts()["a.md"].pending_trash,
        before.pending_trash
    );
    crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get), 1);
    assert!(!f.host.trash.contains_key("a.md"));
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"recreated");
    assert_eq!(fs::read_dir(&f.trash).unwrap().count(), 0);
}

#[test]
fn restart_after_payload_sync_before_trash_dirsync_retries_payload() {
    let mut f = Fixture::new();
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0);
    f.host.advance_save(0);
    crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(io::ErrorKind::Other)));
    crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
    f.host.advance_save(0);
    assert_eq!(crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get), 1);
    assert!(f.host.pages["a.md"].risk);
    f.restart();
    assert_eq!(f.host.observe("a.md"), Disposition::Applied);
    crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get), 1);
}

#[test]
fn recapture_after_source_sync_error_does_not_revive_finished_trash_debt() {
    let mut f = Fixture::new();
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    f.host
        .fs
        .faults
        .insert(Phase::PageSync, [io::ErrorKind::Other].into());
    assert_eq!(f.save("a.md"), Outcome::Uncertain);
    assert!(f.host.pages["a.md"].risk);
    // The buffer/base/version still equal the previous draft. The recovery
    // capture must nevertheless record the already completed trash witness.
    assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
    f.drain();
    f.restart();
    assert_eq!(f.host.observe("a.md"), Disposition::Applied);
    crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
    crate::atomic_file::FAIL_FILE_SYNC.with(|flag| flag.set(true));
    let outcome = f.save("a.md");
    crate::atomic_file::FAIL_FILE_SYNC.with(|flag| flag.set(false));
    assert_eq!(outcome, Outcome::Published);
    assert_eq!(
        crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get),
        0,
        "finished payload debt was revived from the recaptured record"
    );
}

#[test]
fn discard_cannot_retire_the_last_unsynced_trash_identity() {
    let mut f = Fixture::new();
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0);
    fs::write(f.graph.join("a.md"), b"unflushed R1 payload").unwrap();
    f.host.advance_save(0);
    crate::atomic_file::FAIL_FILE_SYNC.with(|flag| flag.set(true));
    f.host.advance_save(0);
    fs::write(f.graph.join("a.md"), b"externally recreated").unwrap();
    f.send(
        "a.md",
        RequestKind::Discard {
            version: f.host.pages["a.md"].version,
        },
    );
    assert!(f.host.pages["a.md"].clean());
    crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
    assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
    f.host
        .fs
        .faults
        .insert(Phase::TrashSync, [io::ErrorKind::Other; 3].into());
    for _ in 0..3 {
        f.host.advance_draft();
        assert_eq!(f.host.logical_drafts().len(), 1);
        assert_eq!(f.host.fs.draft_files(true).len(), 1);
    }
    assert!(f
        .host
        .events
        .iter()
        .any(|event| matches!(event, Event::DraftError { failures: 3, .. })));
    assert_eq!(f.host.start_save("a.md"), Disposition::Waiting);
    f.drain();
    assert_eq!(
        crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get),
        1,
        "the last durable payload identity was retired before its file witness"
    );
    assert!(f.host.logical_drafts().is_empty());
    f.edit("a.md", "edit after completed discard");
    f.host
        .fs
        .faults
        .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
    assert_eq!(
        f.save("a.md"),
        Outcome::Published,
        "completed discard must not make later edits depend on old trash"
    );
    f.host.fs.faults.remove(&Phase::TrashSync);
    f.restart();
    f.edit("a.md", "later edit");
    assert_eq!(f.save("a.md"), Outcome::Published);
}

#[test]
fn recreation_and_undo_refreshes_keep_pending_payload_across_restart() {
    for bytes in ["recreated", "A"] {
        let mut f = Fixture::new();
        assert_eq!(f.host.delete("a.md"), Disposition::Pending);
        f.drain();
        let pending = f.host.logical_drafts()["a.md"].pending_trash.clone();
        f.host
            .fs
            .faults
            .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
        assert_eq!(f.save("a.md"), Outcome::Uncertain);
        assert_eq!(f.host.observe("a.md"), Disposition::Applied);
        f.edit("a.md", bytes);
        assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
        f.drain();
        assert_eq!(f.host.logical_drafts()["a.md"].pending_trash, pending);
        f.restart();
        assert_eq!(f.host.observe("a.md"), Disposition::Applied);
        // An unrelated unusable entry must never participate in this save.
        fs::create_dir(f.trash.join("unrelated-unreadable__a.md")).unwrap();
        assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
        f.host.advance_save(0); // temp
        f.host.advance_save(0); // check
        f.host.advance_save(0); // graph rename
        assert_eq!(f.host.job.as_ref().unwrap().phase, SavePhase::TrashSync);
        crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
        crate::atomic_file::FAIL_FILE_SYNC.with(|flag| flag.set(true));
        f.host.advance_save(0);
        assert!(f.host.pages["a.md"].risk);
        assert_eq!(f.host.trash["a.md"].pending, pending);
        assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
        f.drain();
        f.restart();
        assert_eq!(f.host.observe("a.md"), Disposition::Applied);
        crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
        assert_eq!(f.save("a.md"), Outcome::Published);
        assert_eq!(crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get), 2);
        assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), bytes.as_bytes());
        assert!(!f.host.trash.contains_key("a.md"));
        // A subsequent at-risk capture persists the cleared custody, too.
        f.edit("a.md", "later");
        f.host
            .fs
            .faults
            .insert(Phase::PageTemp, [io::ErrorKind::Other].into());
        assert_eq!(f.save("a.md"), Outcome::Failed);
        assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
        f.drain();
        assert!(f.host.logical_drafts()["a.md"].pending_trash.is_empty());
    }
}

#[test]
fn collision_rewrite_is_durable_before_move_and_survives_restart() {
    let mut f = Fixture::new();
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    let original = f.host.logical_drafts()["a.md"].trash.unwrap();
    let occupied = f
        .trash
        .join(format!("{}__a.md", super::trash_name(original)));
    fs::write(&occupied, b"foreign").unwrap();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0);
    f.host.advance_save(0); // EEXIST starts a custody rewrite
    f.host
        .fs
        .faults
        .insert(Phase::DraftTemp, [io::ErrorKind::Other].into());
    f.host.advance_draft();
    assert_eq!(f.host.advance_save(0), Disposition::Waiting);
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"A");
    f.drain();
    let record = f.host.logical_drafts()["a.md"].clone();
    assert_ne!(record.trash, Some(original));
    assert_eq!(record.pending_trash, vec![record.trash.unwrap()]);
    f.restart(); // rewritten name survived; no move has happened
    assert_eq!(f.host.observe("a.md"), Disposition::Applied);
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(fs::read(occupied).unwrap(), b"foreign");
    assert_eq!(
        fs::read(f.trash.join(format!(
            "{}__a.md",
            super::trash_name(record.trash.unwrap())
        )))
        .unwrap(),
        b"A"
    );
}

#[test]
fn recovered_candidate_collision_keeps_both_actual_pending_payloads() {
    for restart_before_collision in [false, true] {
        let mut f = Fixture::new();
        assert_eq!(f.host.delete("a.md"), Disposition::Pending);
        f.drain();
        let first = f.host.logical_drafts()["a.md"].trash.unwrap();
        f.host
            .fs
            .faults
            .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
        assert_eq!(f.save("a.md"), Outcome::Uncertain);
        if restart_before_collision {
            f.restart();
        }
        // The previous move happened, but another editor has recreated the source.
        // The recovered candidate's collision must not drop the earlier payload.
        fs::write(f.graph.join("a.md"), b"second").unwrap();
        let version = f.host.pages["a.md"].version;
        f.send(
            "a.md",
            RequestKind::Submit {
                bytes: None,
                version,
                resolve: Some(Some(Arc::from(b"second".as_slice()))),
            },
        );
        assert_eq!(f.host.observe("a.md"), Disposition::Applied);
        assert!(!f.host.pages["a.md"].conflict);
        assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
        f.drain();
        assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
        f.host.advance_save(0);
        f.host.advance_save(0); // collision with first, already moved payload
        f.drain();
        let pending = f.host.logical_drafts()["a.md"].pending_trash.clone();
        assert_eq!(pending.len(), 2);
        assert!(pending.contains(&first));
        f.host.advance_save(0); // second move
        f.restart();
        assert_eq!(f.host.observe("a.md"), Disposition::Applied);
        crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
        assert_eq!(f.save("a.md"), Outcome::Published);
        assert_eq!(crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get), 2);
        assert_eq!(fs::read_dir(&f.trash).unwrap().count(), 2);
    }
}

#[test]
fn k_page_operation_keeps_independent_deletion_custody_after_restart() {
    let mut f = Fixture::new();
    // The model's abstract rewriter may produce no file. Exercise the shared
    // operation vehicle with two deletions plus one receiving page (k = 3).
    assert_eq!(
        f.host.rename_with(
            "a.md",
            "c.md",
            &BTreeSet::from(["b.md".into()]),
            |bytes, _, moving| Ok(if moving { bytes.clone() } else { None })
        ),
        Disposition::Pending
    );
    f.drain();
    for page in ["a.md", "b.md"] {
        f.host
            .fs
            .faults
            .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
        assert_eq!(f.save(page), Outcome::Uncertain);
    }
    f.restart();
    for page in ["a.md", "b.md"] {
        assert_eq!(f.host.logical_drafts()[page].pending_trash.len(), 1);
        assert_eq!(f.host.observe(page), Disposition::Applied);
        crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
        assert_eq!(f.save(page), Outcome::Published);
        assert_eq!(crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get), 1);
    }
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(fs::read(f.graph.join("c.md")).unwrap(), b"A");
    assert_eq!(fs::read_dir(&f.trash).unwrap().count(), 2);
}

#[test]
fn new_graph_directory_retry_keeps_the_missing_ancestor_sync_obligation() {
    let mut f = Fixture::new();
    let parent = f.graph.join("new/deeper");
    let page = "new/deeper/d.md";
    crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(io::ErrorKind::Other)));
    assert!(f
        .host
        .fs
        .page_temp(page, &Some(Arc::from(b"draft".as_slice())))
        .is_err());
    assert!(parent.exists());
    #[cfg(feature = "test-faults")]
    crate::directory_durability::take_synced_directories();
    f.host
        .fs
        .page_temp(page, &Some(Arc::from(b"draft".as_slice())))
        .unwrap();
    #[cfg(feature = "test-faults")]
    assert_eq!(
        crate::directory_durability::take_synced_directories(),
        vec![f.graph.clone(), f.graph.join("new")]
    );
    f.host.fs.page_finish(page);
}

#[test]
fn semantic_rename_preserves_unrelated_prose_title_aliases_and_org_literals() {
    for (key, raw, expected) in [
        (
            "b.md",
            "title:: Referrer\nalias:: Alias\n- A prose [[A]] [[Other]] `[[A]]`\n",
            "title:: Referrer\nalias:: Alias\n- A prose [[C]] [[Other]] `[[A]]`\n",
        ),
        (
            "b.org",
            "#+TITLE: Referrer\n* A prose [[A]] [[Other]]\n#+BEGIN_SRC\n[[A]]\n#+END_SRC\n",
            "#+TITLE: Referrer\n* A prose [[C]] [[Other]]\n#+BEGIN_SRC\n[[A]]\n#+END_SRC\n",
        ),
    ] {
        let mut f = Fixture::new();
        fs::write(
            f.graph.join("a.md"),
            b"title:: A\nalias:: SourceAlias\n- body\n",
        )
        .unwrap();
        fs::write(f.graph.join(key), raw).unwrap();
        if key != "b.md" {
            f.host.keys.insert(key.into());
            f.host.locks.insert(key.into(), Arc::new(Mutex::new(())));
        }
        // This clean held buffer was never supplied in the caller's index list.
        f.send(key, RequestKind::Open);
        assert_eq!(
            f.host.rename(
                "a.md",
                "c.md",
                &BTreeSet::new(),
                "A",
                "C",
                tine_core::config::FileNameFormat::TripleLowbar
            ),
            Disposition::Pending
        );
        f.drain();
        assert_eq!(f.host.pages[key].buf.as_deref(), Some(expected.as_bytes()));
        assert_eq!(
            f.host.pages["c.md"].buf.as_deref(),
            Some(b"title:: C\nalias:: SourceAlias\n- body\n".as_slice())
        );
        assert_eq!(f.save(key), Outcome::Published);
        assert_eq!(fs::read(f.graph.join(key)).unwrap(), expected.as_bytes());
    }
}

fn record(wseq: u64, bytes: &str) -> Record {
    Record {
        page: "a.md".into(),
        wseq,
        version: wseq,
        base: Base::Known(Some(Arc::from(b"A".as_slice()))),
        bytes: Some(Arc::from(bytes.as_bytes())),
        trash: None,
        pending_trash: vec![],
    }
}

#[test]
fn drafts_have_separate_readable_and_durable_witnesses_and_removal_retries_only_sync() {
    let mut f = Fixture::new();
    let name = drafts::page_name("a.md");
    let mut vehicle = Vehicle::write(name.clone(), &[record(1, "mine")]);
    vehicle.advance(&mut f.host.fs);
    assert_eq!(vehicle.stage, Stage::Rename);
    assert!(f.host.fs.draft_files(false).is_empty());
    vehicle.advance(&mut f.host.fs);
    assert_eq!(vehicle.stage, Stage::Sync);
    assert_eq!(f.host.fs.draft_files(false).len(), 1);
    assert!(f.host.fs.draft_files(true).is_empty());
    vehicle.advance(&mut f.host.fs);
    assert_eq!(vehicle.stage, Stage::Present);
    assert_eq!(
        drafts::scan(f.host.fs.draft_files(true)).logical["a.md"],
        record(1, "mine")
    );
    let path = f.app.join("drafts-v2/test-graph").join(&name);
    assert_eq!(&fs::read(&path).unwrap()[..8], b"TINEDRF2");
    let mut remove = Vehicle::remove(name);
    remove.advance(&mut f.host.fs);
    assert!(!path.exists());
    f.host
        .fs
        .faults
        .insert(Phase::DraftSync, [io::ErrorKind::Other].into());
    remove.advance(&mut f.host.fs);
    assert_eq!(remove.stage, Stage::UnlinkSync);
    assert_eq!(f.host.fs.draft_files(true).len(), 1);
    // Any repeated unlink would fail; retry must perform the missing sync only.
    f.host
        .fs
        .faults
        .insert(Phase::DraftUnlink, [io::ErrorKind::PermissionDenied].into());
    remove.advance(&mut f.host.fs);
    assert_eq!(remove.stage, Stage::Absent);
    assert!(f.host.fs.draft_files(true).is_empty());
    assert_eq!(f.host.fs.faults[&Phase::DraftUnlink].len(), 1);
}

#[test]
fn app_data_einval_and_persistent_errors_never_become_unsupported_or_release_custody() {
    let mut f = Fixture::new();
    f.edit("a.md", "mine");
    f.host.switch_request();
    assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
    f.host.advance_draft();
    f.host.advance_draft();
    for _ in 0..3 {
        crate::directory_durability::SYNC_ERROR
            .with(|error| error.set(Some(io::ErrorKind::InvalidInput)));
        f.host.advance_draft();
    }
    assert_eq!(
        f.host.worker.as_ref().unwrap().task.stage,
        Stage::CleanupUnlink
    );
    f.host.advance_draft();
    for _ in 0..4 {
        crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(io::ErrorKind::Other)));
        f.host.advance_draft();
        assert_eq!(
            f.host.worker.as_ref().unwrap().task.stage,
            Stage::CleanupSync
        );
        assert_eq!(f.host.start_save("a.md"), Disposition::Waiting);
        assert!(f.host.logical_drafts().is_empty());
    }
    f.drain();
    assert!(f.host.pages["a.md"].risk);
    assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(
        f.host.logical_drafts()["a.md"].bytes,
        f.host.pages["a.md"].buf
    );
}

#[test]
fn failed_fresh_vehicle_does_not_touch_an_existing_draft() {
    let mut f = Fixture::new();
    let old_name = drafts::page_name("a.md");
    let mut old = Vehicle::write(old_name.clone(), &[record(1, "old")]);
    for _ in 0..3 {
        old.advance(&mut f.host.fs);
    }
    for phase in [Phase::DraftTemp, Phase::DraftRename] {
        f.host
            .fs
            .faults
            .insert(phase, [io::ErrorKind::Other].into());
        let mut fresh = Vehicle::write(drafts::page_name("a.md"), &[record(2, "new")]);
        for _ in 0..4 {
            fresh.advance(&mut f.host.fs);
        }
        assert_eq!(fresh.stage, Stage::Absent);
        assert_eq!(
            drafts::scan(f.host.fs.draft_files(true)).logical["a.md"],
            record(1, "old")
        );
    }
    // Real EEXIST on a fresh-file publication also leaves the earlier vehicle.
    let mut collision = Vehicle::write(old_name, &[record(2, "new")]);
    collision.advance(&mut f.host.fs);
    collision.advance(&mut f.host.fs);
    assert_eq!(collision.stage, Stage::Absent);
    assert_eq!(
        drafts::scan(f.host.fs.draft_files(false)).logical["a.md"],
        record(1, "old")
    );
}

#[test]
fn process_restart_selects_newest_draft_preserves_corruption_and_ignores_v1() {
    let mut f = Fixture::new();
    let v1 = f.app.join("drafts/test-graph.v1.json");
    fs::create_dir_all(v1.parent().unwrap()).unwrap();
    fs::write(&v1, b"untouched-v1").unwrap();
    for (seq, bytes) in [(1, "older"), (2, "newest")] {
        let mut vehicle = Vehicle::write(drafts::page_name("a.md"), &[record(seq, bytes)]);
        for _ in 0..3 {
            vehicle.advance(&mut f.host.fs);
        }
    }
    let dir = f.app.join("drafts-v2/test-graph");
    let mut corrupt = drafts::encode(&[record(3, "corrupt")]);
    corrupt[10] ^= 1;
    fs::write(dir.join("p-broken.draft"), &corrupt).unwrap();
    f.restart();
    assert_eq!(
        f.host.pages["a.md"].buf.as_deref(),
        Some(b"newest".as_slice())
    );
    assert!(f.host.pages["a.md"].typed && f.host.pages["a.md"].risk);
    assert!(!f.host.pages["a.md"].conflict);
    assert!(f.host.pages["a.md"].version > 2);
    assert_eq!(f.host.fs.draft_files(true).len(), 1);
    assert!(f
        .host
        .events
        .contains(&Event::Unreadable("p-broken.draft".into())));
    let backups: Vec<_> = fs::read_dir(dir.join("unreadable")).unwrap().collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read(backups[0].as_ref().unwrap().path()).unwrap(),
        corrupt
    );
    assert_eq!(fs::read(v1).unwrap(), b"untouched-v1");
    assert!(f.root.path().exists());
}

#[test]
fn real_operation_install_explosion_and_restart_keep_all_records() {
    let mut f = Fixture::new();
    fs::write(f.graph.join("b.md"), b"- [[A]]\n").unwrap();
    assert_eq!(
        f.host.rename(
            "a.md",
            "c.md",
            &BTreeSet::from(["b.md".into()]),
            "A",
            "C",
            tine_core::config::FileNameFormat::TripleLowbar
        ),
        Disposition::Pending
    );
    for _ in 0..3 {
        f.host.advance_draft();
    }
    assert_eq!(f.host.start_save("c.md"), Disposition::Waiting);
    f.restart(); // durable-but-unapplied op recovered from real files
    assert_eq!(f.host.pages["c.md"].buf.as_deref(), Some(b"A".as_slice()));
    assert_eq!(f.host.pages["a.md"].buf, None);
    assert_eq!(
        f.host.pages["b.md"].buf.as_deref(),
        Some(b"- [[C]]\n".as_slice())
    );
    let files = f.host.fs.draft_files(true);
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|(name, _)| name.starts_with("p-")));
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(fs::read(f.graph.join("c.md")).unwrap(), b"A");
}
