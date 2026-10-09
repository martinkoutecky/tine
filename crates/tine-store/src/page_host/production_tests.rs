//! Focused native/filesystem target: cargo test -p tine-store --lib page_host::production_tests
//! Runs unchanged on Linux, Windows/NTFS and macOS/APFS. Native power cuts are
//! deliberately excluded; ModelFs owns those claims.
use super::drafts::{self, Stage, Vehicle};
use super::io::{HostIo, Phase, Witness};
use super::production::ProductionIo;
use super::tests::text;
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

    /// This save's own outcome; a guard that observed a change has none.
    fn save(&mut self, page: &str) -> Outcome {
        assert_eq!(self.host.start_save(page), Disposition::Pending);
        let start = self.host.events.len();
        for _ in 0..15 {
            self.host.advance_save(0);
            if self.host.job.is_none() {
                return self.host.events[start..]
                    .iter()
                    .rev()
                    .find_map(|event| {
                        if let Event::SaveOutcome { outcome, .. } = event {
                            Some(*outcome)
                        } else {
                            None
                        }
                    })
                    .expect("the guard observed a change; no save outcome");
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
    f.host.advance_save(0); // guard
    f.host.advance_save(0); // marker
    f.host.advance_save(0); // move
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
    f.host.advance_save(0); // marker
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
    // The owed payload (custody before the save), then the retry's marker.
    assert_eq!(crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get), 2);
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
    // Custody (a)+(b) completed before the failed source sync, so the marker
    // retired with that outcome (A4 rule 2.5). A recapture (nothing new to
    // write here) and a restart must not revive the finished debt.
    assert_eq!(markers(&mut f), 0);
    if f.host.begin_draft("a.md") == Disposition::Pending {
        f.drain();
    }
    f.restart();
    assert!(f.host.custody.is_empty());
    assert_eq!(f.host.observe("a.md"), Disposition::Applied);
    reset_syncs();
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(
        syncs(),
        1,
        "only the restored deletion's own marker; finished payload debt was revived"
    );
}

/// Physical entries in the custody directory, temps included (V2 census).
fn markers(f: &mut Fixture) -> usize {
    fs::read_dir(custody_dir(f)).map_or(0, Iterator::count)
}

fn syncs() -> u64 {
    crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get)
}

fn reset_syncs() {
    crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
}

fn trash_holds(f: &Fixture, bytes: &[u8]) -> bool {
    fs::read_dir(&f.trash)
        .unwrap()
        .any(|entry| fs::read(entry.unwrap().path()).is_ok_and(|b| b == bytes))
}

/// Custody phase (b): the trash directory and each ancestor up to the root.
fn chain(f: &Fixture) -> Vec<PathBuf> {
    f.trash
        .ancestors()
        .take_while(|dir| dir.starts_with(&f.graph))
        .map(PathBuf::from)
        .collect()
}

fn custody_dir(f: &Fixture) -> PathBuf {
    f.app.join("drafts-v2/test-graph/trash-custody")
}

/// Open, delete and run the delete phase up to (and including) the move.
fn delete_through_move(f: &mut Fixture) {
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0); // guard
    f.host.advance_save(0); // marker
    f.host.advance_save(0); // move
    assert!(!f.graph.join("a.md").exists());
    assert_eq!(f.host.job.as_ref().unwrap().phase, SavePhase::TrashSync);
    assert_eq!(markers(f), 1);
}

/// Process crash and relaunch, returning the directories launch synced.
fn relaunch(f: &mut Fixture) -> Vec<PathBuf> {
    f.host.stop();
    let faults = std::mem::take(&mut f.host.fs.faults);
    f.host.fs = ProductionIo::new(&f.graph, &f.app, "test-graph", &f.trash).unwrap();
    f.host.fs.faults = faults;
    #[cfg(feature = "test-faults")]
    crate::directory_durability::take_synced_directories();
    assert!(matches!(
        f.host.launch(),
        Disposition::Applied | Disposition::Pending
    ));
    #[cfg(feature = "test-faults")]
    let synced = crate::directory_durability::take_synced_directories();
    #[cfg(not(feature = "test-faults"))]
    let synced = vec![];
    f.drain();
    synced
}

/// REVIEW-2b F1: every deletion producer (opDelete, an absent-text submit, a
/// Move source) takes the one delete phase: marker, move, payload data sync,
/// trash ancestors, publication, then marker retirement.

