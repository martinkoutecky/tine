//! Save matrix against values captured from v0.6.5 Graph at 9c3d7c376.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::{FaultPoint, PageId, SaveBase, SaveOutcome, SavePagesOutcome, Store, StoreError};

#[test]
fn g6b_one_block_edit_preserves_fixture_layout() {
    for (name, source) in [
        (
            "Cont1",
            include_bytes!("../../../scripts/fixtures/g6b/graph/pages/Cont1.md").as_slice(),
        ),
        (
            "Trailing",
            include_bytes!("../../../scripts/fixtures/g6b/graph/pages/Trailing.md").as_slice(),
        ),
        (
            "TrailingBlanks",
            include_bytes!("../../../scripts/fixtures/g6b/graph/pages/TrailingBlanks.md")
                .as_slice(),
        ),
    ] {
        let fixture = Fixture::new();
        let rel = format!("pages/{name}.md");
        fixture.write(&rel, source);
        let store = Store::open(&fixture.0, Default::default()).unwrap().0;
        let id = PageId::from(rel.as_str());
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw.push_str(" edited");
        assert!(matches!(
            store.save(
                tine_store::EditKind::SaveBlock,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        let expected =
            String::from_utf8(source.to_vec())
                .unwrap()
                .replacen("- target", "- target edited", 1);
        assert_eq!(
            fs::read(&fixture.0.join(rel)).unwrap(),
            expected.as_bytes(),
            "{name}"
        );
        store.close();
    }
}

#[test]
fn g6b_explicit_eof_edit_is_not_lost() {
    let source = include_bytes!("../../../scripts/fixtures/g6b/graph/pages/TrailingBlanks.md");
    let fixture = Fixture::new();
    fixture.write("pages/TrailingBlanks.md", source);
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = PageId::from("pages/TrailingBlanks.md");
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks[1].raw.push('\n');
    assert!(matches!(
        store.save(
            tine_store::EditKind::SaveBlock,
            &id,
            SaveBase::Existing(read.rev),
            &doc
        ),
        SaveOutcome::Saved(_)
    ));
    let after = fs::read(fixture.0.join("pages/TrailingBlanks.md")).unwrap();
    let count = |bytes: &[u8]| {
        bytes
            .iter()
            .rev()
            .take_while(|&&byte| byte == b'\n')
            .count()
    };
    assert!(count(&after) > count(source));
    store.close();
}

#[test]
fn save_pages_keeps_order_and_reports_preflight_conflict_without_writes() {
    let fixture = Fixture::new();
    fixture.write("pages/A.md", "- old A\n");
    fixture.write("pages/B.md", "- old B\n");
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let a = PageId::from("pages/A.md");
    let b = PageId::from("pages/B.md");
    let read_a = store.page(&a).unwrap();
    let read_b = store.page(&b).unwrap();
    let mut doc_a = read_a.doc;
    let mut doc_b = read_b.doc;
    doc_a.blocks[0].raw = "new A".into();
    doc_b.blocks[0].raw = "new B".into();
    let entries = vec![
        (
            a.clone(),
            SaveBase::Existing(read_a.rev),
            doc_a,
            vec![tine_store::EditKind::ReplacePage],
        ),
        (
            b.clone(),
            SaveBase::Existing(read_b.rev),
            doc_b,
            vec![tine_store::EditKind::ReplacePage],
        ),
    ];
    fixture.write("pages/B.md", "- external B\n");
    assert!(matches!(
        store.save_pages(&entries),
        SavePagesOutcome::Failed {
            index: 1,
            outcome: SaveOutcome::Conflict { .. },
            ..
        }
    ));
    assert_eq!(
        fs::read(fixture.0.join("pages/A.md")).unwrap(),
        b"- old A\n"
    );
    assert_eq!(
        fs::read(fixture.0.join("pages/B.md")).unwrap(),
        b"- external B\n"
    );
    let (_, new_b_rev) = store.read(&b.file(), None).unwrap();
    let mut entries = entries;
    entries[1].1 = SaveBase::Existing(new_b_rev);
    let SavePagesOutcome::Ok { outcomes, .. } = store.save_pages(&entries) else {
        panic!("save must commit")
    };
    assert_eq!(outcomes.len(), 2);
    assert!(
        matches!(&outcomes[0], SaveOutcome::Saved(rev) if *rev == store.read(&a.file(), None).unwrap().1)
    );
    assert!(
        matches!(&outcomes[1], SaveOutcome::Saved(rev) if *rev == store.read(&b.file(), None).unwrap().1)
    );
    assert!(String::from_utf8(fixture.files()["pages/A.md"].clone())
        .unwrap()
        .contains("new A"));
    assert!(String::from_utf8(fixture.files()["pages/B.md"].clone())
        .unwrap()
        .contains("new B"));
}

#[test]
fn save_pages_mid_step_failure_restores_prior_files() {
    let fixture = Fixture::new();
    for name in ["A", "B", "C"] {
        fixture.write(&format!("pages/{name}.md"), format!("- old {name}\n"));
    }
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let entries: Vec<_> = ["A", "B", "C"]
        .into_iter()
        .map(|name| {
            let id = PageId::from(format!("pages/{name}.md"));
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = format!("new {name}");
            (
                id,
                SaveBase::Existing(read.rev),
                doc,
                vec![tine_store::EditKind::ReplacePage],
            )
        })
        .collect();
    store.inject_fault(FaultPoint::MidStepIoAt(1));
    assert!(matches!(
        store.save_pages(&entries),
        SavePagesOutcome::Failed {
            index: 1,
            outcome: SaveOutcome::Io(_),
            ..
        }
    ));
    for name in ["A", "B", "C"] {
        assert_eq!(
            fs::read(fixture.0.join(format!("pages/{name}.md"))).unwrap(),
            format!("- old {name}\n").as_bytes()
        );
    }
}

#[test]
fn save_pages_repeated_file_has_own_family() {
    let fixture = Fixture::new();
    fixture.write("pages/A.md", "- old\n");
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = PageId::from("pages/A.md");
    let read = store.page(&id).unwrap();
    let entry = (
        id,
        SaveBase::Existing(read.rev),
        read.doc,
        vec![tine_store::EditKind::ReplacePage],
    );
    assert!(matches!(
        store.save_pages(&[entry.clone(), entry]),
        SavePagesOutcome::Failed {
            outcome: SaveOutcome::Repeated,
            ..
        }
    ));
}

#[test]
fn save_pages_refuses_an_empty_kind_list_before_writing() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let entry = (
        PageId::from("pages/New.md"),
        SaveBase::CreateNew,
        fresh("New", PageKind::Page),
        Vec::new(),
    );
    assert!(matches!(
        store.save_pages(&[entry]),
        SavePagesOutcome::Failed {
            index: 0,
            outcome: SaveOutcome::InvalidTarget(_),
            ..
        }
    ));
    assert!(!fixture.0.join("pages/New.md").exists());
}

#[test]
fn save_pages_rollback_failure_names_recovery_location() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let entries = vec![
        (
            PageId::from("pages/A.md"),
            SaveBase::CreateNew,
            fresh("A", PageKind::Page),
            vec![tine_store::EditKind::CreatePage],
        ),
        (
            PageId::from("pages/B.md"),
            SaveBase::CreateNew,
            fresh("B", PageKind::Page),
            vec![tine_store::EditKind::CreatePage],
        ),
    ];
    store.inject_fault(FaultPoint::MidStepIoAt(1));
    store.inject_fault(FaultPoint::UndoWithdrawalIo);
    let outcome = store.save_pages(&entries);
    assert!(
        matches!(&outcome, SavePagesOutcome::Failed {
        index: 1, outcome: SaveOutcome::Io(_), undo_failed, ..
    } if format!("{undo_failed:?}").contains("pages/B.md")),
        "{outcome:?}"
    );
}

#[test]
fn save_pages_names_files_missing_from_publication_after_disk_write() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    store.inject_fault(FaultPoint::PublicationReadIo);
    let outcome = store.save_pages(&[(
        PageId::from("pages/New.md"),
        SaveBase::CreateNew,
        fresh("New", PageKind::Page),
        vec![tine_store::EditKind::CreatePage],
    )]);
    assert!(
        matches!(&outcome, SavePagesOutcome::Failed { publication_errors, .. }
        if format!("{publication_errors:?}").contains("pages/New.md")),
        "{outcome:?}"
    );
    assert!(
        fixture.0.join("pages/New.md").is_file(),
        "the write landed before publication failed"
    );
}

