//! Save migration matrix: run v0.6.5 and Store against separate identical graphs.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::model::Graph;
use tine_store::{PageId, SaveBase, SaveOutcome, Store, StoreError};

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
        fs::write(self.0.join(rel), bytes).unwrap();
    }

    fn files(&self) -> BTreeMap<String, Vec<u8>> {
        fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, out);
                } else {
                    out.insert(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
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
        path: None,
        guide: false,
    }
}

fn legacy_wire(result: std::io::Result<String>) -> Result<String, String> {
    result.map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            "conflict".into()
        } else {
            error.to_string()
        }
    })
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
        SaveOutcome::Io(error) => Err(error.to_string()),
        SaveOutcome::Closed => Err("store closed".into()),
        SaveOutcome::GuideEphemeral => Ok("guide-ephemeral".into()),
    }
}

fn run(case: Case, use_store: bool) -> (Result<String, String>, BTreeMap<String, Vec<u8>>) {
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
    let graph = Arc::new(Graph::open(&fixture.0));
    let store = Store::from_legacy(Arc::clone(&graph));
    let mut doc = match case {
        Case::New | Case::Appeared => fresh("New", PageKind::Page),
        Case::Alias => fresh("Alt", PageKind::Page),
        Case::Twin => fresh("Note", PageKind::Page),
        Case::Guide => fresh("Guide", PageKind::Page),
        Case::PinnedJournal => {
            let id = PageId::from("journals/Friday, 26-06-2026.org");
            store.page(&id).unwrap().doc
        }
        Case::OrgReadOnly | Case::OrgEditable => {
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
    let result = if use_store {
        if doc.guide {
            store_wire(SaveOutcome::GuideEphemeral, &doc)
        } else {
            let id = match store.target_for_save(&doc) {
                Ok(id) => id,
                Err(outcome) => return (store_wire(outcome, &doc), fixture.files()),
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
            store_wire(store.save(&id, base, &doc), &doc)
        }
    } else if force {
        legacy_wire(graph.force_save_page(&doc))
    } else {
        legacy_wire(graph.save_page(&doc, doc.rev.as_deref()))
    };
    (result, fixture.files())
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
        let old = run(case, false);
        let new = run(case, true);
        assert_eq!(new.0, old.0, "wire result for {case:?}");
        assert_eq!(new.1, old.1, "disk bytes for {case:?}");
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
                assert_eq!(new.1["pages/Owner.md"], b"alias:: Alt\n- owner\n");
                assert!(new.1.contains_key("pages/Alt.md"));
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
    let store = Store::from_legacy(Arc::new(Graph::open(&fixture.0)));
    let id = PageId::from("pages/Note.md");
    let mut doc = store.page(&id).unwrap().doc;
    doc.blocks[0].raw = "mine".into();
    fixture.write("pages/Note.md", "- external one\n");
    let (_, shown_rev) = store.read(&id.file(), None).unwrap();
    fixture.write("pages/Note.md", "- external two\n");
    assert!(matches!(
        store.save(&id, SaveBase::Existing(shown_rev), &doc),
        SaveOutcome::Conflict { .. }
    ));
    assert_eq!(
        fs::read(fixture.0.join("pages/Note.md")).unwrap(),
        b"- external two\n"
    );
}
