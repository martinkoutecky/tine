//! C5/C7/C8 and C6 withdraw power witnesses. Hand-written bounded schedules;
//! no directory model or native power-loss experiment is claimed here.
use super::*;
use io::Phase;
use model_fs::{EnvStep, Fold, ModelFs};
use production::ProductionIo;
use std::{fs, io::Write, path::Path, sync::Barrier, thread};

const PAGE: &str = "pages/a.md";
const OLD: &[u8] = b"- original\n";
const INPUT: &[u8] = b"- admitted input\n";
const EXTERNAL: &[u8] = b"- external complete version\n";
const KEYS: [&str; 3] = [PAGE, "pages/new.md", "pages/changed.md"];

fn native_tempdir() -> tempfile::TempDir {
    match std::env::var_os("TINE_HOST_FS_ROOT") {
        Some(base) => tempfile::tempdir_in(base).unwrap(),
        None => tempfile::tempdir().unwrap(),
    }
}

fn bytes(b: &[u8]) -> Text {
    Some(Arc::from(b))
}

fn host<F: HostIo>(fs: F, keys: &[&str]) -> Host<F> {
    Host::new(
        fs,
        keys.iter()
            .map(|k| ((*k).into(), Arc::new(Mutex::new(()))))
            .collect(),
    )
}

fn send<F: HostIo>(h: &mut Host<F>, page: &str, kind: RequestKind) {
    assert_eq!(
        h.admit(Request {
            id: h.last_admitted + 1,
            generation: h.generation,
            page: page.into(),
            kind
        }),
        Disposition::Applied
    );
    assert_eq!(h.dequeue(), Disposition::Pending);
    assert_eq!(h.apply_request(), Disposition::Applied);
}

fn edit<F: HostIo>(h: &mut Host<F>, key: &str) {
    send(h, key, RequestKind::Open);
    let version = h.pages[key].version;
    send(
        h,
        key,
        RequestKind::Submit {
            bytes: bytes(INPUT),
            version,
            resolve: None,
        },
    );
}

fn draft<F: HostIo>(h: &mut Host<F>, key: &str) {
    assert_eq!(
        h.begin_draft(key),
        Disposition::Pending,
        "draft {key}: {:?}",
        h.pages[key]
    );
    for _ in 0..20 {
        if h.worker.is_none() {
            break;
        }
        h.advance_draft();
    }
    assert!(h.worker.is_none());
    assert_eq!(
        drafts::scan(h.fs.draft_files(true)).logical[key].bytes,
        bytes(INPUT)
    );
}

fn save<F: HostIo>(h: &mut Host<F>, key: &str) {
    assert_eq!(h.start_save(key), Disposition::Pending);
    for _ in 0..15 {
        if h.job.is_none() {
            return;
        }
        h.advance_save();
    }
    panic!("save did not finish");
}

fn launch<F: HostIo>(fs: F, keys: &[&str]) -> Host<F> {
    let mut h = host(fs, keys);
    h.stop();
    assert_eq!(h.launch(), Disposition::Applied);
    h
}

fn recovered<F: HostIo>(h: &mut Host<F>) {
    assert_eq!(h.pages.len(), KEYS.len());
    for key in KEYS {
        assert_eq!(h.pages[key].buf, bytes(INPUT));
        assert!(h.pages[key].risk);
        save(h, key);
        if key == KEYS[2] {
            assert!(h.pages[key].conflict);
            assert_eq!(h.fs.read_page(key).unwrap(), bytes(EXTERNAL));
        } else {
            assert!(h.pages[key].clean());
            assert_eq!(h.fs.read_page(key).unwrap(), bytes(INPUT));
        }
    }
}

