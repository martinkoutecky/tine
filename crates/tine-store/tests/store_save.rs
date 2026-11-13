//! Save matrix against values captured from v0.6.5 Graph at 9c3d7c376.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::host_faults::{abort_before, fail, Phase};
use crate::test_host_writer::{HostWriter, SaveRefused};
use crate::{
    Area, Content, EditKind, FaultPoint, FileRev, PageId, Refusal, SaveBase, Store, StoreError,
    TxOutcome, Why,
};
use tine_core::model::{BlockDto, Format, PageDto, PageKind};

type Entry = (PageId, SaveBase, PageDto, Vec<EditKind>);

/// The guarded multi-page plan these tests pin: one `save_page` step per entry,
/// in input order, handed to the host.
fn plan_all(store: &Store, entries: &[Entry]) -> Result<crate::HostOp, Why> {
    let mut tx = store.page_plan(Some(entries[0].3[0]));
    for (id, base, doc, kinds) in entries {
        tx.save_page(kinds, id, base.clone(), doc);
    }
    tx.plan()
}

fn open(fixture: &Fixture) -> (Arc<Store>, HostWriter) {
    let store = Arc::new(Store::open(&fixture.0, Default::default()).unwrap().0);
    let writer = HostWriter::start(&store);
    (store, writer)
}

#[test]
fn removing_last_org_drawer_entry_saves_only_the_edited_bytes() {
    use tine_core::block_regions::{parse, Edit};

    let _integration_guard = crate::test_transactions::integration_lock();
    for nl in ["\n", "\r\n"] {
        for tail in [
            "\n#+OWNER: retained\n* After  ",
            "\n#+BEGIN_QUOTE\nquoted body\n#+END_QUOTE\n* After  ",
            "\nBody text\n* After  ",
            "\n** Child\n:PROPERTIES:\n:id: child\n:END:\n* After  ",
            "",
        ] {
            for key in ["color", "collapsed", "id"] {
                let drawer = format!(":PROPERTIES:{nl}:{key}: true{nl}:END:{nl}");
                let source = format!(
                    "#+TITLE: Drawer test{nl}* Before  {nl}:PROPERTIES:{nl}:id: before{nl}:END:{nl}* Task{nl}{drawer}{}",
                    tail.strip_prefix('\n').unwrap_or(tail).replace('\n', nl)
                );
                let fixture = Fixture::new();
                fixture.write("pages/Drawer.org", &source);
                let (store, writer) = open(&fixture);
                let id = PageId::from("pages/Drawer.org");
                let read = store.page(&id).unwrap();
                assert!(!read.doc.read_only);
                writer
                    .save(
                        EditKind::SaveBlock,
                        &id,
                        SaveBase::Existing(read.rev),
                        &read.doc,
                    )
                    .unwrap();
                assert_eq!(
                    fs::read_to_string(fixture.0.join(id.as_str())).unwrap(),
                    source
                );
                let read = store.page(&id).unwrap();
                let mut doc = read.doc;
                let raw = &doc.blocks[1].raw;
                doc.blocks[1].raw = parse(raw, true)
                    .apply(
                        raw,
                        true,
                        Edit::Property {
                            key: key.into(),
                            value: None,
                        },
                    )
                    .unwrap();
                writer
                    .save(EditKind::SaveBlock, &id, SaveBase::Existing(read.rev), &doc)
                    .unwrap();
                let expected = source.replacen(&drawer, "", 1);
                assert_eq!(
                    fs::read_to_string(fixture.0.join(id.as_str())).unwrap(),
                    expected,
                    "key={key}, tail={tail:?}, newline={nl:?}"
                );
                let reopened = store.page(&id).unwrap().doc;
                assert_eq!(reopened.blocks[1].raw, doc.blocks[1].raw);
                store.close();
            }
        }
    }
}

