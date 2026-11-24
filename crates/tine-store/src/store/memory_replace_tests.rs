//! `WholeGraph::replace_files` (browser core slice 5a): an edit of a graph
//! built from files answers as `from_files` of the resulting files answers,
//! re-parses and re-captures only the changed pages, and a refused edit
//! leaves the view as it was. The page serializer it shares with the desktop
//! host gives the host's bytes.
use super::subgraph_tests::answers;
use crate::model::file_source::MEMORY_ROOT;
use crate::model::unchanged_save::take_parses;
use crate::{ChangeKind, FileId, FileRev, OpenOptions, PageId, Store, WatchMode, WholeGraph};
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tine_core::model::PageKind;

pub(super) type Files = BTreeMap<String, Vec<u8>>;

/// A config and `pages` pages, each linking the next, plus one journal.
pub(super) fn graph_files(pages: usize) -> Files {
    let mut files = Files::new();
    files.insert(
        "logseq/config.edn".into(),
        b"{:journal/page-title-format \"yyyy-MM-dd\"}\n".to_vec(),
    );
    for i in 0..pages {
        let text = format!("- page {i} links [[P{}]]\n  - child #tag{}\n", i + 1, i % 3);
        files.insert(format!("pages/P{i}.md"), text.into_bytes());
    }
    files.insert(
        "journals/2026_11_23.md".into(),
        b"- TODO see [[P1]]\n".to_vec(),
    );
    files
}

pub(super) fn open(files: &Files) -> WholeGraph {
    let files = (files.iter()).map(|(rel, bytes)| (rel.clone(), bytes.clone()));
    WholeGraph::from_files(files.collect(), Vec::new(), true)
        .unwrap()
        .0
}

/// `changes` applied to `files`, as the browser hands them over.
pub(super) fn apply(
    files: &mut Files,
    changes: &[(&str, Option<&str>)],
) -> Vec<(String, Option<Vec<u8>>)> {
    (changes.iter())
        .map(|(rel, text)| {
            match text {
                Some(text) => files.insert(rel.to_string(), text.as_bytes().to_vec()),
                None => files.remove(*rel),
            };
            (rel.to_string(), text.map(|text| text.as_bytes().to_vec()))
        })
        .collect()
}

/// The view answers as a fresh open of `files` answers.
pub(super) fn assert_opens_alike(view: &WholeGraph, files: &Files) {
    let fresh = open(files);
    let (got, want) = (
        answers(view, MEMORY_ROOT, false),
        answers(&fresh, MEMORY_ROOT, false),
    );
    for (got, want) in got.iter().zip(&want) {
        assert_eq!(got, want);
    }
    assert_eq!(got.len(), want.len());
    let meta = |view: &WholeGraph| format!("{:?}", view.config().config);
    assert_eq!(meta(view), meta(&fresh));
    for rel in files.keys().filter(|rel| rel.ends_with(".md")) {
        let page = |view: &WholeGraph| view.page(&PageId::from(rel.as_str())).map(|dto| dto.blocks);
        assert_eq!(
            format!("{:?}", page(view)),
            format!("{:?}", page(&fresh)),
            "{rel}"
        );
    }
}

/// The parsed documents two views share, by pointer.
fn shared_documents(a: &WholeGraph, b: &WholeGraph) -> usize {
    let (a, b) = (a.test_read_snapshot(), b.test_read_snapshot());
    let before: Vec<_> = a
        .physical_pages
        .iter()
        .map(|(_, doc)| Arc::clone(doc))
        .collect();
    (b.physical_pages.iter())
        .filter(|(_, doc)| before.iter().any(|old| Arc::ptr_eq(old, doc)))
        .count()
}

/// Slice 5a proof 1, and the I-25 unit cost: a replace of one page in a
/// 60-page graph parses that page once and re-captures it alone (every other
/// document is the previous capture's, shared), for a 1-block and a
/// 60-block page.
#[test]
fn a_replace_of_one_page_recaptures_only_that_page() {
    for blocks in [1, 60] {
        let mut files = graph_files(60);
        let mut view = open(&files);
        let before = view.clone_without_memory();
        let text: String = (0..blocks)
            .map(|i| format!("- edited {i} [[P9]]\n"))
            .collect();
        let changes = apply(&mut files, &[("pages/P7.md", Some(&text))]);
        take_parses();
        let started = std::time::Instant::now();
        let pages = view.replace_files(changes).unwrap().pages;
        let (elapsed, parses) = (started.elapsed(), take_parses());
        eprintln!("replace of a {blocks}-block page: {parses} parse(s), {elapsed:?}");
        assert_eq!(parses, 1, "{blocks} blocks");
        assert_eq!(
            pages,
            vec![(
                FileId::from("pages/P7.md".to_owned()),
                ChangeKind::Modified,
                PageKind::Page,
                "P7".to_owned()
            )]
        );
        let total = view.test_read_snapshot().physical_pages.len();
        assert_eq!(total, 61);
        assert_eq!(
            shared_documents(&before, &view),
            total - 1,
            "{blocks} blocks"
        );
        assert_eq!(view.rev().0, before.rev().0 + 1);
        assert_opens_alike(&view, &files);
    }
}