#[test]
fn m4_c5_model_relaunch_at_new_path_surfaces_then_recovers_every_draft() {
    let mut fs = ModelFs::default();
    fs.external(PAGE, bytes(OLD), true);
    fs.external(KEYS[2], bytes(OLD), true);
    let mut old = host(fs, &KEYS);
    for key in KEYS {
        edit(&mut old, key);
        old.fs.inject(Phase::PageTemp, [model_fs::Fault::Before]);
        save(&mut old, key);
        draft(&mut old, key);
    }
    old.stop();
    old.fs.crash();
    // The test-side namespace boundary is explicit: a different root path
    // selects an empty app-data namespace, not an automatic draft merge.
    let orphan = old.fs.draft_files(false);
    assert_eq!(drafts::scan(orphan.clone()).logical.len(), KEYS.len());
    let mut moved = old.fs.clone();
    moved.files.retain(|k, _| !k.starts_with("draft/"));
    moved.stable.retain(|k, _| !k.starts_with("draft/"));
    moved.external(KEYS[2], bytes(EXTERNAL), true);
    let mut new = launch(moved, &KEYS);
    assert!(new.pages.is_empty());
    // Explicit adoption transfers the whole old namespace. Production app
    // census/adoption is tested separately by root_identity's M4 companion.
    for (name, data) in orphan {
        new.fs
            .files
            .insert(format!("draft/{name}"), Arc::from(data));
    }
    new.stop();
    assert_eq!(new.launch(), Disposition::Applied);
    recovered(&mut new);
    new.fs.assert_no_escapes();
}

fn physical_drafts(path: &Path) -> Vec<(String, Vec<u8>)> {
    fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_type().unwrap().is_file())
        .map(|e| {
            (
                e.file_name().to_str().unwrap().into(),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn m4_c5_native_moved_root_keeps_physical_orphans_and_recovers_after_adoption() {
    let dir = native_tempdir();
    let old_root = dir.path().join("old-root");
    let moved = dir.path().join("moved-root");
    let app = dir.path().join("app");
    fs::create_dir_all(old_root.join("pages")).unwrap();
    for key in [PAGE, KEYS[2]] {
        fs::write(old_root.join(key), OLD).unwrap();
    }
    let io = ProductionIo::attach(
        &old_root,
        &app,
        "old-path",
        &old_root.join("logseq/.tine-trash/pages"),
    );
    let mut old = host(io, &KEYS);
    for key in KEYS {
        edit(&mut old, key);
        old.fs
            .faults
            .entry(Phase::PageTemp)
            .or_default()
            .push_back(std::io::ErrorKind::Other);
        save(&mut old, key);
        draft(&mut old, key);
    }
    let before = physical_drafts(&app.join("drafts-v2/old-path"));
    assert_eq!(drafts::scan(before.clone()).logical.len(), KEYS.len());
    // Drop without a saved-stop: the already durable vehicles are the only
    // recovery input. Move the real graph directory between incarnations.
    drop(old);
    fs::rename(&old_root, &moved).unwrap();
    fs::write(moved.join(KEYS[2]), EXTERNAL).unwrap();
    let new_io = ProductionIo::attach(
        &moved,
        &app,
        "new-path",
        &moved.join("logseq/.tine-trash/pages"),
    );
    let new = launch(new_io, &KEYS);
    assert!(new.pages.is_empty());
    assert_eq!(physical_drafts(&app.join("drafts-v2/old-path")), before);
    drop(new);
    // Adapter-side half of explicit adoption; actual census/refusal/rename
    // policy runs in the app companion, not a mock production guard.
    let target = app.join("drafts-v2/new-path");
    for child in ["unreadable", "trash-custody"] {
        let p = target.join(child);
        if p.exists() {
            fs::remove_dir(p).unwrap();
        }
    }
    if target.exists() {
        fs::remove_dir(&target).unwrap();
    }
    fs::rename(app.join("drafts-v2/old-path"), &target).unwrap();
    crate::directory_durability::sync_private_directory(target.parent().unwrap()).unwrap();
    let io = ProductionIo::attach(
        &moved,
        &app,
        "new-path",
        &moved.join("logseq/.tine-trash/pages"),
    );
    let mut new = launch(io, &KEYS);
    recovered(&mut new);
    assert!(!old_root.exists());
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no sibling escape"
    );
}

#[test]
fn m4_c7_model_partial_external_reads_conflict_without_publishing() {
    for prefix in [0, 1, EXTERNAL.len() - 1] {
        for at_open in [false, true] {
            let mut fs = ModelFs::default();
            fs.external(PAGE, bytes(OLD), true);
            let mut h = host(fs, &[PAGE]);
            if at_open {
                h.fs.inject_env(
                    Phase::Read,
                    0,
                    EnvStep::ExternalInPlace(PAGE.into(), Arc::from(EXTERNAL), prefix),
                );
            }
            edit(&mut h, PAGE);
            if at_open {
                assert_eq!(h.pages[PAGE].base, Base::Known(bytes(&EXTERNAL[..prefix])));
                h.fs.external(PAGE, bytes(EXTERNAL), true);
            } else {
                h.fs.inject_env(
                    Phase::Read,
                    0,
                    EnvStep::ExternalInPlace(PAGE.into(), Arc::from(EXTERNAL), prefix),
                );
            }
            save(&mut h, PAGE);
            assert!(h.pages[PAGE].conflict);
            if !at_open {
                assert_eq!(h.pages[PAGE].obs, Some(bytes(&EXTERNAL[..prefix])));
            }
            assert_eq!(h.pages[PAGE].buf, bytes(INPUT));
            assert!(!h.fs.calls.contains(&Phase::PageRename));
            if !at_open {
                h.fs.external(PAGE, bytes(EXTERNAL), true);
            }
            assert_eq!(h.fs.alpha(PAGE, false), bytes(EXTERNAL));
            draft(&mut h, PAGE);
            h.fs.assert_no_escapes();
        }
    }
}

/// The hook runs after the reader opens its handle and stats it, before its
/// bounded read. The other thread truncates/writes a prefix and waits until
/// that read completes before writing the rest. No sleeps or polling.
fn partial_writer(path: &Path, prefix: usize) -> (Arc<Barrier>, thread::JoinHandle<()>) {
    let begin = Arc::new(Barrier::new(2));
    let exposed = Arc::new(Barrier::new(2));
    let finish = Arc::new(Barrier::new(2));
    let path = path.to_owned();
    let (b, e, f) = (begin.clone(), exposed.clone(), finish.clone());
    let writer = thread::spawn(move || {
        b.wait();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .unwrap();
        file.write_all(&EXTERNAL[..prefix]).unwrap();
        e.wait();
        f.wait();
        file.write_all(&EXTERNAL[prefix..]).unwrap();
        file.sync_all().unwrap();
    });
    crate::model::READ_PARSE_BYTES_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            begin.wait();
            exposed.wait();
        }));
    });
    (finish, writer)
}