#[test]
fn every_deletion_producer_takes_the_one_delete_phase() {
    for producer in ["opDelete", "absent submit", "Move source"] {
        let mut f = Fixture::new();
        f.send("a.md", RequestKind::Open);
        match producer {
            "opDelete" => {
                assert_eq!(f.host.delete("a.md"), Disposition::Pending);
                f.drain();
            }
            "absent submit" => {
                let version = f.host.pages["a.md"].version;
                f.send(
                    "a.md",
                    RequestKind::Submit {
                        bytes: None,
                        version,
                        resolve: None,
                    },
                );
            }
            _ => {
                f.send("b.md", RequestKind::Open);
                let source_version = f.host.pages["a.md"].version;
                let receiver_version = f.host.pages["b.md"].version;
                f.send(
                    "a.md",
                    RequestKind::Move {
                        receiver: "b.md".into(),
                        source_text: None,
                        receiver_text: Some(Arc::from(b"AB".as_slice())),
                        source_version,
                        receiver_version,
                    },
                );
                f.drain();
            }
        }
        reset_syncs();
        #[cfg(feature = "test-faults")]
        crate::directory_durability::take_synced_directories();
        assert_eq!(f.save("a.md"), Outcome::Published, "{producer}");
        assert!(!f.graph.join("a.md").exists());
        // The marker's own temp fsync, then the payload's data (custody (a)).
        assert_eq!(
            syncs(),
            2,
            "{producer}: published with no payload data sync"
        );
        #[cfg(feature = "test-faults")]
        {
            let synced = crate::directory_durability::take_synced_directories();
            let retire = synced.iter().rposition(|d| *d == custody_dir(&f)).unwrap();
            let written = synced.iter().position(|d| *d == custody_dir(&f)).unwrap();
            let trash = synced.iter().position(|d| *d == f.trash).unwrap();
            assert!(
                written < trash,
                "{producer}: marker entry not durable before the move"
            );
            for dir in chain(&f) {
                assert!(
                    synced[..retire].contains(&dir),
                    "{producer}: {dir:?} not synced before retirement: {synced:?}"
                );
            }
        }
        assert_eq!(markers(&mut f), 0, "{producer}");
        assert!(trash_holds(&f, b"A"), "{producer}");
    }
}

/// REVIEW-2b F2 + REVIEW-A2 B2 + REVIEW-A3 B2: an interrupted trash chain
/// (an ancestor sync failed, or a crash right after the move) is completed by
/// launch: payload data, then the whole ancestor chain, then retirement, and
/// all of it before the graph-launch source-directory syncs.
#[test]
fn launch_redoes_payload_and_ancestor_custody_before_retiring_the_marker() {
    for crash_before_b in [false, true] {
        let mut f = Fixture::new();
        fs::remove_dir_all(f.graph.join("logseq")).unwrap();
        delete_through_move(&mut f);
        if !crash_before_b {
            crate::directory_durability::SYNC_ERROR
                .with(|error| error.set(Some(io::ErrorKind::Other)));
            f.host.advance_save(0);
            assert!(f.host.pages["a.md"].risk);
            assert_eq!(markers(&mut f), 1, "an Uncertain chain keeps its marker");
        }
        reset_syncs();
        let synced = relaunch(&mut f);
        assert_eq!(syncs(), 1, "launch redoes the payload data sync");
        #[cfg(feature = "test-faults")]
        {
            let chain = chain(&f);
            assert_eq!(
                synced.get(..chain.len()),
                Some(chain.as_slice()),
                "{synced:?}"
            );
            assert_eq!(
                synced.get(chain.len()),
                Some(&custody_dir(&f)),
                "{synced:?}"
            );
        }
        let _ = synced;
        assert_eq!(markers(&mut f), 0);
        assert!(trash_holds(&f, b"A"));
    }
}

/// Launch covers every existing ancestor of a nested page, not just its
/// parent: an interrupted nested creation leaves entries no leaf sync covers.
#[cfg(feature = "test-faults")]
#[test]
fn launch_syncs_every_ancestor_of_a_nested_page() {
    let mut f = Fixture::new();
    fs::create_dir_all(f.graph.join("x/y")).unwrap();
    fs::write(f.graph.join("x/y/z.md"), b"Z").unwrap();
    f.host.keys.insert("x/y/z.md".into());
    let synced = relaunch(&mut f);
    for dir in ["x", "x/y"] {
        assert!(synced.contains(&f.graph.join(dir)), "{dir}: {synced:?}");
    }
}

/// REVIEW-2b-r2 R1: launch also syncs every existing directory of the trash
/// chain, because a crash between a deletion's mkdirs and their parent syncs
/// loses the obligation and the next creation sees the entries. A trash
/// directory that does not exist is skipped, not a warning.
#[cfg(feature = "test-faults")]
#[test]
fn launch_syncs_the_existing_trash_chain() {
    let mut f = Fixture::new();
    let chain = ["logseq", "logseq/.tine-trash", "logseq/.tine-trash/pages"];
    let synced = relaunch(&mut f);
    for dir in chain {
        assert!(synced.contains(&f.graph.join(dir)), "{dir}: {synced:?}");
    }
    fs::remove_dir_all(f.graph.join(chain[1])).unwrap();
    let synced = relaunch(&mut f);
    assert!(synced.contains(&f.graph.join(chain[0])), "{synced:?}");
    for dir in &chain[1..] {
        assert!(!synced.contains(&f.graph.join(dir)), "{dir}: {synced:?}");
    }
    assert!(
        f.host.fs.launch_warnings.is_empty(),
        "{:?}",
        f.host.fs.launch_warnings
    );
}

