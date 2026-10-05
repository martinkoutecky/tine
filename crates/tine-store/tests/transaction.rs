use std::fs::{self, File};
use std::io::Write;
#[cfg(feature = "test-faults")]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::{
    Area, Content, FileId, FileRev, OpenOptions, PageId, Refusal, RenameMap, SaveBase, StepResult,
    Store, TxOutcome, WatchMode, Why,
};

struct Fixture {
    root: PathBuf,
    store: Arc<Store>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_watch(WatchMode::Notify, &[])
    }

    fn with_watch(watch: WatchMode, files: &[(&str, &[u8])]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-transaction-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for dir in ["pages", "journals", "assets", "logseq"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        for (rel, bytes) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let store = Arc::new(
            Store::open(
                &root,
                OpenOptions {
                    watch,
                    ..Default::default()
                },
            )
            .unwrap()
            .0,
        );
        store.whole_graph().unwrap();
        Self { root, store }
    }

    fn id(&self, area: Area, rel: &str) -> FileId {
        self.store.file_id(area, rel).unwrap()
    }

    fn put(&self, rel: &str, bytes: &[u8]) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let temp = self.root.join(format!(
            ".transaction-fixture-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&temp, bytes).unwrap();
        fs::rename(temp, path).unwrap();
    }

    fn bytes(&self, rel: &str) -> Option<Vec<u8>> {
        fs::read(self.root.join(rel)).ok()
    }

    fn rev(&self, file: &FileId) -> FileRev {
        self.store.read(file, None).unwrap().1
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn doc(name: &str, raw: &str) -> PageDto {
    PageDto {
        name: name.into(),
        kind: PageKind::Page,
        title: name.into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "tx-block".into(),
            raw: raw.into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,

        guide: false,
    }
}

fn committed(outcome: TxOutcome) -> Vec<StepResult> {
    match outcome {
        TxOutcome::Committed { steps, .. } => steps,
        other => panic!("expected commit: {other:?}"),
    }
}

fn refused(outcome: TxOutcome) -> (Why, tine_store::Rollback) {
    match outcome {
        TxOutcome::NotCommitted { why, rollback, .. } => (why, rollback),
        other => panic!("expected refusal: {other:?}"),
    }
}

#[test]
fn read_only_expectation_passes_without_touching_file() {
    let f = Fixture::with_watch(
        WatchMode::Notify,
        &[("assets/sidecar.edn", b"{:highlights []}")],
    );
    let sidecar = f.id(Area::Assets, "sidecar.edn");
    let before = fs::metadata(f.root.join("assets/sidecar.edn"))
        .unwrap()
        .modified()
        .unwrap();
    let mut tx = f.store.transaction(None);
    tx.expect(&sidecar, f.rev(&sidecar));
    assert!(matches!(
        committed(tx.commit()).as_slice(),
        [StepResult::Unchanged { file, .. }] if file == &sidecar
    ));
    assert_eq!(f.bytes("assets/sidecar.edn").unwrap(), b"{:highlights []}");
    assert_eq!(
        fs::metadata(f.root.join("assets/sidecar.edn"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
}

#[test]
fn missing_read_only_expectation_conflicts() {
    let f = Fixture::with_watch(
        WatchMode::Notify,
        &[("assets/missing.edn", b"former bytes")],
    );
    let missing = f.id(Area::Assets, "missing.edn");
    let expected = f.rev(&missing);
    fs::remove_file(f.root.join("assets/missing.edn")).unwrap();
    let mut tx = f.store.transaction(None);
    tx.expect(&missing, expected);
    assert!(matches!(
        tx.commit(),
        TxOutcome::NotCommitted {
            step: 0,
            why: Why::Conflict { file, disk: None },
            ..
        } if file == missing
    ));
    assert!(f.bytes("assets/missing.edn").is_none());
}

#[test]
fn read_only_expectation_conflict_undoes_an_earlier_step() {
    use tine_store::FaultPoint;

    let f = Fixture::with_watch(WatchMode::Notify, &[("assets/sidecar.edn", b"original")]);
    let sidecar = f.id(Area::Assets, "sidecar.edn");
    let created = f.id(Area::Assets, "created.bin");
    let mut tx = f.store.transaction(None);
    tx.create(&created, Content::Bytes(b"created".to_vec()))
        .expect(&sidecar, f.rev(&sidecar));
    f.store.inject_fault(FaultPoint::Stage2MismatchAt(1));
    let outcome = tx.commit();
    assert!(matches!(
        outcome,
        TxOutcome::NotCommitted {
            step: 1,
            why: Why::Conflict { file, .. },
            ..
        } if file == sidecar
    ));
    assert!(f.bytes("assets/created.bin").is_none());
    assert_eq!(f.bytes("assets/sidecar.edn").unwrap(), b"external stage-2");
}

#[cfg(all(feature = "test-faults", unix))]
#[test]
fn move_reports_directory_sync_failure_after_rename() {
    use tine_store::FaultPoint;

    let f = Fixture::with_watch(WatchMode::Notify, &[("assets/source.bin", b"source")]);
    let source = f.id(Area::Assets, "source.bin");
    let destination = f.id(Area::Assets, "destination.bin");
    let revision = f.rev(&source);
    f.store.inject_fault(FaultPoint::DirectorySyncIo);
    let mut tx = f.store.transaction(None);
    tx.move_file(&source, revision, &destination, None);
    let outcome = tx.commit();
    assert!(
        matches!(outcome, TxOutcome::NotCommitted { why: Why::Failed(_), .. }),
        "directory sync failure after rename must be reported by Transaction::commit; exemplar sync_move_dirs: {outcome:?}"
    );
}

#[cfg(all(feature = "test-faults", unix))]
#[test]
fn create_undoes_publication_when_directory_sync_fails() {
    use tine_store::FaultPoint;

    let f = Fixture::new();
    let created = f.id(Area::Assets, "created.bin");
    f.store.inject_fault(FaultPoint::DirectorySyncIo);
    let mut tx = f.store.transaction(None);
    tx.create(&created, Content::Bytes(b"created".to_vec()));
    let outcome = tx.commit();
    assert!(
        matches!(outcome, TxOutcome::NotCommitted { why: Why::Failed(_), .. }),
        "directory sync failure after create must be reported by Transaction::commit; exemplar atomic_write_new: {outcome:?}"
    );
    assert_eq!(f.bytes("assets/created.bin"), None);
}

#[test]
fn step_successes_and_noop() {
    let f = Fixture::new();
    f.put("pages/A.md", b"- before\n");
    f.put("pages/B.md", b"- [[A]]\n");
    f.put("assets/meta.edn", b"old");
    f.put("assets/delete.bin", b"delete me");
    let a = PageId::from("pages/A.md");
    let b = f.id(Area::Pages, "B.md");
    let moved = f.id(Area::Pages, "C.md");
    let meta = f.id(Area::Assets, "meta.edn");
    let created = f.id(Area::Assets, "new.bin");
    let delete = f.id(Area::Assets, "delete.bin");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.save_page(
        &[tine_store::EditKind::ReplacePage],
        &a,
        SaveBase::Existing(f.rev(&a.file())),
        &doc("A", "after"),
    );
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Written { .. }
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.save_page(
        &[tine_store::EditKind::ReplacePage],
        &a,
        SaveBase::Existing(f.rev(&a.file())),
        &doc("A", "after"),
    );
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Unchanged { .. }
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create(&created, Content::Bytes(b"new".to_vec()));
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Written { .. }
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.replace(&meta, f.rev(&meta), b"replacement".to_vec());
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Written { .. }
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.rewrite_refs(
        &PageId::from("pages/B.md"),
        f.rev(&b),
        &RenameMap(vec![("A".into(), "C".into())]),
    );
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Written { .. }
    ));
    assert_eq!(f.bytes("pages/B.md").unwrap(), b"- [[C]]\n");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.move_file(&b, f.rev(&b), &moved, None);
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Moved { .. }
    ));
    assert!(f.bytes("pages/B.md").is_none());
    assert_eq!(f.bytes("pages/C.md").unwrap(), b"- [[C]]\n");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.trash(&delete, f.rev(&delete), tine_store::TrashIf::Any);
    match &committed(tx.commit())[0] {
        StepResult::Trashed { trashed, .. } => {
            assert!(trashed.as_str().starts_with("logseq/.tine-trash/assets/"));
            assert_eq!(f.bytes(trashed.as_str()).unwrap(), b"delete me");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn preflight_refusals_leave_disk_and_rollback_empty() {
    let f = Fixture::new();
    f.put("pages/A.md", b"- a\n");
    f.put("pages/A.org", b"* twin\n");
    f.put("assets/x.bin", b"x");
    let a = f.id(Area::Pages, "A.md");
    let x = f.id(Area::Assets, "x.bin");
    let y = f.id(Area::Assets, "y.bin");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create(&a, Content::Bytes(b"- new\n".to_vec()));
    let (why, rb) = refused(tx.commit());
    assert!(matches!(why, Why::Conflict { .. }));
    assert!(rb.kept_external.is_empty() && rb.undo_failed.is_empty());
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create(&f.id(Area::Pages, "B.md"), Content::Bytes(vec![0xff]));
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::Undecodable)
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.replace(&a, f.rev(&a), b"bad".to_vec());
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.move_file(&x, f.rev(&x), &f.id(Area::Trash, "assets/illegal"), None);
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create_unique(Area::Assets, "../bad", ".png", Content::Bytes(vec![1]));
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    assert!(f.store.file_id(Area::Meta, ".tine-trash/forged").is_err());
    f.put("logseq/config.edn", b"{}\n");
    let config = f.id(Area::Meta, "config.edn");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.replace(
        &config,
        f.rev(&config),
        b"{:preferred-format :org}\n".to_vec(),
    );
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Written { .. }
    ));
    assert_eq!(
        f.bytes("logseq/config.edn").unwrap(),
        b"{:preferred-format :org}\n"
    );
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.replace(
        &config,
        f.rev(&config),
        b"{:pages-directory \"../outside\"}\n".to_vec(),
    );
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    assert_eq!(
        f.bytes("logseq/config.edn").unwrap(),
        b"{:preferred-format :org}\n"
    );
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.trash(&config, f.rev(&config), tine_store::TrashIf::Any);
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.replace(&x, f.rev(&x), b"new".to_vec())
        .create(&x, Content::Bytes(b"again".to_vec()));
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::RepeatedFile(_))
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create_unique(
        Area::Assets,
        "y",
        ".bin",
        Content::Bytes(b"unique".to_vec()),
    );
    tx.create(&y, Content::Bytes(b"fixed".to_vec()));
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::RepeatedFile(_))
    ));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create(&y, Content::Bytes(b"safe".to_vec()));
    tx.create(
        &f.id(Area::Pages, "A.org"),
        Content::Bytes(b"* new\n".to_vec()),
    );
    let (_, rb) = refused(tx.commit());
    assert!(rb.kept_external.is_empty() && rb.undo_failed.is_empty());
    assert!(f.bytes("assets/y.bin").is_none());
    assert_eq!(f.bytes("assets/x.bin").unwrap(), b"x");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "OG-RULES Rule 8")]