#[test]
fn m4_c7_native_in_place_writer_during_read_conflicts_and_keeps_input() {
    for prefix in [0, 1, EXTERNAL.len() - 1] {
        for at_open in [false, true] {
            let dir = native_tempdir();
            let root = dir.path().join("graph");
            let outside = dir.path().join("outside");
            fs::create_dir_all(root.join("pages")).unwrap();
            fs::create_dir(&outside).unwrap();
            let path = root.join(PAGE);
            fs::write(&path, OLD).unwrap();
            let io = ProductionIo::attach(
                &root,
                &dir.path().join("app"),
                "c7",
                &root.join("logseq/.tine-trash/pages"),
            );
            let mut h = host(io, &[PAGE]);
            if !at_open {
                edit(&mut h, PAGE);
            }
            let (finish, writer) = partial_writer(&path, prefix);
            if at_open {
                edit(&mut h, PAGE);
                assert_eq!(h.pages[PAGE].base, Base::Known(bytes(&EXTERNAL[..prefix])));
            } else {
                assert_eq!(h.start_save(PAGE), Disposition::Pending);
                assert_eq!(h.advance_save(), Disposition::Pending); // temp
                assert_eq!(h.advance_save(), Disposition::Applied); // torn check
                assert!(h.pages[PAGE].conflict);
                assert_eq!(h.pages[PAGE].obs, Some(bytes(&EXTERNAL[..prefix])));
            }
            // Release even if a later assertion fails: no stranded writer.
            finish.wait();
            writer.join().unwrap();
            if at_open {
                save(&mut h, PAGE);
            }
            assert!(h.pages[PAGE].conflict);
            assert_eq!(h.pages[PAGE].buf, bytes(INPUT));
            assert_eq!(fs::read(&path).unwrap(), EXTERNAL);
            assert!(!h
                .events
                .iter()
                .any(|e| matches!(e, Event::Renamed { .. } | Event::Published { .. })));
            draft(&mut h, PAGE);
            assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
        }
    }
}