#[test]
fn g6b_one_block_edit_preserves_fixture_layout() {
    let _integration_guard = crate::test_transactions::integration_lock();
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
        let (store, writer) = open(&fixture);
        let id = PageId::from(rel.as_str());
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw.push_str(" edited");
        writer
            .save(
                crate::EditKind::SaveBlock,
                &id,
                SaveBase::Existing(read.rev),
                &doc,
            )
            .unwrap();
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
    let _integration_guard = crate::test_transactions::integration_lock();
    let source = include_bytes!("../../../scripts/fixtures/g6b/graph/pages/TrailingBlanks.md");
    let fixture = Fixture::new();
    fixture.write("pages/TrailingBlanks.md", source);
    let (store, writer) = open(&fixture);
    let id = PageId::from("pages/TrailingBlanks.md");
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks[1].raw.push('\n');
    writer
        .save(
            crate::EditKind::SaveBlock,
            &id,
            SaveBase::Existing(read.rev),
            &doc,
        )
        .unwrap();
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
fn a_page_save_with_an_empty_kind_list_is_refused_before_writing() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut tx = store.page_plan(Some(EditKind::CreatePage));
        tx.save_page(
            &[],
            &PageId::from("pages/New.md"),
            SaveBase::CreateNew,
            &fresh("New", PageKind::Page),
        );
        tx.plan()
    }));
    let message = refused.expect_err("OG-RULES Rule 8: a kindless page save must not run");
    let text = message
        .downcast_ref::<&str>()
        .map(|text| text.to_string())
        .or_else(|| message.downcast_ref::<String>().cloned());
    assert!(
        text.as_deref()
            .is_some_and(|text| text.contains("OG-RULES Rule 8")),
        "{text:?}"
    );
    assert!(!fixture.0.join("pages/New.md").exists());
}

#[test]
fn page_save_transaction_crash_worker() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (Ok(root), Ok(app_data)) = (
        std::env::var("TINE_PAGE_SAVE_TX_CRASH_ROOT"),
        std::env::var("TINE_PAGE_SAVE_TX_CRASH_APP"),
    ) else {
        return;
    };
    let store = Arc::new(Store::open(Path::new(&root), Default::default()).unwrap().0);
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
                vec![crate::EditKind::ReplacePage],
            )
        })
        .collect();
    // A kill between the first page's write and the second's.
    abort_before(Phase::PageRename, 1);
    let host = crate::PageHost::start_for_tests(&store, Path::new(&app_data)).unwrap();
    let _ = host.apply(plan_all(&store, &entries).unwrap());
    panic!("page save transaction crash fault did not abort");
}