/// Slice 5a proof 2: a deletion, a creation in a new directory, and the
/// deletion of a directory's last file (which takes the directory with it,
/// so a file may then take its name).
#[test]
fn delete_and_add_work() {
    let mut files = graph_files(5);
    files.insert("pages/sub/Only.md".into(), b"- only [[P1]]\n".to_vec());
    let mut view = open(&files);
    let changes = apply(
        &mut files,
        &[
            ("pages/P3.md", None),
            ("pages/new/Fresh.md", Some("- fresh [[P1]]\n")),
            ("pages/sub/Only.md", None),
        ],
    );
    let pages = view.replace_files(changes).unwrap().pages;
    let mut kinds: Vec<_> = (pages.iter())
        .map(|(id, kind, _, _)| (id.as_str(), *kind))
        .collect();
    kinds.sort_by_key(|(id, _)| *id);
    assert_eq!(
        kinds,
        [
            ("pages/P3.md", ChangeKind::Removed),
            ("pages/new/Fresh.md", ChangeKind::Created),
            ("pages/sub/Only.md", ChangeKind::Removed),
        ]
    );
    assert_opens_alike(&view, &files);
    // `pages/sub` went with its last file: a file may now hold that name.
    let changes = apply(&mut files, &[("pages/sub", Some("not a page"))]);
    view.replace_files(changes).unwrap();
    assert_opens_alike(&view, &files);
    // A deleted page can come back.
    let changes = apply(&mut files, &[("pages/P3.md", Some("- back\n"))]);
    let pages = view.replace_files(changes).unwrap().pages;
    assert_eq!(pages[0].1, ChangeKind::Created);
    assert_opens_alike(&view, &files);
}

/// Slice 5a proof 3: a config change applies as an open reads it: a
/// journal title format (renames every journal), a hidden directory, and
/// the config's removal (the defaults, as an open without one).
#[test]
fn a_config_change_applies() {
    let mut files = graph_files(4);
    files.insert("pages/secret/Hidden.md".into(), b"- hush\n".to_vec());
    let mut view = open(&files);
    let config = "{:journal/page-title-format \"MMM do, yyyy\" :hidden [\"pages/secret\"]}\n";
    let changes = apply(&mut files, &[("logseq/config.edn", Some(config))]);
    let replaced = view.replace_files(changes).unwrap();
    let config_file = (
        FileId::from("logseq/config.edn".to_owned()),
        ChangeKind::Modified,
        Some(FileRev::from_bytes(config.as_bytes())),
    );
    assert_eq!(replaced.files, vec![config_file]);
    assert_eq!(replaced.graph_rev, view.rev());
    assert_eq!(
        format!("{:?}", view.resolve("Nov 23rd, 2026", true)),
        format!("{:?}", open(&files).resolve("Nov 23rd, 2026", true))
    );
    assert_opens_alike(&view, &files);
    assert!(!format!("{:?}", view.inventory(crate::InventoryScope::All)).contains("Hidden"));
    let changes = apply(&mut files, &[("logseq/config.edn", None)]);
    view.replace_files(changes).unwrap();
    assert!(view.config().problem.is_none());
    // A removal keeps the settings in force (contract config-live-reload §2):
    // the page stays hidden here, while a reopen takes the defaults.
    assert!(!format!("{:?}", view.inventory(crate::InventoryScope::All)).contains("Hidden"));
    assert!(format!("{:?}", open(&files).inventory(crate::InventoryScope::All)).contains("Hidden"));
    // A config that is not EDN is read as an open reads it, too.
    let changes = apply(&mut files, &[("logseq/config.edn", Some("{:hidden [\n"))]);
    view.replace_files(changes).unwrap();
    assert_opens_alike(&view, &files);
}