#[test]
fn m4_c8_model_case_alias_is_seen_by_admission_and_never_overwritten() {
    for (existing, proposed) in [
        ("pages/foo.md", "pages/Foo.md"),
        ("pages/FOO.md", "pages/foo.md"),
    ] {
        let mut fs = ModelFs::default();
        fs.env.fold = Fold::Case;
        fs.external(existing, bytes(OLD), true);
        // Logical keys may carry registered physical spellings too.
        fs.spell("creating-key", proposed);
        assert_eq!(
            fs.page_owner("creating-key", &bytes(INPUT), None).unwrap(),
            Some(existing.into())
        );
        assert_eq!(fs.read_page("creating-key").unwrap(), bytes(OLD));
        fs.page_temp("creating-key", &bytes(INPUT)).unwrap();
        assert_eq!(
            fs.page_rename("creating-key").unwrap_err().kind,
            ErrorKind::Collision
        );
        assert_eq!(fs.alpha(existing, false), bytes(OLD));
        assert_eq!(fs.alpha(existing, true), bytes(OLD));
        assert!(!fs.files.contains_key("graph/creating-key"));
        fs.assert_no_escapes();

        // A claimant arriving after an absent base was admitted must also
        // trip the host's ordinary check, without reaching publication.
        let mut fs = ModelFs::default();
        fs.env.fold = Fold::Case;
        fs.spell("creating-key", proposed);
        let mut h = host(fs, &["creating-key"]);
        edit(&mut h, "creating-key");
        h.fs.external(existing, bytes(OLD), true);
        save(&mut h, "creating-key");
        assert!(h.pages["creating-key"].conflict);
        assert_eq!(h.fs.alpha(existing, false), bytes(OLD));
        assert!(!h.fs.calls.contains(&Phase::PageRename));
        draft(&mut h, "creating-key");
        h.fs.assert_no_escapes();
    }
    // Negative control: on a distinct-case filesystem there is no alias.
    let mut fs = ModelFs::default();
    fs.external("pages/foo.md", bytes(OLD), true);
    assert_eq!(
        fs.page_owner("pages/Foo.md", &bytes(INPUT), None).unwrap(),
        None
    );
    assert_eq!(fs.read_page("pages/Foo.md").unwrap(), None);
}

const NATIVE_MODELFS_ONLY: &[(&str, &str)] = &[
    (
        "C8",
        "case-sensitive volumes (including Linux ext4): runtime probe skips native alias claim",
    ),
    (
        "C6-power",
        "native power cuts unavailable; directory-sync and atomic-move API witnesses only",
    ),
];

#[test]
fn m4_c8_native_case_insensitive_volume_admission_refuses_collision() {
    let dir = native_tempdir();
    fs::create_dir(dir.path().join("pages")).unwrap();
    let lower = dir.path().join("pages/foo.md");
    let upper = dir.path().join("pages/Foo.md");
    fs::write(&lower, OLD).unwrap();
    if !same_file::is_same_file(&lower, &upper).unwrap_or(false) {
        eprintln!("NATIVE_MODELFS_ONLY C8: {}", NATIVE_MODELFS_ONLY[0].1);
        return;
    }
    let store = crate::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    store.whole_graph_reconciled().unwrap();
    let result = store
        .page_plan(None)
        .page_create_checks(&crate::FileId::from("pages/Foo.md".to_owned()), "Foo");
    assert!(
        matches!(
            result,
            Err(crate::Why::Conflict { .. })
                | Err(crate::Why::Refused(crate::Refusal::Twin { .. }))
        ),
        "admission did not diagnose the occupied folded slot: {result:?}"
    );
    // The same create-new primitive used by PreparedWrite::publish(true)
    // must refuse the folded physical slot, independently of admission.
    assert_eq!(
        crate::atomic_file::atomic_write_new(&upper, INPUT)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(lower).unwrap(), OLD);
    store.close();
}