#[test]
fn a_page_save_transaction_crash_between_steps_keeps_earlier_disk_write() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    for name in ["A", "B", "C"] {
        fixture.write(&format!("pages/{name}.md"), format!("- old {name}\n"));
    }
    let app_data = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(tine_store::test_binary::own_test_binary())
        .arg("--exact")
        .arg("test_transactions::store_save::page_save_transaction_crash_worker")
        .env("TINE_PAGE_SAVE_TX_CRASH_ROOT", &fixture.0)
        .env("TINE_PAGE_SAVE_TX_CRASH_APP", app_data.path())
        .output()
        .unwrap();
    assert!(!output.status.success(), "fault must abort between steps");
    assert!(String::from_utf8(fixture.files()["pages/A.md"].clone())
        .unwrap()
        .contains("new A"));
    assert_eq!(fixture.files()["pages/B.md"], b"- old B\n");
    assert_eq!(fixture.files()["pages/C.md"], b"- old C\n");
    // The host promises no multi-page rollback: the operation is completed
    // from its draft at relaunch. The graph opens, and every page shows the
    // operation's bytes, A's earlier disk write among them.
    let reopened = Arc::new(Store::open(&fixture.0, Default::default()).unwrap().0);
    let host = crate::PageHost::start_for_tests(&reopened, app_data.path()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while ["B", "C"]
        .iter()
        .any(|name| !fixture.files()[&format!("pages/{name}.md")].starts_with(b"- new"))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "relaunch did not complete the operation: {:?}",
            fixture.files()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let view = reopened.whole_graph().unwrap();
    assert!(view.unreadable_files().is_empty());
    for name in ["A", "B", "C"] {
        let read = reopened
            .page(&PageId::from(format!("pages/{name}.md")))
            .unwrap();
        assert_eq!(
            read.doc.blocks[0].raw,
            format!("new {name}"),
            "after relaunch"
        );
    }
    drop((view, host));
    reopened.close();
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
                    // A save in flight (a relaunched host completing its
                    // operation) renames its temp file away between the
                    // listing and the read: that file is gone, not unread.
                    let bytes = match fs::read(&path) {
                        Ok(bytes) => bytes,
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(err) => panic!("read {}: {err}", path.display()),
                    };
                    // Key by the `/`-separated graph-relative id, as on
                    // every platform (Windows lists `pages\\New.md`).
                    out.insert(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                        bytes,
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
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    fs::write(fixture.0.join(".tine-test-pause-load"), b"").unwrap();
    let (store, writer) = open(&fixture);
    let id = PageId::from("pages/BeforeReady.md");
    writer
        .save(
            crate::EditKind::ReplacePage,
            &id,
            SaveBase::CreateNew,
            &fresh("BeforeReady", PageKind::Page),
        )
        .unwrap();
    fs::remove_file(fixture.0.join(".tine-test-pause-load")).unwrap();
    let first = store.whole_graph().unwrap();
    assert!(first
        .corpus(None, false)
        .pages
        .iter()
        .any(|page| page.id == id.as_str()));
    store.close();
}

fn store_wire(
    outcome: Result<FileRev, SaveRefused>,
    id: &PageId,
    doc: &PageDto,
) -> Result<String, String> {
    let twin = || {
        format!(
            "\"{}\" exists as both a .md and a .org file — remove one (e.g. in Logseq) to edit it in Tine",
            doc.name
        )
    };
    match outcome {
        Ok(rev) => Ok(rev.into()),
        Err(SaveRefused::Host(crate::ApplyRefusal::Changed)) => Err("conflict".into()),
        Err(SaveRefused::Host(refusal)) => Err(format!("{refusal:?}")),
        Err(SaveRefused::Planned(why)) => Err(match why {
            Why::Conflict {
                file,
                disk: Some(_),
            } if file != id.file() => twin(),
            Why::Conflict { .. } => "conflict".into(),
            Why::Refused(Refusal::ReadOnly(reason) | Refusal::InvalidTarget(reason)) => reason,
            Why::Refused(Refusal::Twin { .. }) => twin(),
            Why::Refused(refusal) => format!("{refusal:?}"),
            Why::Failed(error) => error.to_string(),
        }),
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
    let (store, writer) = open(&fixture);
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
        Ok("guide-ephemeral".to_string())
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
                crate::Resolved::Existing { id, .. } | crate::Resolved::Absent { id } => id,
                crate::Resolved::Alias { .. } => return (Err("conflict".into()), fixture.files()),
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
                Err(StoreError::NotFound(_)) => SaveBase::CreateNew,
                Err(error) => panic!("unexpected pre-read error: {error:?}"),
            }
        } else {
            doc.rev
                .clone()
                .map(|rev| SaveBase::Existing(rev.into()))
                .unwrap_or(SaveBase::CreateNew)
        };
        store_wire(
            writer.save(crate::EditKind::ReplacePage, &id, base, &doc),
            &id,
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
    let _integration_guard = crate::test_transactions::integration_lock();
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
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    fixture.write("pages/Note.md", "- original\n");
    let (store, writer) = open(&fixture);
    let id = PageId::from("pages/Note.md");
    let mut doc = store.page(&id).unwrap().doc;
    doc.blocks[0].raw = "mine".into();
    fixture.write("pages/Note.md", "- external one\n");
    let (_, shown_rev) = store.read(&id.file(), None).unwrap();
    fixture.write("pages/Note.md", "- external two\n");
    assert!(matches!(
        writer.save(
            crate::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(shown_rev),
            &doc
        ),
        Err(SaveRefused::Planned(Why::Conflict { .. }))
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
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("logseq")).unwrap();
    fs::write(
        fixture.0.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar :preferred-format \"Org\"}\n",
    )
    .unwrap();
    let (store, writer) = open(&fixture);
    let id = match store.whole_graph().unwrap().resolve("Proj/Child", false) {
        crate::Resolved::Absent { id } => id,
        _ => panic!("a brand-new name must resolve Absent"),
    };
    assert_eq!(id.as_str(), "pages/Proj___Child.org");
    let mut doc = fresh("Proj/Child", PageKind::Page);
    doc.format = Format::Org;
    writer
        .save(crate::EditKind::ReplacePage, &id, SaveBase::CreateNew, &doc)
        .unwrap();
    let files = fixture.files();
    assert_eq!(files.keys().collect::<Vec<_>>(), ["pages/Proj___Child.org"]);
}

/// B15b: a pathless save that resolved `Absent` before the name appeared on disk
/// carries `CreateNew`; the store must refuse it and leave the new file alone.
#[test]
fn create_new_onto_a_name_that_now_exists_conflicts_and_writes_nothing() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    let (store, writer) = open(&fixture);
    let id = match store.whole_graph().unwrap().resolve("New", false) {
        crate::Resolved::Absent { id } => id,
        _ => panic!("a brand-new name must resolve Absent"),
    };
    fixture.write("pages/New.md", "- external creation\n");
    assert!(matches!(
        writer.save(
            crate::EditKind::ReplacePage,
            &id,
            SaveBase::CreateNew,
            &fresh("New", PageKind::Page)
        ),
        Err(SaveRefused::Planned(Why::Conflict { .. }))
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
fn hostile_page_content_is_a_refusal_not_an_io_failure() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    let source = format!("- {}x{}\n", "[".repeat(513), "]".repeat(513));
    fixture.write("pages/Deep.md", &source);
    let (store, writer) = open(&fixture);
    let id = PageId::from("pages/Deep.md");
    let (_, rev) = store.read(&id.file(), None).unwrap();
    assert!(matches!(
        writer.save(
            crate::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(rev),
            &fresh("Deep", PageKind::Page)
        ),
        Err(SaveRefused::Planned(Why::Refused(Refusal::InvalidTarget(
            _
        ))))
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
        writer.save(
            crate::EditKind::ReplacePage,
            &PageId::from("pages/New.md"),
            SaveBase::CreateNew,
            &doc
        ),
        Err(SaveRefused::Planned(Why::Refused(Refusal::InvalidTarget(
            _
        ))))
    ));
}

#[test]
fn own_journal_creation_updates_day_and_published_answers() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    let (store, writer) = open(&fixture);
    store.whole_graph().unwrap();
    let id = PageId::from("journals/2026_09_25.org");
    let mut doc = fresh("Sep 25th, 2026", PageKind::Journal);
    doc.format = Format::Org;
    writer
        .save(crate::EditKind::ReplacePage, &id, SaveBase::CreateNew, &doc)
        .unwrap();
    assert_eq!(store.journal_id(crate::Day(20260925), false), id);
    let view = store.whole_graph().unwrap();
    assert!(matches!(
        view.resolve("Sep 25th, 2026", true),
        crate::Resolved::Existing { id: found, .. } if found == id
    ));
    assert!(view
        .inventory(crate::InventoryScope::All)
        .0
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
    assert!(view
        .complete_page_names("Sep 25", 10, None, false, None)
        .into_iter()
        .map(|(page, _)| page)
        .collect::<Vec<_>>()
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
}

/// B15b: a loaded duplicate-day stray saves to the id it was loaded from (the
/// #21 pin), never to the canonical file the day's name resolves to.
#[test]
fn a_loaded_stray_journal_saves_to_its_own_file() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("logseq")).unwrap();
    fs::write(
        fixture.0.join("logseq/config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fixture.write("journals/2026_06_26.md", "- canonical\n");
    fixture.write("journals/Friday, 26-06-2026.md", "- stray\n");
    let (store, writer) = open(&fixture);
    let stray = PageId::from("journals/Friday, 26-06-2026.md");
    let mut doc = store.page(&stray).unwrap().doc;
    let resolved = match store.whole_graph().unwrap().resolve(&doc.name, true) {
        crate::Resolved::Existing { id, .. } => id,
        _ => panic!("the day must resolve to an existing journal"),
    };
    assert_ne!(resolved, stray, "the day's name must not answer the stray");
    doc.blocks[0].raw = "stray edit".into();
    let base = SaveBase::Existing(doc.rev.clone().unwrap().into());
    writer
        .save(crate::EditKind::ReplacePage, &stray, base, &doc)
        .unwrap();
    let files = fixture.files();
    assert_eq!(files["journals/2026_06_26.md"], b"- canonical\n");
    assert_eq!(files["journals/Friday, 26-06-2026.md"], b"- stray edit\n");
}

#[test]
/// A file whose roots all sit under one tab keeps that base offset on the
/// edited line; splicing the serializer's column-0 line re-nested the child.
fn g6b_base_indented_file_keeps_its_outline_and_bytes() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let source = "\t- root\n\t\t- child\n\t- sibling\n";
    let fixture = Fixture::new();
    fixture.write("pages/BaseTab.md", source.as_bytes());
    let (store, writer) = open(&fixture);
    let id = PageId::from("pages/BaseTab.md");
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    assert_eq!(doc.blocks.len(), 2, "{:?}", doc.blocks);
    doc.blocks[0].children[0].raw.push_str(" edited");
    writer
        .save(
            crate::EditKind::SaveBlock,
            &id,
            SaveBase::Existing(read.rev),
            &doc,
        )
        .unwrap();
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

/// A plan reads its pages in input order and refuses the whole operation on a
/// stale base, before anything reaches disk; replanned from the current base,
/// the host writes every page.
#[test]
fn a_page_plan_keeps_order_and_reports_preflight_conflict_without_writes() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    fixture.write("pages/A.md", "- old A\n");
    fixture.write("pages/B.md", "- old B\n");
    let (store, writer) = open(&fixture);
    let a = PageId::from("pages/A.md");
    let b = PageId::from("pages/B.md");
    let read_a = store.page(&a).unwrap();
    let read_b = store.page(&b).unwrap();
    let mut doc_a = read_a.doc;
    let mut doc_b = read_b.doc;
    doc_a.blocks[0].raw = "new A".into();
    doc_b.blocks[0].raw = "new B".into();
    let mut entries = vec![
        (
            a.clone(),
            SaveBase::Existing(read_a.rev),
            doc_a,
            vec![crate::EditKind::ReplacePage],
        ),
        (
            b.clone(),
            SaveBase::Existing(read_b.rev),
            doc_b,
            vec![crate::EditKind::ReplacePage],
        ),
    ];
    fixture.write("pages/B.md", "- external B\n");
    assert!(matches!(
        plan_all(&store, &entries),
        Err(Why::Conflict { file, .. }) if file == b.file()
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
    entries[1].1 = SaveBase::Existing(new_b_rev);
    let op = plan_all(&store, &entries).unwrap();
    let order: Vec<_> = op.pages.iter().map(|(page, ..)| page.clone()).collect();
    assert_eq!(order, [a, b]);
    writer.apply(op).unwrap();
    assert!(String::from_utf8(fixture.files()["pages/A.md"].clone())
        .unwrap()
        .contains("new A"));
    assert!(String::from_utf8(fixture.files()["pages/B.md"].clone())
        .unwrap()
        .contains("new B"));
}

#[test]
fn a_page_plan_naming_a_file_twice_has_its_own_family() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    fixture.write("pages/A.md", "- old\n");
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = PageId::from("pages/A.md");
    let read = store.page(&id).unwrap();
    let entry = (
        id,
        SaveBase::Existing(read.rev),
        read.doc,
        vec![crate::EditKind::ReplacePage],
    );
    assert!(matches!(
        plan_all(&store, &[entry.clone(), entry]),
        Err(Why::Refused(Refusal::RepeatedFile(_)))
    ));
}

/// The generic undo stays for non-page transactions: an undo that fails names
/// the file it left, so the user can recover it.
#[test]
fn a_transaction_rollback_failure_names_recovery_location() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let a = store.file_id(Area::Assets, "A.txt").unwrap();
    let b = store.file_id(Area::Assets, "B.txt").unwrap();
    store.inject_fault(FaultPoint::MidStepIoAt(1));
    store.inject_fault(FaultPoint::UndoWithdrawalIo);
    let mut tx = store.transaction(None);
    tx.create(&a, Content::Bytes(b"first".to_vec()));
    tx.create(&b, Content::Bytes(b"second".to_vec()));
    let outcome = tx.commit();
    assert!(
        matches!(&outcome, TxOutcome::NotCommitted {
        step: 1, why: Why::Failed(_), rollback, ..
    } if format!("{:?}", rollback.undo_failed).contains("assets/B.txt")),
        "{outcome:?}"
    );
}

/// G26 (storage.qnt mutant MQ, I-2): a save whose bytes equal the disk reports
/// success, and success retires the editor's crash copy. Those bytes may be
/// another program's unsynced write of the same text (Syncthing delivering the
/// user's own edit), so the save must make them durable before it answers. The
/// host does not skip equal bytes: they take the whole temp, rename and
/// directory-sync recipe, and a failure of either step is not a success.
#[test]
fn an_equal_bytes_host_save_makes_the_bytes_durable_or_does_not_report_success() {
    let _integration_guard = crate::test_transactions::integration_lock();
    for phase in [Phase::PageTemp, Phase::PageRename, Phase::PageSync] {
        let fixture = Fixture::new();
        fixture.write("pages/Equal.md", "- mine\n");
        let store = Arc::new(Store::open(&fixture.0, Default::default()).unwrap().0);
        let id = PageId::from("pages/Equal.md");
        let read = store.page(&id).unwrap();
        fail(phase, std::io::ErrorKind::Other, 1_000);
        let writer = HostWriter::start(&store);
        let outcome = writer.save(
            crate::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &read.doc,
        );
        assert!(
            matches!(outcome, Err(SaveRefused::Host(_))),
            "an equal-bytes save reported success past a failed {phase:?} (G26, mutant MQ): {outcome:?}"
        );
        assert_eq!(
            fs::read(fixture.0.join("pages/Equal.md")).unwrap(),
            b"- mine\n"
        );
    }
}

/// G26 on the live non-page writer: an equal `Replace` reports `Unchanged` only
/// after syncing the bytes at the path.
#[test]
fn an_unchanged_non_page_replace_makes_the_equal_bytes_durable() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("assets")).unwrap();
    fixture.write("assets/equal.txt", "same");
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let id = store.file_id(Area::Assets, "equal.txt").unwrap();
    let (_, rev) = store.read(&id, None).unwrap();
    crate::directory_durability::take_synced_directories();
    let mut tx = store.transaction(None);
    tx.replace(&id, rev, b"same".to_vec());
    assert!(matches!(
        tx.commit(),
        TxOutcome::Committed { steps, .. } if matches!(steps[0], crate::StepResult::Unchanged { .. })
    ));
    let synced = crate::directory_durability::take_synced_directories();
    assert!(
        synced.iter().any(|dir| dir.ends_with("assets")),
        "an Unchanged replace reported success without syncing its directory (G26, mutant MQ): {synced:?}"
    );
}
