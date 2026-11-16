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
    assert_model_aliases_refused(&[
        ("pages/foo.md", "pages/Foo.md"),
        ("pages/FOO.md", "pages/foo.md"),
    ]);
    // Negative control: on a distinct-case filesystem there is no alias.
    let mut fs = ModelFs::default();
    fs.external("pages/foo.md", bytes(OLD), true);
    assert_eq!(
        fs.page_owner("pages/Foo.md", &bytes(INPUT), None).unwrap(),
        None
    );
    assert_eq!(fs.read_page("pages/Foo.md").unwrap(), None);
}

#[test]
fn sb1_c8_model_unicode_case_aliases_refuse_collision() {
    assert_model_aliases_refused(&[
        ("pages/Kůň.md", "pages/kůň.md"),
        ("pages/KŮŇ.md", "pages/kůň.md"),
        ("pages/Σ.md", "pages/ς.md"),
    ]);
}

#[test]
fn sb1_c8_model_normalization_aliases_refuse_collision() {
    assert_model_aliases_refused(&[
        ("pages/Kůň.md", "pages/Ku\u{30a}n\u{30c}.md"),
        ("pages/Ku\u{30a}n\u{30c}.md", "pages/Kůň.md"),
    ]);
}

#[test]
fn sb1_c8_model_full_fold_aliases_refuse_collision() {
    assert_model_aliases_refused(&[
        ("pages/ß.md", "pages/SS.md"),
        ("pages/ﬃ.md", "pages/FFI.md"),
    ]);
}

#[test]
fn sb1_c8_model_turkish_i_uses_default_non_turkic_fold() {
    assert_model_aliases_refused(&[("pages/İ.md", "pages/i\u{307}.md")]);
    // Case models Unicode default full folding, not locale-specific Turkish
    // casing or NTFS's upcase table. Native tests probe each pair separately.
    for (existing, proposed) in [("I", "ı"), ("İ", "i")] {
        let mut fs = ModelFs::default();
        fs.env.fold = Fold::Case;
        fs.external(existing, bytes(OLD), true);
        assert_eq!(fs.case_alias(proposed), None);
    }
}

fn assert_model_aliases_refused(pairs: &[(&str, &str)]) {
    for &(existing, proposed) in pairs {
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

        let mut fs = ModelFs::default();
        fs.external(existing, bytes(OLD), true);
        assert_eq!(fs.case_alias(proposed), None);
        fs.page_temp(proposed, &bytes(INPUT)).unwrap();
        fs.page_rename(proposed).unwrap();
        fs.page_sync(proposed).unwrap();
        assert_eq!(fs.alpha(existing, true), bytes(OLD));
        assert_eq!(fs.alpha(proposed, true), bytes(INPUT));
    }
}

#[test]
fn sb1_c8_model_respelled_key_can_still_save_its_own_file() {
    for (before, after) in [
        ("Kůň", "kůň"),
        ("Kůň", "Ku\u{30a}n\u{30c}"),
        ("ß", "SS"),
        ("İ", "i\u{307}"),
    ] {
        let mut fs = ModelFs::default();
        fs.env.fold = Fold::Case;
        fs.spell("same-key", before);
        fs.external("same-key", bytes(OLD), true);
        fs.spell("same-key", after);
        assert_eq!(
            fs.page_owner("same-key", &bytes(INPUT), None).unwrap(),
            None
        );
        assert_eq!(fs.read_page("same-key").unwrap(), bytes(OLD));
        fs.page_temp("same-key", &bytes(INPUT)).unwrap();
        fs.page_rename("same-key").unwrap();
        fs.page_sync("same-key").unwrap();
        assert_eq!(fs.alpha("same-key", true), bytes(INPUT));
    }
}

/// These are probes of individual names in the directory being tested, not
/// an OS-wide guess about its collation table. In particular NTFS upcasing
/// cannot expand one UTF-16 character to `SS`; APFS default full folding can.
const SB1_C8_UNICODE_PAIRS: &[(&str, &str)] = &[
    ("Kůň", "kůň"),
    ("KŮŇ", "kůň"),
    ("Kůň", "Ku\u{30a}n\u{30c}"),
    ("ß", "SS"),
    ("İ", "i\u{307}"),
    ("I", "ı"),
    ("İ", "i"),
];