#[test]
fn single_save_reports_incomplete_rollback() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    store.inject_fault(FaultPoint::MidStepIoAt(0));
    store.inject_fault(FaultPoint::UndoWithdrawalIo);
    let outcome = store.save(
        tine_store::EditKind::CreatePage,
        &PageId::from("pages/New.md"),
        SaveBase::CreateNew,
        &fresh("New", PageKind::Page),
    );
    assert!(
        matches!(outcome, SaveOutcome::Io(ref error) if error.message.contains("rollback incomplete") && error.message.contains("pages/New.md")),
        "{outcome:?}"
    );
}

#[test]
fn save_pages_crash_worker() {
    let Ok(root) = std::env::var("TINE_SAVE_PAGES_CRASH_ROOT") else {
        return;
    };
    let store = Store::open(Path::new(&root), Default::default()).unwrap().0;
    let entries: Vec<_> = ["A", "B", "C"]
        .into_iter()
        .map(|name| {
            let id = PageId::from(format!("pages/{name}.md"));
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = format!("new {name}");
            (
                id,
                SaveBase::Existing(read.rev),
                doc,
                vec![tine_store::EditKind::ReplacePage],
            )
        })
        .collect();
    store.inject_fault(FaultPoint::AbortAfterStep(0));
    let _ = store.save_pages(&entries);
    panic!("save_pages crash fault did not abort");
}