/// The no-crash neighbour: a failed parent sync in the trash chain creation is
/// retained in the process, and the next move redoes it before moving, even
/// though every chain directory now exists.
#[cfg(feature = "test-faults")]
#[test]
fn a_failed_trash_chain_sync_is_redone_before_the_next_move() {
    let f = &mut Fixture::new();
    fs::remove_dir_all(f.graph.join("logseq")).unwrap();
    crate::directory_durability::SYNC_ERROR.with(|error| error.set(Some(io::ErrorKind::Other)));
    assert!(f.host.fs.trash_move("a.md", "p1").result.is_err());
    assert!(f.trash.is_dir() && f.graph.join("a.md").exists());
    crate::directory_durability::take_synced_directories();
    assert!(f.host.fs.trash_move("a.md", "p1").result.is_ok());
    let owed = ["", "logseq", "logseq/.tine-trash"].map(|dir| f.graph.join(dir));
    assert_eq!(crate::directory_durability::take_synced_directories(), owed);
    assert_eq!(fs::read(f.trash.join("p1")).unwrap(), b"A");
}

/// Malformed imported state: a checksummed marker that names no page, or a
/// payload outside the trash directory, is quarantined, never acted on, and
/// the graph still opens.
#[test]
fn malformed_custody_markers_are_quarantined_not_acted_on() {
    let mut f = Fixture::new();
    let cases = [("", "x__a.md"), ("a.md", "../a.md"), ("a.md", "sub/a.md")];
    fs::create_dir_all(custody_dir(&f)).unwrap();
    for (i, (page, payload)) in cases.iter().enumerate() {
        let marker = drafts::Marker {
            page: page.to_string(),
            payload: payload.to_string(),
        };
        fs::write(
            custody_dir(&f).join(format!("{i}.tcm")),
            drafts::encode_marker(&marker),
        )
        .unwrap();
    }
    relaunch(&mut f);
    for i in 0..cases.len() {
        let event = Event::Unreadable(format!("trash-custody/{i}.tcm"));
        assert!(f.host.events.contains(&event), "{i}: {:?}", f.host.events);
    }
    assert!(f.host.custody.is_empty());
    assert_eq!(markers(&mut f), 0);
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"A");
}

#[test]
fn restart_must_not_forget_unflushed_trash_payload() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0);
    f.host.advance_save(0);
    fs::write(f.graph.join("a.md"), b"unflushed-external").unwrap();
    f.host.advance_save(0);
    crate::atomic_file::FAIL_FILE_SYNC.with(|flag| flag.set(true));
    f.host.advance_save(0);
    assert_eq!(markers(&mut f), 1);
    reset_syncs();
    f.restart();
    assert_eq!(f.host.observe("a.md"), Disposition::Applied);
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert!(
        syncs() >= 1,
        "Published after restart without flushing the readable trash payload"
    );
    assert_eq!(markers(&mut f), 0);
    assert!(trash_holds(&f, b"unflushed-external"));
}

/// A marker whose payload was never moved (crash between marker and move)
/// owes nothing: launch retires it without listing the trash.
#[test]
fn restart_after_marker_before_move_retires_the_missing_payload_marker() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0); // guard
    f.host.advance_save(0); // marker
    assert_eq!(markers(&mut f), 1);
    relaunch(&mut f);
    assert_eq!(markers(&mut f), 0);
    assert!(f.host.custody.is_empty());
    f.edit("a.md", "recreated");
    reset_syncs();
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(syncs(), 1, "only the page's own temp");
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"recreated");
    assert_eq!(fs::read_dir(&f.trash).unwrap().count(), 0);
}

/// REVIEW-2b F3 + REVIEW-A3: a collision after launch rewrites no draft and
/// leaves no unused marker; the retry uses a fresh payload name.
#[test]
fn collision_after_launch_rewrites_no_draft_and_leaves_no_unused_marker() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    f.restart();
    let drafts = f.host.fs.draft_files(true);
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0); // guard
    f.host.advance_save(0); // marker
    let (_, payload) = f.host.job.as_ref().unwrap().marker.clone().unwrap();
    fs::write(f.trash.join(&payload), b"foreign").unwrap();
    f.host.advance_save(0); // real EEXIST
    assert!(
        f.host.worker.is_none(),
        "collision installed a draft rewrite"
    );
    assert_eq!(f.host.job.as_ref().unwrap().phase, SavePhase::Marker);
    assert_eq!(markers(&mut f), 0, "unused marker retired");
    assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"A");
    f.restart(); // a crash between the collision and the retry
    assert_eq!(f.host.fs.draft_files(true), drafts);
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(fs::read(f.trash.join(&payload)).unwrap(), b"foreign");
    assert!(trash_holds(&f, b"A"));
    assert_eq!(markers(&mut f), 0);
}