/// Slice 5a proof 4: every refused change leaves the view answering as
/// before, at the same revision, and the next edit still applies.
#[test]
fn refused_changes_leave_the_graph_unchanged() {
    let mut files = graph_files(6);
    files.insert("pages/sub/Only.md".into(), b"- only\n".to_vec());
    let mut view = open(&files);
    let before = answers(&view, MEMORY_ROOT, true);
    let rev = view.rev().0;
    let edit = Some("- edited\n");
    // Nesting past the parse input's 128 levels.
    let deep: String = (0..200)
        .map(|depth| format!("{}- d\n", "  ".repeat(depth)))
        .collect();
    let refused: &[(&[(&str, Option<&str>)], io::ErrorKind)] = &[
        (&[("pages//P1.md", edit)], io::ErrorKind::InvalidInput),
        (&[("/pages/P1.md", edit)], io::ErrorKind::InvalidInput),
        (&[("pages/./P1.md", edit)], io::ErrorKind::InvalidInput),
        (&[("pages/P1.md/", edit)], io::ErrorKind::InvalidInput),
        (
            &[("pages/P1.md", edit), ("pages/P1.md", None)],
            io::ErrorKind::InvalidInput,
        ),
        // A new key below a file, or naming a directory.
        (&[("pages/P1.md/x.md", edit)], io::ErrorKind::InvalidInput),
        (&[("pages/sub", edit)], io::ErrorKind::InvalidInput),
        (&[("pages", None)], io::ErrorKind::InvalidInput),
        // Ancestor and descendant in one batch, either order.
        (
            &[("pages/a", edit), ("pages/a/b.md", edit)],
            io::ErrorKind::InvalidInput,
        ),
        (
            &[("pages/a/b.md", edit), ("pages/a", edit)],
            io::ErrorKind::InvalidInput,
        ),
        (&[("pages/Missing.md", None)], io::ErrorKind::NotFound),
        // A valid edit refused with the batch it came in.
        (
            &[("pages/P2.md", edit), ("pages/Bad.md", Some(&deep))],
            io::ErrorKind::InvalidData,
        ),
        // Layouts an open refuses: `assets` a file, a pages directory
        // through a file.
        (
            &[("pages/P2.md", edit), ("assets", edit)],
            io::ErrorKind::InvalidInput,
        ),
        (
            &[
                ("pages/P2.md", edit),
                (
                    "logseq/config.edn",
                    Some("{:pages-directory \"pages/P1.md/x\"}"),
                ),
            ],
            io::ErrorKind::InvalidInput,
        ),
    ];
    for (changes, kind) in refused {
        let mut scratch = files.clone();
        let error = view
            .replace_files(apply(&mut scratch, changes))
            .unwrap_err();
        assert_eq!(error.kind(), *kind, "{changes:?}: {error}");
        assert_eq!(answers(&view, MEMORY_ROOT, true), before, "{changes:?}");
        assert_eq!(view.rev().0, rev, "{changes:?}");
    }
    let invalid_utf8 = vec![("pages/P2.md".to_owned(), Some(b"- \xff\n".to_vec()))];
    let error = view.replace_files(invalid_utf8).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(answers(&view, MEMORY_ROOT, true), before);
    // After a layout refusal the restored files still take an edit.
    let changes = apply(&mut files, &[("pages/P2.md", edit)]);
    view.replace_files(changes).unwrap();
    assert_opens_alike(&view, &files);
}

/// A view not built from files is not replaced.
#[test]
fn only_a_graph_built_from_files_is_replaced() {
    let mut view = open(&graph_files(1)).clone_without_memory();
    let error = view
        .replace_files(vec![("pages/P0.md".into(), None)])
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
}