#[test]
fn save_pages_crash_between_steps_keeps_earlier_disk_write() {
    let fixture = Fixture::new();
    for name in ["A", "B", "C"] {
        fixture.write(&format!("pages/{name}.md"), format!("- old {name}\n"));
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("save_pages_crash_worker")
        .env("TINE_SAVE_PAGES_CRASH_ROOT", &fixture.0)
        .output()
        .unwrap();
    assert!(!output.status.success(), "fault must abort between steps");
    assert!(String::from_utf8(fixture.files()["pages/A.md"].clone())
        .unwrap()
        .contains("new A"));
    assert_eq!(fixture.files()["pages/B.md"], b"- old B\n");
    assert_eq!(fixture.files()["pages/C.md"], b"- old C\n");
}

#[derive(Clone, Copy, Debug)]
enum Case {
    New,
    Appeared,
    Matching,
    Stale,
    Deleted,
    OrgReadOnly,
    OrgEditable,
    Crlf,
    LoneCr,
    OrgLoneCr,
    Trivia,
    Preamble,
    PinnedJournal,
    Alias,
    Twin,
    Guide,
    KeepMine,
    KeepMineDeleted,
    KeepMineUndecodable,
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-store-save-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        Self(root)
    }

    fn write(&self, rel: &str, bytes: impl AsRef<[u8]>) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp = self.0.join(format!(
            ".save-fixture-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&temp, bytes).unwrap();
        fs::rename(temp, self.0.join(rel)).unwrap();
    }

    fn files(&self) -> BTreeMap<String, Vec<u8>> {
        fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, out);
                } else {
                    // Key by the `/`-separated graph-relative id, as on
                    // every platform (Windows lists `pages\\New.md`).
                    out.insert(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                        fs::read(path).unwrap(),
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(&self.0, &self.0.join("pages"), &mut files);
        visit(&self.0, &self.0.join("journals"), &mut files);
        files
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn fresh(name: &str, kind: PageKind) -> PageDto {
    PageDto {
        name: name.into(),
        kind,
        title: name.into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "test-block".into(),
            raw: "mine".into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,

        guide: false,
    }
}

#[test]
fn own_write_before_initial_load_is_in_first_snapshot() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join(".tine-test-pause-load"), b"").unwrap();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = PageId::from("pages/BeforeReady.md");
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::CreateNew,
            &fresh("BeforeReady", PageKind::Page)
        ),
        SaveOutcome::Saved(_)
    ));
    fs::remove_file(fixture.0.join(".tine-test-pause-load")).unwrap();
    let first = store.whole_graph().unwrap();
    assert!(first
        .corpus()
        .pages
        .iter()
        .any(|page| page.id == id.as_str()));
    store.close();
}