/// REVIEW-2b F4: delete/recreate cycles whose trash witnesses are Unsupported
/// leave zero markers and do bounded, non-growing work per cycle.
#[test]
fn weak_trash_cycles_leave_zero_markers_and_bounded_work() {
    let mut f = Fixture::new();
    let mut per_cycle = vec![];
    let weak_save = |f: &mut Fixture| {
        assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
        for _ in 0..12 {
            if f.host.job.is_none() {
                break;
            }
            if matches!(
                f.host.job.as_ref().unwrap().phase,
                SavePhase::TrashSync | SavePhase::Custody
            ) {
                crate::directory_durability::SYNC_ERROR
                    .with(|error| error.set(Some(io::ErrorKind::InvalidInput)));
            }
            f.host.advance_save(0);
        }
        assert!(f.host.pages["a.md"].clean());
    };
    for cycle in 0..6 {
        f.send("a.md", RequestKind::Open);
        assert_eq!(f.host.delete("a.md"), Disposition::Pending);
        f.drain();
        reset_syncs();
        weak_save(&mut f);
        f.edit("a.md", &format!("again {cycle}"));
        weak_save(&mut f);
        per_cycle.push(syncs());
        assert_eq!(markers(&mut f), 0, "cycle {cycle}");
        assert!(f.host.custody.is_empty(), "cycle {cycle}");
    }
    assert!(
        per_cycle.iter().all(|&n| n == per_cycle[0]),
        "file syncs per cycle grow: {per_cycle:?}"
    );
}

/// REVIEW-2b F5 / R-STORAGE-ERROR: a payload that cannot be synced (here a
/// foreign unreadable file at its name) fails saves of the page twice, then
/// the three-failure escape lets the save go ahead with a sticky error.
/// Discard never waits for trash custody.
#[cfg(unix)]
#[test]
fn unsyncable_payload_escapes_after_three_failures_and_never_blocks_discard() {
    use std::os::unix::fs::PermissionsExt;
    for discard in [false, true] {
        let mut f = Fixture::new();
        delete_through_move(&mut f);
        f.host
            .fs
            .faults
            .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
        f.host.advance_save(0);
        assert!(f.host.job.is_none());
        let payload = fs::read_dir(&f.trash)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        fs::set_permissions(&payload, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(&payload).is_ok() {
            eprintln!("running with CAP_DAC_OVERRIDE; the unreadable fault is unavailable");
            return;
        }
        relaunch(&mut f);
        assert_eq!(markers(&mut f), 1, "unfinished custody is kept as debt");
        assert_eq!(f.host.observe("a.md"), Disposition::Applied);
        f.edit("a.md", "recreated");
        if discard {
            let version = f.host.pages["a.md"].version;
            f.send("a.md", RequestKind::Discard { version });
            assert!(!f.host.pages["a.md"].typed && f.host.pages["a.md"].buf.is_none());
            f.host
                .fs
                .faults
                .insert(Phase::TrashSync, [io::ErrorKind::Other; 3].into());
            if f.host.begin_draft("a.md") == Disposition::Pending {
                f.drain();
            }
            assert!(f.host.worker.is_none());
            assert_eq!(
                f.host.fs.faults[&Phase::TrashSync].len(),
                3,
                "no trash barrier"
            );
            continue;
        }
        assert_eq!(f.save("a.md"), Outcome::Failed);
        assert_eq!(f.save("a.md"), Outcome::Failed);
        assert_eq!(f.save("a.md"), Outcome::Published);
        assert!(f
            .host
            .events
            .iter()
            .any(|event| matches!(event, Event::CustodyError { .. })));
        assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), b"recreated");
        fs::set_permissions(&payload, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(markers(&mut f), 1, "the marker stays for the next launch");
        relaunch(&mut f);
        assert_eq!(markers(&mut f), 0);
    }
}