fn raw_page_write_without_an_edit_kind_is_a_caller_bug() {
    let f = Fixture::new();
    let id = f.id(Area::Pages, "Kindless.md");
    let mut tx = f.store.transaction(None);
    tx.create(&id, Content::Bytes(b"- no intent\n".to_vec()));
    let _ = tx.commit();
}

#[test]
fn raw_non_page_write_needs_no_edit_kind() {
    let f = Fixture::new();
    let id = f.id(Area::Assets, "note.txt");
    let mut tx = f.store.transaction(None);
    tx.create(&id, Content::Bytes(b"asset".to_vec()));
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
}

#[test]
fn transaction_journal_create_updates_day_and_view() {
    let f = Fixture::new();
    let id = f.id(Area::Journals, "2026_09_25.org");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create(&id, Content::Bytes(b"* arrived\n".to_vec()));
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    assert_eq!(
        f.store.journal_id(tine_store::Day(20260925)).as_str(),
        id.as_str()
    );
    let view = f.store.whole_graph().unwrap();
    assert!(
        matches!(view.resolve("Sep 25th, 2026", true), tine_store::Resolved::Existing { id: found, .. } if found.as_str() == id.as_str())
    );
    assert!(view
        .inventory()
        .0
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
    assert!(view
        .complete_page_names("Sep 25", 10)
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
}

#[test]
fn raw_page_create_refuses_hostile_input_but_move_preserves_rescue_bytes() {
    let f = Fixture::new();
    let hostile = format!("- {}x{}\n", "[".repeat(513), "]".repeat(513));
    let page = f.id(Area::Pages, "Deep.md");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create(&page, Content::Bytes(hostile.as_bytes().to_vec()));
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    assert!(f.bytes("pages/Deep.md").is_none());

    let stream_source = f.root.join("hostile-stream");
    fs::write(&stream_source, hostile.as_bytes()).unwrap();
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create(
        &page,
        Content::Stream {
            source: File::open(&stream_source).unwrap(),
            max_bytes: hostile.len() as u64,
        },
    );
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    assert!(f.bytes("pages/Deep.md").is_none());

    f.put("assets/deep.txt", hostile.as_bytes());
    let source = f.id(Area::Assets, "deep.txt");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.move_file(&source, f.rev(&source), &page, None);
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    assert!(f.bytes("assets/deep.txt").is_none());
    assert_eq!(f.bytes("pages/Deep.md"), Some(hostile.into_bytes()));
    assert!(f
        .store
        .whole_graph()
        .unwrap()
        .unreadable_files()
        .iter()
        .any(|(id, _)| { id.as_str() == "pages/Deep.md" }));
}

#[test]
fn stage_one_conflicts_for_guarded_steps() {
    let f = Fixture::new();
    f.put("pages/A.md", b"- a\n");
    f.put("assets/x.bin", b"x");
    let a = PageId::from("pages/A.md");
    let x = f.id(Area::Assets, "x.bin");
    let stale = FileRev::from("0000000000000000".to_owned());
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.save_page(
        &[tine_store::EditKind::ReplacePage],
        &a,
        SaveBase::Existing(stale.clone()),
        &doc("A", "new"),
    );
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.replace(&x, stale.clone(), b"new".to_vec());
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.rewrite_refs(
        &a,
        stale.clone(),
        &RenameMap(vec![("A".into(), "B".into())]),
    );
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.move_file(&x, stale.clone(), &f.id(Area::Assets, "y.bin"), None);
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.trash(&x, stale, tine_store::TrashIf::Any);
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    assert_eq!(f.bytes("pages/A.md").unwrap(), b"- a\n");
    assert_eq!(f.bytes("assets/x.bin").unwrap(), b"x");
}

#[test]
fn indexed_twin_is_refused_before_any_write() {
    let f = Fixture::new();
    f.put("pages/Twin.org", b"* existing\n");
    f.put("assets/other.bin", b"old");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.replace(
        &f.id(Area::Assets, "other.bin"),
        f.rev(&f.id(Area::Assets, "other.bin")),
        b"new".to_vec(),
    );
    tx.create(
        &f.id(Area::Pages, "Twin.md"),
        Content::Bytes(b"- proposed\n".to_vec()),
    );
    let (why, rollback) = refused(tx.commit());
    assert!(matches!(why, Why::Refused(Refusal::Twin { .. })), "{why:?}");
    assert!(rollback.kept_external.is_empty() && rollback.undo_failed.is_empty());
    assert_eq!(f.bytes("assets/other.bin").unwrap(), b"old");
    assert!(f.bytes("pages/Twin.md").is_none());
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.save_page(
        &[tine_store::EditKind::ReplacePage],
        &PageId::from("pages/Twin.md"),
        SaveBase::CreateNew,
        &doc("Twin", "mine"),
    );
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::Twin { .. })
    ));
}

