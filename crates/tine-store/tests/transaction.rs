use std::fs::{self, File};
use std::io::Write;
#[cfg(feature = "test-faults")]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::model::Graph;
use tine_store::{
    Area, Content, FileId, FileRev, PageId, Refusal, RenameMap, SaveBase, StepResult, Store,
    TxOutcome, Why,
};

struct Fixture {
    root: PathBuf,
    store: Arc<Store>,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-transaction-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for dir in ["pages", "journals", "assets", "logseq"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let store = Arc::new(Store::from_legacy(Arc::new(Graph::open(&root))));
        Self { root, store }
    }

    fn id(&self, area: Area, rel: &str) -> FileId {
        self.store.file_id(area, rel).unwrap()
    }

    fn put(&self, rel: &str, bytes: &[u8]) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
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
        path: None,
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
    let mut tx = f.store.transaction();
    tx.save_page(&a, SaveBase::Existing(f.rev(&a.file())), &doc("A", "after"));
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Written { .. }
    ));
    let mut tx = f.store.transaction();
    tx.save_page(&a, SaveBase::Existing(f.rev(&a.file())), &doc("A", "after"));
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Unchanged { .. }
    ));
    let mut tx = f.store.transaction();
    tx.create(&created, Content::Bytes(b"new".to_vec()));
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Written { .. }
    ));
    let mut tx = f.store.transaction();
    tx.replace(&meta, f.rev(&meta), b"replacement".to_vec());
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Written { .. }
    ));
    let mut tx = f.store.transaction();
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
    let mut tx = f.store.transaction();
    tx.move_file(&b, f.rev(&b), &moved, None);
    assert!(matches!(
        committed(tx.commit())[0],
        StepResult::Moved { .. }
    ));
    assert!(f.bytes("pages/B.md").is_none());
    assert_eq!(f.bytes("pages/C.md").unwrap(), b"- [[C]]\n");
    let mut tx = f.store.transaction();
    tx.trash(&delete, f.rev(&delete));
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
    let mut tx = f.store.transaction();
    tx.create(&a, Content::Bytes(b"- new\n".to_vec()));
    let (why, rb) = refused(tx.commit());
    assert!(matches!(why, Why::Conflict { .. }));
    assert!(rb.kept_external.is_empty() && rb.undo_failed.is_empty());
    let mut tx = f.store.transaction();
    tx.create(&f.id(Area::Pages, "B.md"), Content::Bytes(vec![0xff]));
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::Undecodable)
    ));
    let mut tx = f.store.transaction();
    tx.replace(&a, f.rev(&a), b"bad".to_vec());
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    let mut tx = f.store.transaction();
    tx.move_file(&x, f.rev(&x), &f.id(Area::Trash, "assets/illegal"), None);
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    let mut tx = f.store.transaction();
    tx.create_unique(Area::Assets, "../bad", "png", Content::Bytes(vec![1]));
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    assert!(f.store.file_id(Area::Meta, ".tine-trash/forged").is_err());
    f.put("logseq/config.edn", b"{}\n");
    let config = f.id(Area::Meta, "config.edn");
    let mut tx = f.store.transaction();
    tx.replace(
        &config,
        f.rev(&config),
        b"{:preferred-format :org}\n".to_vec(),
    );
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::InvalidTarget(_))
    ));
    assert_eq!(f.bytes("logseq/config.edn").unwrap(), b"{}\n");
    let mut tx = f.store.transaction();
    tx.replace(&x, f.rev(&x), b"new".to_vec())
        .create(&x, Content::Bytes(b"again".to_vec()));
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::RepeatedFile(_))
    ));
    let mut tx = f.store.transaction();
    tx.create_unique(Area::Assets, "y", "bin", Content::Bytes(b"unique".to_vec()));
    tx.create(&y, Content::Bytes(b"fixed".to_vec()));
    assert!(matches!(
        refused(tx.commit()).0,
        Why::Refused(Refusal::RepeatedFile(_))
    ));
    let mut tx = f.store.transaction();
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
fn stage_one_conflicts_for_guarded_steps() {
    let f = Fixture::new();
    f.put("pages/A.md", b"- a\n");
    f.put("assets/x.bin", b"x");
    let a = PageId::from("pages/A.md");
    let x = f.id(Area::Assets, "x.bin");
    let stale = FileRev::from("0000000000000000".to_owned());
    let mut tx = f.store.transaction();
    tx.save_page(&a, SaveBase::Existing(stale.clone()), &doc("A", "new"));
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    let mut tx = f.store.transaction();
    tx.replace(&x, stale.clone(), b"new".to_vec());
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    let mut tx = f.store.transaction();
    tx.rewrite_refs(
        &a,
        stale.clone(),
        &RenameMap(vec![("A".into(), "B".into())]),
    );
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    let mut tx = f.store.transaction();
    tx.move_file(&x, stale.clone(), &f.id(Area::Assets, "y.bin"), None);
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    let mut tx = f.store.transaction();
    tx.trash(&x, stale);
    assert!(matches!(refused(tx.commit()).0, Why::Conflict { .. }));
    assert_eq!(f.bytes("pages/A.md").unwrap(), b"- a\n");
    assert_eq!(f.bytes("assets/x.bin").unwrap(), b"x");
}