/// REVIEW-2b F9: a recreation (or undo) of P never renames before P's custody
/// phases complete. Unrelated trash entries never participate (REVIEW-A3 R1).
#[test]
fn recreation_never_renames_before_custody_completes() {
    for bytes in ["recreated", "A"] {
        for restart in [false, true] {
            let mut f = Fixture::new();
            delete_through_move(&mut f);
            f.host
                .fs
                .faults
                .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
            f.host.advance_save(0);
            assert!(f.host.pages["a.md"].risk);
            assert_eq!(f.host.observe("a.md"), Disposition::Applied);
            fs::create_dir(f.trash.join("unrelated-unreadable__a.md")).unwrap();
            if restart {
                f.host
                    .fs
                    .faults
                    .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
                relaunch(&mut f);
                assert_eq!(f.host.fs.faults[&Phase::TrashSync].len(), 0);
                assert_eq!(markers(&mut f), 1);
                assert_eq!(f.host.observe("a.md"), Disposition::Applied);
            }
            f.edit("a.md", bytes);
            f.host
                .fs
                .faults
                .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
            assert_eq!(f.save("a.md"), Outcome::Failed);
            assert!(!f.graph.join("a.md").exists(), "renamed before custody");
            reset_syncs();
            assert_eq!(f.save("a.md"), Outcome::Published);
            assert_eq!(syncs(), 2, "payload, then the page's own temp");
            assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), bytes.as_bytes());
            assert_eq!(markers(&mut f), 0);
            assert!(f.host.custody.is_empty());
        }
    }
}

/// Discard has no trash barrier (A4 rule 4): it completes while trash
/// syncs would fail, and the marker carries the custody to the next launch.
#[test]
fn discard_has_no_trash_barrier_and_launch_completes_custody() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    f.host.advance_save(0);
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
    f.host
        .fs
        .faults
        .insert(Phase::TrashSync, [io::ErrorKind::Other; 3].into());
    if f.host.begin_draft("a.md") == Disposition::Pending {
        f.drain();
    }
    assert!(f.host.logical_drafts().is_empty());
    assert_eq!(
        f.host.fs.faults[&Phase::TrashSync].len(),
        3,
        "no trash barrier"
    );
    f.host.fs.faults.clear();
    assert_eq!(markers(&mut f), 1);
    reset_syncs();
    relaunch(&mut f);
    assert_eq!(syncs(), 1);
    assert_eq!(markers(&mut f), 0);
    assert!(trash_holds(&f, b"unflushed R1 payload"));
    f.edit("a.md", "later edit");
    assert_eq!(f.save("a.md"), Outcome::Published);
}

/// A second deletion while the first payload's custody is owed: the save
/// completes the earlier custody first, then moves the new source under a
/// fresh name; both payloads survive and no marker remains.
#[test]
fn second_deletion_with_owed_custody_keeps_both_payloads() {
    for restart_before in [false, true] {
        let mut f = Fixture::new();
        delete_through_move(&mut f);
        f.host
            .fs
            .faults
            .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
        f.host.advance_save(0);
        if restart_before {
            f.host
                .fs
                .faults
                .insert(Phase::TrashSync, [io::ErrorKind::Other].into());
            relaunch(&mut f);
            assert_eq!(f.host.fs.faults[&Phase::TrashSync].len(), 0);
            assert_eq!(markers(&mut f), 1);
        }
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
        reset_syncs();
        assert_eq!(f.save("a.md"), Outcome::Published);
        assert_eq!(syncs(), 3, "the owed payload, the marker, the new payload");
        assert!(trash_holds(&f, b"A") && trash_holds(&f, b"second"));
        assert_eq!(fs::read_dir(&f.trash).unwrap().count(), 2);
        assert_eq!(markers(&mut f), 0);
    }
}

/// An op over k pages with several deletions: each keeps its own marker
/// across a restart, and launch completes each independently.
#[test]
fn k_page_operation_keeps_independent_deletion_custody_after_restart() {
    let mut f = Fixture::new();
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
    assert_eq!(markers(&mut f), 2);
    reset_syncs();
    relaunch(&mut f);
    assert_eq!(syncs(), 2);
    assert_eq!(markers(&mut f), 0);
    for page in ["a.md", "b.md"] {
        assert_eq!(f.host.observe(page), Disposition::Applied);
        reset_syncs();
        assert_eq!(f.save(page), Outcome::Published);
        assert_eq!(syncs(), 1, "only the restored deletion's marker");
    }
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(fs::read(f.graph.join("c.md")).unwrap(), b"A");
    assert_eq!(fs::read_dir(&f.trash).unwrap().count(), 2);
}