/// Slice 5a task 3: file times given at construction answer
/// `page_modified`; a written file takes the time of its write.
#[test]
fn file_times_answer_page_modified() {
    let files = graph_files(2);
    let at = |secs| Some(UNIX_EPOCH + Duration::from_secs(secs));
    let all: Vec<_> = (files.iter())
        .map(|(rel, bytes)| (rel.clone(), bytes.clone()))
        .collect();
    let times = vec![("pages/P1.md".to_owned(), at(1_700_000_000))];
    let (mut view, _) = WholeGraph::from_files(all.clone(), times.clone(), true).unwrap();
    // A time given twice, or for no file, is refused, whether or not the
    // caller could represent it (round 3, finding 4).
    for times in [
        vec![times[0].clone(), times[0].clone()],
        vec![("pages/None.md".to_owned(), at(1))],
        vec![("pages/None.md".to_owned(), None)],
        vec![("pages/P1.md".to_owned(), None), times[0].clone()],
    ] {
        let error = WholeGraph::from_files(all.clone(), times, true)
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
    }
    // A time the caller cannot represent, for a file there, is no time.
    let given = vec![("pages/P1.md".to_owned(), None)];
    let (unrepresented, _) = WholeGraph::from_files(all.clone(), given, false).unwrap();
    assert!(unrepresented.page_modified().is_empty());
    assert_eq!(
        view.page_modified(),
        vec![("pages/P1.md".to_owned(), 1_700_000_000)]
    );
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    view.replace_files(vec![("pages/P0.md".into(), Some(b"- new\n".to_vec()))])
        .unwrap();
    let modified: BTreeMap<_, _> = view.page_modified().into_iter().collect();
    assert_eq!(modified["pages/P1.md"], 1_700_000_000);
    assert!(modified["pages/P0.md"] >= started, "{modified:?}");
    assert!(
        open(&files).page_modified().is_empty(),
        "no times, as before"
    );
}

/// Slice 5a proof 5: the bytes a memory graph's submit writes are the
/// desktop page host's bytes for the same DTO over the same old text, on
/// Markdown and Org: both go through the store's one page serializer.
#[test]
fn submit_bytes_equal_the_desktop_hosts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let sources = [
        ("logseq/config.edn", "{}\n"),
        ("pages/Notes.md", "title:: Notes\n\n- first [[Org Page]]\n  id:: 6566c5a4-0000-4000-8000-000000000001\n- second\n  - child\n"),
        ("pages/Org Page.org", "#+title: Org Page\n* first heading\n** nested\n* second heading :tag:\n"),
    ];
    for (rel, text) in sources {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let options = OpenOptions {
        approved_external_assets: None,
        watch: WatchMode::Poll,
        launch_checkpoint: None,
    };
    let (store, _, _) = Store::open(root, options).unwrap();
    let files = sources
        .iter()
        .map(|(rel, text)| (rel.to_string(), text.as_bytes().to_vec()));
    let (mut memory, _) = WholeGraph::from_files(files.collect(), Vec::new(), true).unwrap();
    for (rel, old) in &sources[1..] {
        let id = PageId::from(*rel);
        let mut dto = memory.page(&id).unwrap();
        dto.blocks[0].raw.push_str(" edited");
        dto.blocks.remove(1);
        let (desktop, _) = store
            .page_plan(None)
            .serialize_host_input(&id.file(), &dto, Some(old.as_bytes()))
            .unwrap();
        let rev = memory.rev().0;
        let (browser, change) = memory.submit_page(&id, &dto, true).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&browser),
            String::from_utf8_lossy(&desktop)
        );
        assert!(
            String::from_utf8_lossy(&browser).contains("edited"),
            "{rel}"
        );
        // Written: the page reads as the submitted text, one revision on.
        assert_eq!(change.graph_rev, memory.rev());
        assert_eq!(memory.rev().0, rev + 1);
        assert_eq!(memory.memory_page_bytes(&id), Some(browser.clone()));
        assert!(memory.page(&id).unwrap().blocks[0].raw.ends_with(" edited"));
        // A creation: no old text on either side.
        let fresh = PageId::from(rel.replace("pages/", "pages/New "));
        let (desktop, _) = store
            .page_plan(None)
            .serialize_host_input(&fresh.file(), &dto, None)
            .unwrap();
        assert_eq!(memory.submit_page(&fresh, &dto, true).unwrap().0, desktop);
    }
    // Refusals are the host's and write nothing: a Guide DTO, a path that
    // is no page, a stale copy.
    let id = PageId::from("pages/Notes.md");
    let (rev, before) = (memory.rev().0, memory.memory_page_bytes(&id));
    let mut guide = memory.page(&id).unwrap();
    guide.guide = true;
    assert!(matches!(
        memory.submit_page(&id, &guide, true),
        Err(crate::PageRefusal::InvalidTarget { .. })
    ));
    let dto = memory.page(&id).unwrap();
    let outside = PageId::from("logseq/custom.css");
    assert!(matches!(
        memory.submit_page(&outside, &dto, true),
        Err(crate::PageRefusal::InvalidTarget { .. })
    ));
    assert_eq!(
        memory.submit_page(&id, &dto, false).unwrap_err(),
        crate::PageRefusal::NotAdmitted
    );
    assert_eq!(
        (memory.rev().0, memory.memory_page_bytes(&id)),
        (rev, before)
    );
    store.close();
}