fn store_wire(outcome: SaveOutcome, doc: &PageDto) -> Result<String, String> {
    match outcome {
        SaveOutcome::Saved(rev) | SaveOutcome::Unchanged(rev) => Ok(rev.into()),
        SaveOutcome::Conflict { .. } | SaveOutcome::Deleted => Err("conflict".into()),
        SaveOutcome::ReadOnly(reason) | SaveOutcome::InvalidTarget(reason) => Err(reason),
        SaveOutcome::Twin { .. } => Err(format!(
            "\"{}\" exists as both a .md and a .org file — remove one (e.g. in Logseq) to edit it in Tine",
            doc.name
        )),
        SaveOutcome::Repeated => Err("repeated".into()),
        SaveOutcome::UnreadableOwner { file } => Err(format!("unreadable owner {}", file.as_str())),
        SaveOutcome::Io(error) => Err(error.to_string()),
        SaveOutcome::Closed => Err("store closed".into()),
        SaveOutcome::GuideEphemeral => Ok("guide-ephemeral".into()),
    }
}

fn run(case: Case) -> (Result<String, String>, BTreeMap<String, Vec<u8>>) {
    let fixture = Fixture::new();
    match case {
        Case::Matching
        | Case::Stale
        | Case::Deleted
        | Case::KeepMine
        | Case::KeepMineDeleted
        | Case::KeepMineUndecodable => {
            fixture.write("pages/Note.md", "- before\n");
        }
        Case::OrgReadOnly => fixture.write("pages/Note.org", "* a\n*** c\n"),
        Case::OrgEditable => fixture.write("pages/Note.org", "* before\n"),
        Case::Crlf => fixture.write("pages/Note.md", b"- before\r\n"),
        // K01a: lone-CR line breaks separate blocks and survive the edit.
        Case::LoneCr => fixture.write("pages/Note.md", b"- before\r- keep\r\t- child\r"),
        // K01a: a lone-CR org page is editable and keeps its terminators.
        Case::OrgLoneCr => fixture.write("pages/Note.org", b"* before\r* keep\r** child\r"),
        Case::Trivia => fixture.write("pages/Note.md", "foo:: bar\n\n\n- before\n"),
        Case::Preamble => fixture.write("pages/Note.md", "A:: 1\nB:: 2\n- before\n"),
        Case::PinnedJournal => {
            fixture.write("journals/2026_06_26.org", "* canonical\n");
            fixture.write("journals/Friday, 26-06-2026.org", "* stray\n");
        }
        Case::Alias => fixture.write("pages/Owner.md", "alias:: Alt\n- owner\n"),
        Case::Twin => {
            fixture.write("pages/Note.md", "- md\n");
            fixture.write("pages/Note.org", "* org\n");
        }
        Case::New | Case::Appeared | Case::Guide => {}
    }
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let mut doc = match case {
        Case::New | Case::Appeared => fresh("New", PageKind::Page),
        Case::Alias => fresh("Alt", PageKind::Page),
        Case::Twin => fresh("Note", PageKind::Page),
        Case::Guide => fresh("Guide", PageKind::Page),
        Case::PinnedJournal => {
            let id = PageId::from("journals/Friday, 26-06-2026.org");
            store.page(&id).unwrap().doc
        }
        Case::OrgReadOnly | Case::OrgEditable | Case::OrgLoneCr => {
            store.page(&PageId::from("pages/Note.org")).unwrap().doc
        }
        _ => store.page(&PageId::from("pages/Note.md")).unwrap().doc,
    };
    if matches!(case, Case::Guide) {
        doc.guide = true;
    }
    if matches!(case, Case::Preamble) {
        doc.pre_block = Some("A:: 1".into());
        doc.blocks.insert(
            0,
            BlockDto {
                id: "moved-property".into(),
                raw: "B:: 2".into(),
                ..Default::default()
            },
        );
    } else if !matches!(case, Case::Trivia | Case::OrgReadOnly) && doc.rev.is_some() {
        doc.blocks[0].raw = "mine".into();
    }
    match case {
        Case::Appeared => fixture.write("pages/New.md", "- external creation\n"),
        Case::Stale | Case::KeepMine => fixture.write("pages/Note.md", "- external edit\n"),
        Case::Deleted | Case::KeepMineDeleted => {
            fs::remove_file(fixture.0.join("pages/Note.md")).unwrap();
        }
        Case::KeepMineUndecodable => fixture.write("pages/Note.md", b"\xff\xfeunknown"),
        _ => {}
    }
    let force = matches!(
        case,
        Case::KeepMine | Case::KeepMineDeleted | Case::KeepMineUndecodable
    );
    let result = if doc.guide {
        store_wire(SaveOutcome::GuideEphemeral, &doc)
    } else {
        let id = match case {
            Case::PinnedJournal => PageId::from("journals/Friday, 26-06-2026.org"),
            Case::OrgReadOnly | Case::OrgEditable | Case::OrgLoneCr => {
                PageId::from("pages/Note.org")
            }
            _ => match store
                .whole_graph()
                .unwrap()
                .resolve(&doc.name, doc.kind == PageKind::Journal)
            {
                tine_store::Resolved::Existing { id, .. } | tine_store::Resolved::Absent { id } => {
                    id
                }
                tine_store::Resolved::Alias { .. } => {
                    return (Err("conflict".into()), fixture.files())
                }
            },
        };
        let base = if force {
            match store.read(&id.file(), None) {
                Ok((bytes, rev)) => {
                    if std::str::from_utf8(&bytes).is_err() {
                        return (
                            Err("stream did not contain valid UTF-8".into()),
                            fixture.files(),
                        );
                    }
                    SaveBase::Existing(rev)
                }
                Err(StoreError::NotFound) => SaveBase::CreateNew,
                Err(error) => panic!("unexpected pre-read error: {error:?}"),
            }
        } else {
            doc.rev
                .clone()
                .map(|rev| SaveBase::Existing(rev.into()))
                .unwrap_or(SaveBase::CreateNew)
        };
        store_wire(
            store.save(tine_store::EditKind::ReplacePage, &id, base, &doc),
            &doc,
        )
    };
    (result, fixture.files())
}