#[test]
fn sb1_c8_native_unicode_admission_and_noreplace_follow_volume() {
    for &(incumbent, proposed) in SB1_C8_UNICODE_PAIRS {
        let dir = native_tempdir();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let from = format!("pages/{incumbent}.md");
        let to = format!("pages/{proposed}.md");
        let old = dir.path().join(&from);
        let new = dir.path().join(&to);
        // Give the incumbent a different logical title: on a sensitive
        // volume this tests physical admission independently of name twins.
        let incumbent_bytes = b"title:: Incumbent\n- original\n";
        fs::write(&old, incumbent_bytes).unwrap();
        let aliases = match same_file::is_same_file(&old, &new) {
            Ok(same) => same,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => panic!("volume probe {incumbent:?}/{proposed:?}: {error}"),
        };
        eprintln!("SB1_C8_VOLUME {incumbent:?}/{proposed:?}: alias={aliases}");
        if cfg!(any(windows, target_os = "macos")) && proposed == "kůň" {
            assert!(
                aliases,
                "hosted NTFS/default APFS must execute Unicode case collision coverage"
            );
        }
        if cfg!(target_os = "macos") && incumbent == "Kůň" && proposed != "kůň" {
            assert!(
                aliases,
                "hosted APFS must execute normalization collision coverage"
            );
        }
        if cfg!(windows) && incumbent == "ß" {
            assert!(
                !aliases,
                "NTFS upcasing does not perform the full ß/SS fold"
            );
        }
        if cfg!(target_os = "macos") && incumbent == "ß" {
            assert!(
                aliases,
                "hosted default APFS must execute full-fold collision coverage"
            );
        }
        let store = crate::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        store.whole_graph_reconciled().unwrap();
        let id = crate::FileId::from(to.clone());
        let admission = store.page_plan(None).page_create_checks(&id, proposed);
        if aliases {
            assert!(
                matches!(
                    admission,
                    Err(crate::Why::Conflict { .. })
                        | Err(crate::Why::Refused(crate::Refusal::Twin { .. }))
                ),
                "{incumbent:?}/{proposed:?}: {admission:?}"
            );
            assert_eq!(
                crate::atomic_file::atomic_write_new(&new, INPUT)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::AlreadyExists
            );
            let incoming = dir.path().join("pages/incoming.md");
            fs::write(&incoming, INPUT).unwrap();
            let rename_admission = store.page_plan(None).admission_slot(
                &id,
                None,
                Some(&crate::FileId::from("pages/incoming.md".to_owned())),
            );
            assert!(
                matches!(
                    rename_admission,
                    Err(crate::Why::Conflict { .. })
                        | Err(crate::Why::Refused(crate::Refusal::Twin { .. }))
                ),
                "rename onto {incumbent:?}/{proposed:?}: {rename_admission:?}"
            );
            assert_eq!(
                crate::no_replace::move_file_noreplace(&incoming, &new)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::AlreadyExists
            );
            assert_eq!(fs::read(&incoming).unwrap(), INPUT);
            // Android/NFS flag-refusal policy: even on an unknown volume,
            // fstatat of the actual target must see its own Unicode aliases.
            #[cfg(unix)]
            {
                struct Reset;
                impl Drop for Reset {
                    fn drop(&mut self) {
                        crate::no_replace::REFUSE_NOREPLACE_FLAG.with(|flag| flag.set(false));
                    }
                }
                crate::no_replace::REFUSE_NOREPLACE_FLAG.with(|flag| flag.set(true));
                let _reset = Reset;
                assert_eq!(
                    crate::no_replace::move_file_noreplace(&incoming, &new)
                        .unwrap_err()
                        .kind(),
                    std::io::ErrorKind::AlreadyExists
                );
                assert_eq!(fs::read(&incoming).unwrap(), INPUT);
            }
        } else {
            assert!(
                admission.is_ok(),
                "distinct physical slot {incumbent:?}/{proposed:?}: {admission:?}"
            );
            crate::atomic_file::atomic_write_new(&new, INPUT).unwrap();
            assert_eq!(fs::read(&new).unwrap(), INPUT);
            // Files that coexist must retain separate identities and remain
            // editable, even if their logical name folds happen to collide.
            let first = store.page(&crate::PageId::from(from.clone())).unwrap();
            let second = store.page(&crate::PageId::from(to.clone())).unwrap();
            assert_ne!(first.id, second.id);
            let mut first_doc = first.doc;
            let mut second_doc = second.doc;
            first_doc.blocks[0].raw = "first saved independently".into();
            second_doc.blocks[0].raw = "second saved independently".into();
            let mut tx = store.page_plan(Some(crate::EditKind::ReplacePage));
            tx.save_page(
                &[crate::EditKind::ReplacePage],
                &first.id,
                crate::SaveBase::Existing(first.rev),
                &first_doc,
            );
            tx.save_page(
                &[crate::EditKind::ReplacePage],
                &second.id,
                crate::SaveBase::Existing(second.rev),
                &second_doc,
            );
            assert!(matches!(
                tx.commit_internal(),
                crate::TxOutcome::Committed { .. }
            ));
            let saved_first = fs::read_to_string(&old).unwrap();
            let saved_second = fs::read_to_string(&new).unwrap();
            assert!(saved_first.contains("first saved independently"));
            assert!(!saved_first.contains("second saved independently"));
            assert!(saved_second.contains("second saved independently"));
            assert!(!saved_second.contains("first saved independently"));
        }
        if aliases {
            assert_eq!(fs::read(&old).unwrap(), incumbent_bytes);
        }
        store.close();
    }
}