#[test]
fn indexed_twin_is_refused_before_any_write() {
    let f = Fixture::new();
    f.put("pages/Twin.org", b"* existing\n");
    f.put("assets/other.bin", b"old");
    let mut tx = f.store.transaction();
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
    let mut tx = f.store.transaction();
    tx.save_page(
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
fn unique_names_and_stream_limit() {
    let f = Fixture::new();
    f.put("assets/x.png", b"old");
    f.put("assets/x_1.png", b"old1");
    let mut tx = f.store.transaction();
    tx.create_unique(Area::Assets, "x", "png", Content::Bytes(b"new".to_vec()));
    match &committed(tx.commit())[0] {
        StepResult::Written { file, .. } => assert_eq!(file.as_str(), "assets/x_2.png"),
        other => panic!("{other:?}"),
    }
    assert_eq!(f.bytes("assets/x_2.png").unwrap(), b"new");
    let source = f.root.join("source.bin");
    let mut handle = File::create(&source).unwrap();
    handle.write_all(b"12345").unwrap();
    drop(handle);
    let mut tx = f.store.transaction();
    tx.create_unique(
        Area::Assets,
        "large",
        "bin",
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

#[test]
fn transaction_revision_advances_only_for_disk_change() {
    let f = Fixture::new();
    f.put("assets/x.bin", b"old");
    let x = f.id(Area::Assets, "x.bin");
    let initial = f.store.whole_graph().unwrap().rev();
    let mut tx = f.store.transaction();
    tx.replace(&x, f.rev(&x), b"new".to_vec());
    let changed = match tx.commit() {
        TxOutcome::Committed { graph_rev, .. } => graph_rev,
        other => panic!("{other:?}"),
    };
    assert!(changed > initial);
    let a = PageId::from("pages/A.md");
    f.put("pages/A.md", b"- same\n");
    let mut tx = f.store.transaction();
    tx.save_page(&a, SaveBase::Existing(f.rev(&a.file())), &doc("A", "same"));
    let unchanged = match tx.commit() {
        TxOutcome::Committed { steps, graph_rev } => {
            assert!(matches!(steps[0], StepResult::Unchanged { .. }));
            graph_rev
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(unchanged, changed);
}

#[cfg(feature = "test-faults")]
mod faults {
    use super::*;
    use tine_store::FaultPoint;

    fn triple(point: FaultPoint, undo_writer: bool) {
        let f = Fixture::new();
        f.put("pages/A.md", b"- old A\n");
        f.put("pages/B.md", b"- [[A]]\n");
        f.put("assets/c.bin", b"old C");
        let a = PageId::from("pages/A.md");
        let b = f.id(Area::Pages, "B.md");
        let c = f.id(Area::Assets, "c.bin");
        let d = f.id(Area::Pages, "D.md");
        let mut tx = f.store.transaction();
        tx.save_page(&a, SaveBase::Existing(f.rev(&a.file())), &doc("A", "new A"));
        tx.move_file(
            &b,
            f.rev(&b),
            &d,
            Some(&RenameMap(vec![("A".into(), "D".into())])),
        );
        tx.trash(&c, f.rev(&c));
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
        let mut tx = f.store.transaction();
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
            let mut tx = f.store.transaction();
            match case {
                0 => {
                    tx.save_page(&a, SaveBase::Existing(f.rev(&a.file())), &doc("A", "new"));
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
                        "bin",
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
                    tx.trash(&x, f.rev(&x));
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
            let mut tx = store.transaction();
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