fn assert_save_tree(actual: &BTreeMap<String, Vec<u8>>, case: Case) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/legacy_and_store_saves_match_on_data_safety_matrix")
        .join(format!("{case:?}"));
    let mut expected = BTreeMap::new();
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                // `/`-separated like `Fixture::files` (Windows lists `\\`).
                out.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    visit(&root, &root, &mut expected);
    for (path, bytes) in &expected {
        match actual.get(path) {
            Some(found) => assert_eq!(found, bytes, "first differing path: {path}"),
            None => panic!("first differing path: {path} (missing)"),
        }
    }
    if let Some(path) = actual.keys().find(|path| !expected.contains_key(*path)) {
        panic!("first differing path: {path} (unexpected)");
    }
}

#[test]
fn legacy_and_store_saves_match_on_data_safety_matrix() {
    for case in [
        Case::New,
        Case::Appeared,
        Case::Matching,
        Case::Stale,
        Case::Deleted,
        Case::OrgReadOnly,
        Case::OrgEditable,
        Case::Crlf,
        Case::LoneCr,
        Case::OrgLoneCr,
        Case::Trivia,
        Case::Preamble,
        Case::PinnedJournal,
        Case::Alias,
        Case::Twin,
        Case::Guide,
        Case::KeepMine,
        Case::KeepMineDeleted,
        Case::KeepMineUndecodable,
    ] {
        let new = run(case);
        let expected_result = match case {
            Case::Alias => Err("conflict".to_string()),
            Case::Appeared => Err("conflict".to_string()),
            Case::Crlf => Ok("2be7206b0f37adb0".to_string()),
            Case::LoneCr => Ok("badb812c198da790".to_string()),
            Case::OrgLoneCr => Ok("3a585cba16df7a44".to_string()),
            Case::Deleted => Err("conflict".to_string()),
            Case::Guide => Ok("guide-ephemeral".to_string()),
            Case::KeepMine => Ok("eb859457eef7db1b".to_string()),
            Case::KeepMineDeleted => Ok("eb859457eef7db1b".to_string()),
            Case::KeepMineUndecodable => Err("stream did not contain valid UTF-8".to_string()),
            Case::Matching => Ok("eb859457eef7db1b".to_string()),
            Case::New => Ok("eb859457eef7db1b".to_string()),
            Case::OrgEditable => Ok("bf0ecc2919540932".to_string()),
            Case::OrgReadOnly => Err("org file is read-only (does not round-trip)".to_string()),
            Case::PinnedJournal => Ok("bf0ecc2919540932".to_string()),
            Case::Preamble => Err("refusing to move page-header property into outline content: B:: 2".to_string()),
            Case::Stale => Err("conflict".to_string()),
            Case::Trivia => Ok("7d8a07fd9b40a488".to_string()),
            Case::Twin => Err("\"Note\" exists as both a .md and a .org file — remove one (e.g. in Logseq) to edit it in Tine".to_string()),
        };
        assert_eq!(new.0, expected_result, "wire result for {case:?}");
        if matches!(case, Case::Deleted | Case::Guide) {
            assert!(new.1.is_empty(), "disk tree for {case:?}");
        } else {
            assert_save_tree(&new.1, case);
        }
        if matches!(case, Case::Appeared | Case::Stale | Case::Deleted) {
            assert_eq!(new.0, Err("conflict".into()), "{case:?}");
        }
        if matches!(case, Case::Guide) {
            assert_eq!(new.0, Ok("guide-ephemeral".into()));
        }
        match case {
            Case::OrgReadOnly => assert_eq!(
                new.0,
                Err("org file is read-only (does not round-trip)".into())
            ),
            Case::Preamble => assert!(
                new.0.as_ref().is_err_and(
                    |message| message.starts_with("refusing to move page-header property")
                ),
                "{new:?}"
            ),
            Case::Twin => assert!(
                new.0
                    .as_ref()
                    .is_err_and(|message| message.contains("exists as both")),
                "{new:?}"
            ),
            Case::KeepMineUndecodable => {
                assert_eq!(new.0, Err("stream did not contain valid UTF-8".into()))
            }
            Case::Trivia => assert_eq!(new.1["pages/Note.md"], b"foo:: bar\n\n\n- before\n"),
            Case::Crlf => assert!(new.1["pages/Note.md"].windows(2).any(|w| w == b"\r\n")),
            Case::PinnedJournal => {
                assert_eq!(new.1["journals/2026_06_26.org"], b"* canonical\n");
                assert_ne!(new.1["journals/Friday, 26-06-2026.org"], b"* stray\n");
            }
            Case::Alias => {
                // B15b: a name that is only an alias is refused, not written.
                assert_eq!(new.1["pages/Owner.md"], b"alias:: Alt\n- owner\n");
                assert!(!new.1.contains_key("pages/Alt.md"));
            }
            Case::KeepMineDeleted => assert!(new.1.contains_key("pages/Note.md")),
            _ => {}
        }
    }
}