#[test]
fn sb1_c8_native_unicode_self_alias_rename_keeps_bytes() {
    let mut executed = 0;
    for &(before, after) in SB1_C8_UNICODE_PAIRS {
        let dir = native_tempdir();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let from = crate::FileId::from(format!("pages/{before}.md"));
        let to = crate::FileId::from(format!("pages/{after}.md"));
        let src = dir.path().join(from.as_str());
        let dst = dir.path().join(to.as_str());
        fs::write(&src, OLD).unwrap();
        let aliases = match same_file::is_same_file(&src, &dst) {
            Ok(same) => same,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => panic!("volume probe: {error}"),
        };
        if !aliases {
            eprintln!("SB1_C8_SELF_ALIAS_NOT_PRESENT {before:?}/{after:?}");
            continue;
        }
        executed += 1;
        let store = crate::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        {
            let _writer = store.writer.lock().unwrap();
            let tx = store.page_plan(None);
            tx.admission_slot(&to, None, Some(&from)).unwrap();
            let (moved, result) = tx.respell_entry(&from, &to, OLD);
            assert!(
                result.is_ok() && moved,
                "{before:?}/{after:?}: {result:?}, moved={moved}"
            );
        }
        assert_eq!(fs::read(&dst).unwrap(), OLD);
        let entries: Vec<_> = fs::read_dir(dir.path().join("pages"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(entries.contains(&std::ffi::OsString::from(format!("{after}.md"))));
        assert!(!entries.contains(&std::ffi::OsString::from(format!("{before}.md"))));
        store.close();
    }
    assert!(
        executed > 0 || !cfg!(any(windows, target_os = "macos")),
        "Windows/macOS must execute native Unicode self-alias renames"
    );
    if executed == 0 {
        eprintln!("NATIVE_MODELFS_ONLY SB1-C8 self-alias: actual volume is case- and normalization-sensitive");
    }
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
        // NTFS and APFS fold case by default, as do the hosted runners: a
        // skip there would leave C8 untested where it matters.
        assert!(
            !cfg!(any(windows, target_os = "macos")),
            "C8 expects a case-insensitive temp volume on Windows and macOS"
        );
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