#[test]
fn m4_c6_withdraw_model_power_cuts_keep_page_or_conflict_payload() {
    let mut cases = 0;
    for fresh in [false, true] {
        let mut fs = ModelFs::default();
        fs.external(PAGE, bytes(OLD), true);
        if !fresh {
            // A prior withdrawal left a durable trash chain.
            fs.external("prior.md", bytes(OLD), true);
            fs.trash_move("prior.md", "prior").result.unwrap();
            fs.trash_sync("prior.md", "prior").unwrap();
            fs.page_sync("prior.md").unwrap();
        }
        // Same primitives as withdraw: durable bounded chain, then atomic
        // no-replace move, with no post-move directory sync. The conflict
        // directory differs only in spelling from ModelFs's pages chain.
        fs.trash_move(PAGE, "conflict").result.unwrap();
        assert_eq!(fs.alpha(PAGE, false), None);
        assert_eq!(fs.files[&format!("trash/{PAGE}/conflict")].as_ref(), OLD);
        for keep in [BTreeSet::new(), BTreeSet::from([PAGE.into()])] {
            for cut in fs.cuts(&keep) {
                let mut after = fs.clone();
                after.power_cut(&keep, false, &cut);
                let page = after.alpha(PAGE, false);
                let trash = after.files.get(&format!("trash/{PAGE}/conflict")).cloned();
                assert!(
                    page == bytes(OLD) || trash == bytes(OLD),
                    "lost admitted durable bytes"
                );
                if page.is_none() {
                    assert_eq!(trash, bytes(OLD));
                }
                after.assert_no_escapes();
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 6); // optional surplus destination when old source survives
}

#[test]
fn m4_c6_native_withdraw_witnesses_durable_chain_and_same_volume_move() {
    for fresh in [false, true] {
        let dir = native_tempdir();
        // Match Store's canonical spelling: Windows verbatim paths and
        // macOS's /var alias otherwise fail the private containment check.
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir(root.join("pages")).unwrap();
        let source = root.join(PAGE);
        fs::write(&source, OLD).unwrap();
        let durable_file = fs::OpenOptions::new().write(true).open(&source).unwrap();
        durable_file.sync_all().unwrap();
        drop(durable_file);
        crate::directory_durability::sync_directory_entry(source.parent().unwrap()).unwrap();
        let store = crate::Store::open(&root, Default::default()).unwrap().0;
        let trash = root.join("logseq/.tine-trash/conflicts");
        if !fresh {
            fs::write(root.join("pages/prior.md"), OLD).unwrap();
            store
                .graph
                .transaction_withdraw_exact(&root.join("pages/prior.md"), OLD, "m4-prior")
                .unwrap();
        }
        crate::directory_durability::take_synced_directories();
        let result = store
            .graph
            .transaction_withdraw_exact(&source, OLD, "m4-power")
            .unwrap();
        let crate::model::Withdrawal::Exact(payload) = result else {
            panic!("withdraw did not keep exact bytes")
        };
        assert_eq!(payload.parent(), Some(trash.as_path()));
        assert_eq!(fs::read(&payload).unwrap(), OLD);
        assert!(!source.exists());
        let syncs = crate::directory_durability::take_synced_directories();
        if fresh {
            for parent in [
                root.clone(),
                root.join("logseq"),
                root.join("logseq/.tine-trash"),
            ] {
                assert!(
                    syncs.contains(&parent),
                    "new conflict chain omitted parent sync: {syncs:?}"
                );
            }
        }
        // Atomic no-replace is the single-move recipe; there is no claim of
        // an OS power cut here. ModelFs enumerates old-source/new-trash pairs.
        assert!(NATIVE_MODELFS_ONLY
            .iter()
            .any(|(family, _)| *family == "C6-power"));
        store.close();
    }
}