#[test]
fn keep_mine_rechecks_the_version_read_for_the_banner_action() {
    let fixture = Fixture::new();
    fixture.write("pages/Note.md", "- original\n");
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = PageId::from("pages/Note.md");
    let mut doc = store.page(&id).unwrap().doc;
    doc.blocks[0].raw = "mine".into();
    fixture.write("pages/Note.md", "- external one\n");
    let (_, shown_rev) = store.read(&id.file(), None).unwrap();
    fixture.write("pages/Note.md", "- external two\n");
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(shown_rev),
            &doc
        ),
        SaveOutcome::Conflict { .. }
    ));
    assert_eq!(
        fs::read(fixture.0.join("pages/Note.md")).unwrap(),
        b"- external two\n"
    );
}

/// B15b: a pathless save gets its target from `WholeGraph::resolve`. A brand-new
/// namespaced name must come back `Absent` at the file the graph's
/// `:file/name-format` and preferred format name, and saving there creates it.
#[test]
fn absent_resolve_names_the_file_by_name_format_and_preferred_format() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("logseq")).unwrap();
    fs::write(
        fixture.0.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar :preferred-format \"Org\"}\n",
    )
    .unwrap();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = match store.whole_graph().unwrap().resolve("Proj/Child", false) {
        tine_store::Resolved::Absent { id } => id,
        _ => panic!("a brand-new name must resolve Absent"),
    };
    assert_eq!(id.as_str(), "pages/Proj___Child.org");
    let mut doc = fresh("Proj/Child", PageKind::Page);
    doc.format = Format::Org;
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::CreateNew,
            &doc
        ),
        SaveOutcome::Saved(_)
    ));
    let files = fixture.files();
    assert_eq!(files.keys().collect::<Vec<_>>(), ["pages/Proj___Child.org"]);
}