#[test]
fn transaction_refuses_guide_dto_without_writing() {
    let f = Fixture::new();
    let mut guide = doc("Guide", "should stay ephemeral");
    guide.guide = true;
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.save_page(
        &[tine_store::EditKind::ReplacePage],
        &PageId::from("pages/Guide.md"),
        SaveBase::CreateNew,
        &guide,
    );
    assert!(matches!(refused(tx.commit()).0, Why::Refused(_)));
    assert!(f.bytes("pages/Guide.md").is_none());
}

#[test]
fn create_unique_refuses_an_indexed_page_twin() {
    let f = Fixture::with_watch(WatchMode::Poll, &[("pages/Twin.md", b"- existing\n")]);
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create_unique(
        Area::Pages,
        "twin",
        ".org",
        Content::Bytes(b"* duplicate\n".to_vec()),
    );
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::Twin { .. })
    ));
    assert!(f.bytes("pages/twin.org").is_none());
}

#[test]
fn unique_names_and_stream_limit() {
    let f = Fixture::new();
    f.put("assets/x.png", b"old");
    f.put("assets/x_1.png", b"old1");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create_unique(Area::Assets, "x", ".png", Content::Bytes(b"new".to_vec()));
    match &committed(tx.commit())[0] {
        StepResult::Written { file, .. } => assert_eq!(file.as_str(), "assets/x_2.png"),
        other => panic!("{other:?}"),
    }
    assert_eq!(f.bytes("assets/x_2.png").unwrap(), b"new");
    let source = f.root.join("source.bin");
    let mut handle = File::create(&source).unwrap();
    handle.write_all(b"12345").unwrap();
    drop(handle);
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create_unique(
        Area::Assets,
        "large",
        ".bin",
        Content::Stream {
            source: File::open(source).unwrap(),
            max_bytes: 4,
        },
    );
    assert!(matches!(refused(tx.commit()).0, Why::Failed(_)));
    assert!(f.bytes("assets/large.bin").is_none());
    let leftovers: Vec<_> = fs::read_dir(f.root.join("assets"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("tine-tx"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[cfg(feature = "test-faults")]
#[test]
fn post_apply_read_failure_cannot_report_complete_publication() {
    use tine_store::FaultPoint;
    let f = Fixture::with_watch(WatchMode::Poll, &[("assets/x.bin", b"old")]);
    let id = f.id(Area::Assets, "x.bin");
    let mut tx = f.store.transaction(None);
    tx.replace(&id, f.rev(&id), b"new".to_vec());
    f.store.inject_fault(FaultPoint::PublicationReadIo);
    let outcome = tx.commit();
    assert!(!matches!(outcome, TxOutcome::Committed { .. }),
        "I-9: a post-apply read error must not report complete publication; exemplar transaction.rs post-apply sweep: {outcome:?}");
    assert_eq!(f.bytes("assets/x.bin"), Some(b"new".to_vec()));
}

#[test]
fn transaction_revision_advances_only_for_disk_change() {
    let f = Fixture::with_watch(
        WatchMode::Poll,
        &[("assets/x.bin", b"old"), ("pages/A.md", b"- same\n")],
    );
    let x = f.id(Area::Assets, "x.bin");
    let initial = f.store.whole_graph().unwrap().rev();
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.replace(&x, f.rev(&x), b"new".to_vec());
    let changed = match tx.commit() {
        TxOutcome::Committed { graph_rev, .. } => graph_rev,
        other => panic!("{other:?}"),
    };
    assert!(changed > initial);
    f.store.scan_refresh().unwrap();
    let before_unchanged = f.store.whole_graph().unwrap().rev();
    let a = PageId::from("pages/A.md");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.save_page(
        &[tine_store::EditKind::ReplacePage],
        &a,
        SaveBase::Existing(f.rev(&a.file())),
        &doc("A", "same"),
    );
    let unchanged = match tx.commit() {
        TxOutcome::Committed {
            steps, graph_rev, ..
        } => {
            assert!(matches!(steps[0], StepResult::Unchanged { .. }));
            graph_rev
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(unchanged, before_unchanged);
}

#[cfg(feature = "test-faults")]
mod faults {
    use super::*;
    use tine_store::FaultPoint;

    #[test]
    fn failed_undo_of_own_bytes_stays_own_in_report_and_feed() {
        let f = Fixture::new();
        let changes = f.store.subscribe();
        let a = f.id(Area::Pages, "A.md");
        let b = f.id(Area::Pages, "B.md");
        f.store.inject_fault(FaultPoint::MidStepIoAt(1));
        f.store.inject_fault(FaultPoint::UndoWithdrawalIo);
        let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.create(&a, Content::Bytes(b"- first\n".to_vec()));
        tx.create(&b, Content::Bytes(b"- second\n".to_vec()));
        let TxOutcome::NotCommitted { rollback, .. } = tx.commit() else {
            panic!("second step must fail");
        };
        assert!(!rollback.undo_failed.is_empty());
        assert!(
            rollback.kept_external.is_empty(),
            "own bytes are not external: {rollback:?}"
        );
        let change = changes.try_recv().unwrap().expect("leftover publication");
        assert_eq!(change.origin, tine_store::Origin::Own);
        assert_eq!(f.bytes("pages/B.md"), Some(b"- second\n".to_vec()));
    }

    fn triple(point: FaultPoint, undo_writer: bool) {
        let f = Fixture::new();
        f.put("pages/A.md", b"- old A\n");
        f.put("pages/B.md", b"- [[A]]\n");
        f.put("assets/c.bin", b"old C");
        let a = PageId::from("pages/A.md");
        let b = f.id(Area::Pages, "B.md");
        let c = f.id(Area::Assets, "c.bin");
        let d = f.id(Area::Pages, "D.md");
        let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.save_page(
            &[tine_store::EditKind::ReplacePage],
            &a,
            SaveBase::Existing(f.rev(&a.file())),
            &doc("A", "new A"),
        );
        tx.move_file(
            &b,
            f.rev(&b),
            &d,
            Some(&RenameMap(vec![("A".into(), "D".into())])),
        );
        tx.trash(&c, f.rev(&c), tine_store::TrashIf::Any);
        f.store.inject_fault(point);
        if undo_writer {
            f.store.inject_fault(FaultPoint::UndoLiveWrite);
        }
        let (why, rollback) = refused(tx.commit());
        assert!(matches!(why, Why::Conflict { .. } | Why::Failed(_)));
        assert!(rollback.undo_failed.is_empty(), "{rollback:?}");
        if undo_writer
            || matches!(
                point,
                FaultPoint::Stage2Mismatch
                    | FaultPoint::Stage2MismatchAt(_)
                    | FaultPoint::NoReplaceCollision
            )
        {
            assert!(!rollback.kept_external.is_empty());
        } else {
            assert!(rollback.kept_external.is_empty(), "{rollback:?}");
        }
        if point != FaultPoint::Stage2Mismatch && !(undo_writer && point == FaultPoint::MidStepIo) {
            assert_eq!(f.bytes("pages/A.md").unwrap(), b"- old A\n");
        }
        assert_eq!(f.bytes("pages/B.md").unwrap(), b"- [[A]]\n");
        if point != FaultPoint::Stage2MismatchAt(2) {
            assert_eq!(f.bytes("assets/c.bin").unwrap(), b"old C");
        }
        if f.bytes("pages/D.md").is_some() {
            assert!(rollback
                .kept_external
                .iter()
                .any(|(file, _)| file.as_str() == "pages/D.md"));
        }
        let recovery = f.root.join("logseq/.tine-trash");
        let mut collected = Vec::new();
        fn visit(dir: &Path, out: &mut Vec<Vec<u8>>) {
            if !dir.exists() {
                return;
            }
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path, out);
                } else {
                    out.push(fs::read(path).unwrap());
                }
            }
        }
        visit(&recovery, &mut collected);
        let old_a_live = f.bytes("pages/A.md").as_deref() == Some(b"- old A\n");
        assert!(old_a_live || collected.iter().any(|v| v == b"- old A\n"));
        let old_c_live = f.bytes("assets/c.bin").as_deref() == Some(b"old C");
        assert!(old_c_live || collected.iter().any(|v| v == b"old C"));
    }

    #[test]
    fn injected_three_step_failures_preserve_baselines() {
        triple(FaultPoint::Stage2Mismatch, false);
        triple(FaultPoint::NoReplaceCollision, false);
        triple(FaultPoint::MidStepIo, false);
        triple(FaultPoint::MidStepIo, true);
        triple(FaultPoint::Stage2MismatchAt(2), false);
        triple(FaultPoint::MidStepIoAt(2), false);
        triple(FaultPoint::MidStepIoAt(2), true);
    }

    #[test]
    fn late_twin_withdraws_only_created_file() {
        let f = Fixture::new();
        let page = f.id(Area::Pages, "Twin.md");
        f.store.inject_fault(FaultPoint::TwinAfterPublish);
        let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.create(&page, Content::Bytes(b"- mine\n".to_vec()));
        assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
        assert!(f.bytes("pages/Twin.md").is_none());
        assert_eq!(f.bytes("pages/Twin.org").unwrap(), b"external twin");
    }

    #[test]
    fn each_step_rolls_back_after_mid_step_io() {
        for case in 0..7 {
            let f = Fixture::new();
            f.put("pages/A.md", b"- old A\n");
            f.put("pages/B.md", b"- [[A]]\n");
            f.put("assets/x.bin", b"old x");
            let a = PageId::from("pages/A.md");
            let b = PageId::from("pages/B.md");
            let x = f.id(Area::Assets, "x.bin");
            let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
            match case {
                0 => {
                    tx.save_page(
                        &[tine_store::EditKind::ReplacePage],
                        &a,
                        SaveBase::Existing(f.rev(&a.file())),
                        &doc("A", "new"),
                    );
                }
                1 => {
                    tx.create(
                        &f.id(Area::Assets, "new.bin"),
                        Content::Bytes(b"new".to_vec()),
                    );
                }
                2 => {
                    tx.create_unique(
                        Area::Assets,
                        "unique",
                        ".bin",
                        Content::Bytes(b"new".to_vec()),
                    );
                }
                3 => {
                    tx.replace(&x, f.rev(&x), b"new".to_vec());
                }
                4 => {
                    tx.rewrite_refs(
                        &b,
                        f.rev(&b.file()),
                        &RenameMap(vec![("A".into(), "Z".into())]),
                    );
                }
                5 => {
                    tx.move_file(
                        &b.file(),
                        f.rev(&b.file()),
                        &f.id(Area::Pages, "Moved.md"),
                        None,
                    );
                }
                _ => {
                    tx.trash(&x, f.rev(&x), tine_store::TrashIf::Any);
                }
            }
            f.store.inject_fault(FaultPoint::MidStepIo);
            let (why, rollback) = refused(tx.commit());
            assert!(matches!(why, Why::Failed(_)), "case {case}: {why:?}");
            assert!(
                rollback.kept_external.is_empty() && rollback.undo_failed.is_empty(),
                "case {case}: {rollback:?}"
            );
            assert_eq!(f.bytes("pages/A.md").unwrap(), b"- old A\n", "case {case}");
            assert_eq!(f.bytes("pages/B.md").unwrap(), b"- [[A]]\n", "case {case}");
            assert_eq!(f.bytes("assets/x.bin").unwrap(), b"old x", "case {case}");
            for path in ["assets/new.bin", "assets/unique.bin", "pages/Moved.md"] {
                assert!(f.bytes(path).is_none(), "case {case}: {path}");
            }
        }
    }
}

#[test]
fn opposite_name_order_serializes_without_deadlock() {
    let f = Fixture::new();
    f.put("assets/a.bin", b"a");
    f.put("assets/b.bin", b"b");
    let a = f.id(Area::Assets, "a.bin");
    let b = f.id(Area::Assets, "b.bin");
    let ar = f.rev(&a);
    let br = f.rev(&b);
    let barrier = Arc::new(Barrier::new(3));
    let (sender, receiver) = mpsc::channel();
    for reverse in [false, true] {
        let store = Arc::clone(&f.store);
        let barrier = Arc::clone(&barrier);
        let sender = sender.clone();
        let (a, b, ar, br) = (a.clone(), b.clone(), ar.clone(), br.clone());
        std::thread::spawn(move || {
            let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
            if reverse {
                tx.replace(&b, br, b"B".to_vec())
                    .replace(&a, ar, b"A".to_vec());
            } else {
                tx.replace(&a, ar, b"A".to_vec())
                    .replace(&b, br, b"B".to_vec());
            }
            barrier.wait();
            sender.send(tx.commit()).unwrap();
        });
    }
    barrier.wait();
    for _ in 0..2 {
        let result = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("transaction deadlocked");
        assert!(matches!(
            result,
            TxOutcome::Committed { .. } | TxOutcome::NotCommitted { .. }
        ));
    }
}

/// A move whose content does not change is a no-replace rename: nothing is
/// left in the trash on success, and undo restores the source name.
#[test]
fn unchanged_move_is_a_rename_without_trash_copy() {
    let f = Fixture::new();
    f.put("pages/A.md", b"- a\n");
    let a = f.id(Area::Pages, "A.md");
    let b = f.id(Area::Pages, "B.md");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.move_file(&a, f.rev(&a), &b, None);
    let steps = committed(tx.commit());
    assert!(matches!(&steps[0], StepResult::Moved { to, .. } if *to == b));
    assert!(f.bytes("pages/A.md").is_none());
    assert_eq!(f.bytes("pages/B.md").unwrap(), b"- a\n");
    assert!(!f.root.join("logseq/.tine-trash").exists());
}

#[cfg(feature = "test-faults")]
mod rename_faults {
    use super::*;
    use tine_store::FaultPoint;

    fn two_steps(f: &Fixture) -> TxOutcome {
        f.put("assets/a.bin", b"old a");
        f.put("assets/c.bin", b"old c");
        let a = f.id(Area::Assets, "a.bin");
        let b = f.id(Area::Assets, "b.bin");
        let c = f.id(Area::Assets, "c.bin");
        let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.move_file(&a, f.rev(&a), &b, None);
        tx.trash(&c, f.rev(&c), tine_store::TrashIf::Any);
        tx.commit()
    }

    #[test]
    fn later_failure_renames_back() {
        let f = Fixture::new();
        f.store.inject_fault(FaultPoint::MidStepIoAt(1));
        let (why, rollback) = refused(two_steps(&f));
        assert!(matches!(why, Why::Failed(_)), "{why:?}");
        assert!(rollback.kept_external.is_empty() && rollback.undo_failed.is_empty());
        assert_eq!(f.bytes("assets/a.bin").unwrap(), b"old a");
        assert!(f.bytes("assets/b.bin").is_none());
        assert_eq!(f.bytes("assets/c.bin").unwrap(), b"old c");
    }

    #[test]
    fn third_party_write_during_undo_keeps_both_versions() {
        let f = Fixture::new();
        f.store.inject_fault(FaultPoint::MidStepIoAt(1));
        f.store.inject_fault(FaultPoint::UndoLiveWrite);
        let (_, rollback) = refused(two_steps(&f));
        assert_eq!(f.bytes("assets/a.bin").unwrap(), b"old a");
        assert_eq!(f.bytes("assets/b.bin").unwrap(), b"external during undo");
        assert!(rollback
            .kept_external
            .iter()
            .any(|(id, _)| id.as_str() == "assets/b.bin"));
        assert_eq!(f.bytes("assets/c.bin").unwrap(), b"old c");
    }
}

/// `IoError::kind` is platform-independent: creating over a directory reports
/// `IsADirectory` everywhere, although Windows fails the read with
/// ERROR_ACCESS_DENIED where Unix reports EISDIR. The Guide copy relies on it to
/// skip an occupied name rather than abort ("Access is denied" on Windows).
#[test]
fn create_over_a_directory_reports_is_a_directory_on_every_platform() {
    let f = Fixture::new();
    fs::create_dir(f.root.join("assets").join("taken.png")).unwrap();
    let id = f.id(Area::Assets, "taken.png");
    let mut tx = f.store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create(&id, Content::Bytes(b"new".to_vec()));
    let (why, _) = refused(tx.commit());
    match why {
        Why::Failed(error) => assert_eq!(error.kind, std::io::ErrorKind::IsADirectory, "{error:?}"),
        other => panic!("expected an I/O failure: {other:?}"),
    }
    assert!(f.root.join("assets").join("taken.png").is_dir());
}

/// Bytes this thread has read through `read`-family syscalls (Linux).
#[cfg(target_os = "linux")]
fn thread_read_bytes() -> u64 {
    let io = fs::read_to_string("/proc/thread-self/io").unwrap();
    io.lines()
        .find_map(|line| line.strip_prefix("rchar: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// REG-OG-C5-L06-S2 (I-22): saving an asset whose name is held by a large
/// file probes the name; it must not read the occupant. Before the fix the
/// unique-name probe loaded the whole occupant into memory to hash a revision
/// it then discarded, so a 1 GiB video under the name could OOM the app.
/// A sparse 256 MiB occupant stands in (no disk cost); the bound is the
/// per-thread read count, which a whole-file read exceeds by 256 MiB.
#[cfg(target_os = "linux")]
#[test]
fn unique_asset_name_probe_never_reads_the_occupant() {
    let f = Fixture::new();
    let big = File::create(f.root.join("assets/movie.mp4")).unwrap();
    big.set_len(256 * 1024 * 1024).unwrap();
    drop(big);
    let before = thread_read_bytes();
    let saved = tine_graph_features::assets::save_asset(&f.store, "movie.mp4", b"small").unwrap();
    let read = thread_read_bytes() - before;
    assert_eq!(saved, "movie_1.mp4");
    assert_eq!(f.bytes("assets/movie_1.mp4").unwrap(), b"small");
    assert!(
        read < 16 * 1024 * 1024,
        "I-22: the unique-name probe read {read} bytes of an occupied 256 MiB asset; exemplar transaction.rs Transaction::occupied"
    );
}

/// REG-OG-C5-L06-S1 sibling (I-1, I-2): trashing an asset moves its only copy
/// into `logseq/.tine-trash/...`, creating the trash directories on first
/// use. Scenario: power loss after the move — the source directory's sync makes
/// the removal durable, so every directory created for the destination must
/// have its own entry synced too, or the asset is lost.
#[cfg(feature = "test-faults")]
#[test]
fn trash_into_new_directories_syncs_every_created_entry() {
    let f = Fixture::with_watch(WatchMode::Notify, &[("assets/clip.bin", b"only copy")]);
    let clip = f.id(Area::Assets, "clip.bin");
    assert!(!f.root.join("logseq/.tine-trash").exists());
    tine_store::directory_durability::take_synced_directories();
    let mut tx = f.store.transaction(None);
    tx.trash(&clip, f.rev(&clip), tine_store::TrashIf::Any);
    let steps = committed(tx.commit());
    let synced = tine_store::directory_durability::take_synced_directories();
    let StepResult::Trashed { trashed, .. } = &steps[0] else {
        panic!("expected trash: {steps:?}");
    };
    let canonical = fs::canonicalize(&f.root).unwrap();
    let trash = canonical.join(trashed.as_str());
    assert_eq!(fs::read(&trash).unwrap(), b"only copy");
    let mut dir = trash.parent().unwrap().to_path_buf();
    while dir != canonical.join("logseq") {
        let parent = dir.parent().unwrap().to_path_buf();
        assert!(
            synced.iter().any(|path| fs::canonicalize(path).ok().as_ref() == Some(&parent)),
            "I-2: created directory {} was never made durable in {}; exemplar directory_durability::create_dir_all_durable; synced {synced:?}",
            dir.display(),
            parent.display()
        );
        dir = parent;
    }
}