/// REVIEW-A3 R1: the adapter locates payloads by recorded basename only. It
/// lists exactly two directories, both in app data: the draft store and its
/// custody markers. Listing the graph trash (or `pages/`) would scale with the
/// trash and act on foreign entries.
#[test]
fn the_adapter_never_lists_the_graph_or_its_trash() {
    let source = include_str!("production.rs");
    let listings: Vec<_> = source
        .lines()
        .filter(|line| line.contains("read_dir("))
        .map(str::trim)
        .collect();
    assert_eq!(
        listings,
        [
            "for entry in fs::read_dir(&drafts)? {",
            "for entry in fs::read_dir(&dir).map_err(report)? {"
        ],
        "only app-data directories are listed (REVIEW-A3 R1)"
    );
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

/// REVIEW-2b-r2 R2, hosted Windows only: under a plain (non-verbatim) app-data
/// root whose marker path passes MAX_PATH, the marker publish (the shared
/// no-replace move) succeeds and the deletion it guards completes.
#[cfg(windows)]
#[test]
fn windows_marker_publish_under_a_long_app_data_root() {
    let root = tempfile::tempdir().unwrap();
    let graph = root.path().join("graph");
    let mut app = root.path().join("app");
    while app.as_os_str().len() < 280 {
        app.push("a-long-app-data-directory-name");
    }
    let trash = graph.join("logseq/.tine-trash/pages");
    fs::create_dir_all(&trash).unwrap();
    fs::create_dir_all(&app).unwrap();
    fs::write(graph.join("a.md"), b"A").unwrap();
    let io = ProductionIo::new(&graph, &app, "test-graph", &trash).unwrap();
    let locks = ["a.md"]
        .into_iter()
        .map(|key| (key.into(), Arc::new(Mutex::new(()))))
        .collect();
    let mut f = Fixture {
        root,
        graph,
        app,
        trash,
        host: Host::new(io, locks),
    };
    assert!(custody_dir(&f).join("x".repeat(40)).as_os_str().len() > 300);
    f.send("a.md", RequestKind::Open);
    assert_eq!(f.host.delete("a.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert!(!f.graph.join("a.md").exists());
    assert!(trash_holds(&f, b"A"));
    assert_eq!(markers(&mut f), 0);
}

/// REVIEW-2b-r2 V1: a regular file named `trash-custody` (imported or left by
/// a crash; malformed state) is quarantined with its bytes preserved, the
/// directory is recreated, and the graph opens.
#[test]
fn r2_listing_error_never_blocks_launch() {
    let mut f = Fixture::new();
    f.host.stop();
    let dir = custody_dir(&f);
    let _ = fs::remove_dir(&dir);
    fs::write(&dir, b"not a directory").unwrap();
    f.host.fs = ProductionIo::new(&f.graph, &f.app, "test-graph", &f.trash).unwrap();
    assert_eq!(f.host.launch(), Disposition::Applied);
    assert!(f.host.alive);
    assert!(dir.is_dir());
    assert_eq!(f.host.custody_unknown, None);
    let event = Event::Unreadable("trash-custody".into());
    assert!(f.host.events.contains(&event));
    let unreadable = f.app.join("drafts-v2/test-graph/unreadable");
    let kept: Vec<_> = fs::read_dir(unreadable)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .collect();
    assert_eq!(kept, [b"not a directory".to_vec()]);
    f.drain();
    f.edit("a.md", "after");
    assert_eq!(f.save("a.md"), Outcome::Published);
    // The reviewer's order: the swap happens after the adapter's scan, so
    // launch's listing meets it; the graph still opens, custody unknown.
    f.host.stop();
    f.host.fs = ProductionIo::new(&f.graph, &f.app, "test-graph", &f.trash).unwrap();
    fs::remove_dir(&dir).unwrap();
    fs::write(&dir, b"swapped").unwrap();
    assert_eq!(f.host.launch(), Disposition::Applied);
    assert!(f.host.alive && f.host.custody_unknown.is_some());
}

/// REVIEW-2b-r2 V1: any other listing error (disk error) opens the graph with
/// a sticky custody-unknown condition that is never read as "no debt"; saves
/// proceed (R-STORAGE-ERROR), and a later successful listing adopts and
/// settles the debt as launch would, then clears the condition.
#[test]
fn r2_listing_failure_opens_the_graph_until_a_retry_settles_the_debt() {
    let mut f = Fixture::new();
    delete_through_move(&mut f);
    f.host.stop();
    f.host.fs = ProductionIo::new(&f.graph, &f.app, "test-graph", &f.trash).unwrap();
    f.host
        .fs
        .faults
        .insert(Phase::CustodyList, [io::ErrorKind::Other; 2].into());
    assert_eq!(f.host.launch(), Disposition::Applied);
    f.drain();
    let unknown = f.host.custody_unknown.clone().unwrap();
    assert!(unknown.contains("trash-custody"), "{unknown}");
    assert_eq!(markers(&mut f), 1, "unknown is not an empty listing");
    f.edit("b.md", "saved while unknown");
    assert_eq!(f.save("b.md"), Outcome::Published);
    f.host.recover_custody();
    assert!(
        f.host.custody_unknown.is_some(),
        "a failed retry stays unknown"
    );
    assert_eq!(markers(&mut f), 1);
    f.host.recover_custody();
    assert_eq!(f.host.custody_unknown, None);
    assert!(f.host.custody.is_empty());
    assert_eq!(markers(&mut f), 0);
    assert!(trash_holds(&f, b"A"));
}

/// REVIEW-2b-r2 V2: a reported retirement error after completed custody is
/// retire-only debt. Four delete/recreate cycles publish normally; once the
/// disk recovers, progress's backoff retry leaves no marker on disk.
#[test]
fn r2_retirement_errors_do_not_turn_into_history() {
    let mut f = Fixture::new();
    for cycle in 0..4 {
        f.send("a.md", RequestKind::Open);
        assert_eq!(f.host.delete("a.md"), Disposition::Pending);
        f.drain();
        let fault = [io::ErrorKind::Other].into();
        f.host.fs.faults.insert(Phase::CustodyRetire, fault);
        assert_eq!(f.save("a.md"), Outcome::Published);
        f.edit("a.md", &format!("recreated {cycle}"));
        assert_eq!(f.save("a.md"), Outcome::Published, "never blocks a save");
    }
    assert_eq!(markers(&mut f), 4);
    assert!(f.host.custody.is_empty(), "no custody is owed");
    let dir = custody_dir(&f);
    let Fixture { root, host, .. } = f;
    let clock = super::native_cost::ManualClock(std::cell::Cell::new(0));
    let mut p = super::progress::Progress::new(host, clock);
    for now in (0..=1000).step_by(100) {
        p.clock.0.set(now);
        p.poll(0);
    }
    assert!(p.host.retire.is_empty());
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    drop(root);
}

/// REVIEW-2b-r2 V2: a process cut between the marker temp's write and its
/// rename leaves an unpublished temp (no destructor runs). Launch reclaims
/// it, so repeated cuts never accumulate physical entries.
#[test]
fn r2_marker_temp_crash_cuts_do_not_accumulate() {
    let mut f = Fixture::new();
    for i in 0..4 {
        let dir = custody_dir(&f);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join(format!("{i}.tcm"));
        std::mem::forget(crate::atomic_file::PreparedWrite::new(&marker, b"M").unwrap());
        assert_eq!(markers(&mut f), 1);
        let synced = relaunch(&mut f);
        assert_eq!(markers(&mut f), 0);
        // The unlink is made durable; a launch with no temp owes no sync.
        #[cfg(feature = "test-faults")]
        assert!(synced.contains(&dir), "{synced:?}");
        #[cfg(feature = "test-faults")]
        assert!(!relaunch(&mut f).contains(&dir));
        let _ = synced;
    }
}

/// A new binding over the same graph and app data (STEP3 §2): no keys
/// until the binding registers them.
fn binding(f: &Fixture) -> Host<ProductionIo> {
    let io = ProductionIo::new(&f.graph, &f.app, "test-graph", &f.trash).unwrap();
    Host::new(io, BTreeMap::new())
}

/// A failed save puts the page at risk; its draft is then written.
fn draft_after_failed_save(f: &mut Fixture, key: &str) {
    f.host
        .fs
        .faults
        .insert(Phase::PageTemp, [io::ErrorKind::Other].into());
    assert_eq!(f.host.start_save(key), Disposition::Pending);
    f.host.advance_save(0);
    assert!(f.host.job.is_none() && f.host.pages[key].risk);
    if f.host.begin_draft(key) == Disposition::Pending {
        f.drain();
    }
    assert!(!f.host.fs.draft_files(true).is_empty());
}

fn relaunch_as(f: &mut Fixture, spellings: &[(&str, &str)]) {
    f.host.stop();
    f.host = binding(f);
    f.host.stop();
    let recovered = f.host.recovered_keys();
    for key in &recovered {
        let spelling = spellings
            .iter()
            .find(|(k, _)| k == key)
            .map_or(key.as_str(), |(_, s)| s);
        f.host
            .register(key.clone(), spelling, Arc::new(Mutex::new(())));
    }
    assert!(matches!(
        f.host.launch(),
        Disposition::Applied | Disposition::Pending
    ));
    f.drain();
}

/// STEP3 §2 / Q4, folding semantics. On a folding volume the store's
/// case-alias resolution spells a recovered `Foo.md` as its entry `foo.md`;
/// Linux cannot fold, so the test registers that spelling. The draft and the
/// custody marker name the old spelling and recover to the same key, and no
/// I/O goes through the old spelling: here that would create a second file.
#[test]
fn an_old_spelling_draft_and_custody_marker_recover_through_the_entry_spelling() {
    let mut f = Fixture::new();
    fs::write(f.graph.join("foo.md"), b"disk").unwrap();
    f.host = binding(&f);
    f.host
        .register("Foo.md".into(), "foo.md", Arc::new(Mutex::new(())));
    f.edit("Foo.md", "draft");
    assert_eq!(f.host.pages["Foo.md"].base, Base::Known(text("disk")));
    draft_after_failed_save(&mut f, "Foo.md");
    // A crash before the save: the draft names `Foo.md`.
    relaunch_as(&mut f, &[("Foo.md", "foo.md")]);
    let page = &f.host.pages["Foo.md"];
    assert!(!page.conflict && page.buf == text("draft"), "{page:?}");
    assert_eq!(f.save("Foo.md"), Outcome::Published);
    assert_eq!(fs::read(f.graph.join("foo.md")).unwrap(), b"draft");
    assert!(!f.graph.join("Foo.md").exists());
    // Delete through the move, then crash holding the custody marker.
    assert_eq!(f.host.delete("Foo.md"), Disposition::Pending);
    f.drain();
    assert_eq!(f.host.start_save("Foo.md"), Disposition::Pending);
    f.host.advance_save(0); // guard
    f.host.advance_save(0); // marker
    f.host.advance_save(0); // move
    assert!(!f.graph.join("foo.md").exists());
    assert_eq!(markers(&mut f), 1);
    relaunch_as(&mut f, &[("Foo.md", "foo.md")]);
    assert!(f.host.custody.is_empty());
    assert_eq!(markers(&mut f), 0);
    assert!(trash_holds(&f, b"draft"));
    assert_eq!(f.host.pages["Foo.md"].buf, None);
    assert_eq!(f.host.observe("Foo.md"), Disposition::Applied);
    assert_eq!(f.save("Foo.md"), Outcome::Published);
    assert!(!f.graph.join("foo.md").exists() && !f.graph.join("Foo.md").exists());
}

/// Q4, case-sensitive semantics: a case-only rename to an absent distinct
/// entry is a host rename between two keys with its operation custody. An
/// old-spelling draft does not recover a separate `Foo.md` after a crash.
#[test]
fn a_case_only_rename_to_a_distinct_entry_is_a_two_key_host_rename() {
    let mut f = Fixture::new();
    fs::write(f.graph.join("Foo.md"), b"disk").unwrap();
    f.host = binding(&f);
    for key in ["Foo.md", "foo.md"] {
        f.host.register(key.into(), key, Arc::new(Mutex::new(())));
    }
    f.edit("Foo.md", "edited");
    draft_after_failed_save(&mut f, "Foo.md");
    assert_eq!(f.save("Foo.md"), Outcome::Published);
    let identity = |bytes: &Text, _: &str, _: bool| Ok(bytes.clone());
    assert_eq!(
        f.host
            .rename_with("Foo.md", "foo.md", &BTreeSet::new(), identity),
        Disposition::Pending
    );
    f.drain();
    // A crash before either save of the operation.
    relaunch_as(&mut f, &[]);
    assert_eq!(f.host.pages["foo.md"].buf, text("edited"));
    assert_eq!(f.host.pages["Foo.md"].buf, None);
    assert_eq!(f.save("foo.md"), Outcome::Published);
    assert_eq!(f.save("Foo.md"), Outcome::Published);
    assert_eq!(fs::read(f.graph.join("foo.md")).unwrap(), b"edited");
    assert!(!f.graph.join("Foo.md").exists());
    assert!(trash_holds(&f, b"edited"));
}

/// STEP3 §2: a request queued before the alias spelling move applies to the
/// same page after it, and the page's I/O follows the new spelling.
#[test]
fn a_request_queued_before_a_spelling_move_applies_to_the_same_page_after_it() {
    let mut f = Fixture::new();
    f.send("a.md", RequestKind::Open);
    let version = f.host.pages["a.md"].version;
    let request = Request {
        id: f.host.last_admitted + 1,
        generation: f.host.generation,
        page: "a.md".into(),
        kind: RequestKind::Submit {
            bytes: text("typed"),
            version,
            resolve: None,
        },
    };
    assert_eq!(f.host.admit(request), Disposition::Applied);
    // The retained writer: reserve, move the entry, respell, release.
    let keys = BTreeSet::from(["a.md".to_string()]);
    assert_eq!(f.host.reserve(&keys), Disposition::Applied);
    fs::rename(f.graph.join("a.md"), f.graph.join("A.md")).unwrap();
    f.host.respell("a.md", "A.md", Arc::new(Mutex::new(())));
    assert_eq!(f.host.release(&keys), Disposition::Applied);
    let page = &f.host.pages["a.md"];
    assert!(!page.conflict && page.buf == text("A"), "{page:?}");
    assert_eq!(f.host.dequeue(), Disposition::Pending);
    assert_eq!(f.host.apply_request(), Disposition::Applied);
    assert_eq!(f.host.pages["a.md"].buf, text("typed"));
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(fs::read(f.graph.join("A.md")).unwrap(), b"typed");
    assert!(!f.graph.join("a.md").exists());
}