/// B15b: a pathless save that resolved `Absent` before the name appeared on disk
/// carries `CreateNew`; the store must refuse it and leave the new file alone.
#[test]
fn create_new_onto_a_name_that_now_exists_conflicts_and_writes_nothing() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = match store.whole_graph().unwrap().resolve("New", false) {
        tine_store::Resolved::Absent { id } => id,
        _ => panic!("a brand-new name must resolve Absent"),
    };
    fixture.write("pages/New.md", "- external creation\n");
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::CreateNew,
            &fresh("New", PageKind::Page)
        ),
        SaveOutcome::Conflict { .. }
    ));
    assert_eq!(
        fixture.files(),
        BTreeMap::from([(
            "pages/New.md".to_string(),
            b"- external creation\n".to_vec()
        )])
    );
}

#[test]
fn late_alternate_extension_twin_never_uses_its_revision_as_target_conflict() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let id = PageId::from("pages/Late.md");
    store.inject_fault(FaultPoint::TwinAfterPublish);
    assert!(matches!(
        store.save(tine_store::EditKind::ReplacePage, &id, SaveBase::CreateNew, &fresh("Late", PageKind::Page)),
        SaveOutcome::Twin { existing } if existing.as_str() == "pages/Late.org"
    ));
}

#[test]
fn hostile_page_content_is_a_refusal_not_an_io_failure() {
    let fixture = Fixture::new();
    let source = format!("- {}x{}\n", "[".repeat(513), "]".repeat(513));
    fixture.write("pages/Deep.md", &source);
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = PageId::from("pages/Deep.md");
    let (_, rev) = store.read(&id.file(), None).unwrap();
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(rev),
            &fresh("Deep", PageKind::Page)
        ),
        SaveOutcome::InvalidTarget(_)
    ));

    let mut doc = fresh("New", PageKind::Page);
    let mut nested = tine_core::model::BlockDto {
        raw: "leaf".into(),
        ..Default::default()
    };
    for _ in 0..513 {
        nested = tine_core::model::BlockDto {
            raw: "parent".into(),
            children: vec![nested],
            ..Default::default()
        };
    }
    doc.blocks = vec![nested];
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &PageId::from("pages/New.md"),
            SaveBase::CreateNew,
            &doc
        ),
        SaveOutcome::InvalidTarget(_)
    ));
}

#[test]
fn own_journal_creation_updates_day_and_published_answers() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let id = PageId::from("journals/2026_09_25.org");
    let mut doc = fresh("Sep 25th, 2026", PageKind::Journal);
    doc.format = Format::Org;
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::CreateNew,
            &doc
        ),
        SaveOutcome::Saved(_)
    ));
    assert_eq!(store.journal_id(tine_store::Day(20260925)), id);
    let view = store.whole_graph().unwrap();
    assert!(matches!(
        view.resolve("Sep 25th, 2026", true),
        tine_store::Resolved::Existing { id: found, .. } if found == id
    ));
    assert!(view
        .inventory(tine_store::InventoryScope::All)
        .0
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
    assert!(view
        .complete_page_names("Sep 25", 10)
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
}

/// B15b: a loaded duplicate-day stray saves to the id it was loaded from (the
/// #21 pin), never to the canonical file the day's name resolves to.
#[test]
fn a_loaded_stray_journal_saves_to_its_own_file() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("logseq")).unwrap();
    fs::write(
        fixture.0.join("logseq/config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fixture.write("journals/2026_06_26.md", "- canonical\n");
    fixture.write("journals/Friday, 26-06-2026.md", "- stray\n");
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let stray = PageId::from("journals/Friday, 26-06-2026.md");
    let mut doc = store.page(&stray).unwrap().doc;
    let resolved = match store.whole_graph().unwrap().resolve(&doc.name, true) {
        tine_store::Resolved::Existing { id, .. } => id,
        _ => panic!("the day must resolve to an existing journal"),
    };
    assert_ne!(resolved, stray, "the day's name must not answer the stray");
    doc.blocks[0].raw = "stray edit".into();
    let base = SaveBase::Existing(doc.rev.clone().unwrap().into());
    assert!(matches!(
        store.save(tine_store::EditKind::ReplacePage, &stray, base, &doc),
        SaveOutcome::Saved(_)
    ));
    let files = fixture.files();
    assert_eq!(files["journals/2026_06_26.md"], b"- canonical\n");
    assert_eq!(files["journals/Friday, 26-06-2026.md"], b"- stray edit\n");
}

#[test]
/// A file whose roots all sit under one tab keeps that base offset on the
/// edited line; splicing the serializer's column-0 line re-nested the child.
fn g6b_base_indented_file_keeps_its_outline_and_bytes() {
    let source = "\t- root\n\t\t- child\n\t- sibling\n";
    let fixture = Fixture::new();
    fixture.write("pages/BaseTab.md", source.as_bytes());
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = PageId::from("pages/BaseTab.md");
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    assert_eq!(doc.blocks.len(), 2, "{:?}", doc.blocks);
    doc.blocks[0].children[0].raw.push_str(" edited");
    assert!(matches!(
        store.save(
            tine_store::EditKind::SaveBlock,
            &id,
            SaveBase::Existing(read.rev),
            &doc
        ),
        SaveOutcome::Saved(_)
    ));
    let after = fs::read_to_string(fixture.0.join("pages/BaseTab.md")).unwrap();
    assert_eq!(after, "\t- root\n\t\t- child edited\n\t- sibling\n");
    let reread = store.page(&id).unwrap().doc;
    assert_eq!(reread.blocks.len(), 2, "outline changed: {after:?}");
    assert_eq!(
        reread.blocks[0].children.len(),
        1,
        "outline changed: {after:?}"
    );
    store.close();
}
