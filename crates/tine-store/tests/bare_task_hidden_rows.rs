//! REG-BARE-TASK-HIDDEN-ROWS (I-4): an empty task (a bare `TODO`/`DONE` title,
//! no text after the marker) keeps its task state through every structural edit
//! that adds or removes a parser-owned row after the title: an id (block
//! reference, copy), a fold, a SCHEDULED line, an id stripped on copy. A
//! Logseq-written file of that shape (`* TODO` + `:PROPERTIES:`) opens as a task.
//!
//! Each case goes through the real path: the parser-owned edit door every writer
//! uses (`block_regions::edit`), the page plan and host, the bytes on disk, a fresh
//! `Store::open`, and the `(task ...)` query the user sees.
use std::collections::BTreeSet;
use std::fs;

use tine_core::block_regions::{edit, Edit};
use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::{parse_query_input, QueryInput};
use tine_store::{EditKind, IrAnswer, IrRequest, PageId, SaveBase, Store};

const TASKS: &str = "(task TODO DONE)";

/// Raw text of every block `(task TODO DONE)` returns, as a `{{query}}` runs it.
fn tasks(store: &Store) -> BTreeSet<String> {
    let graph = store.whole_graph().unwrap();
    let (query, view) = parse_query_input(
        TASKS,
        QueryInput::MacroQuery,
        tine_core::date::JournalDate::today(),
        Registry::none(),
    );
    let IrAnswer::Result(result) = graph
        .query_ir(
            IrRequest::Run {
                query: &query,
                view: &view,
                context: &ExecutionContext::default(),
            },
            &tine_store::Cancel::default(),
        )
        .map(|answer| answer.expect("never cancelled"))
        .unwrap()
    else {
        panic!("query_ir(Run) returns a result");
    };
    let QueryRows::Block { groups } = result.rows else {
        panic!("the task query is block-anchored");
    };
    groups
        .into_iter()
        .flat_map(|g| g.blocks.into_iter().map(|b| b.raw))
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

struct Graph {
    dir: tempfile::TempDir,
    path: &'static str,
}

impl Graph {
    fn new(path: &'static str, source: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::create_dir(dir.path().join("journals")).unwrap();
        fs::write(dir.path().join(path), source).unwrap();
        Graph { dir, path }
    }

    fn open(&self) -> Store {
        Store::open(self.dir.path(), Default::default()).unwrap().0
    }

    fn bytes(&self) -> String {
        fs::read_to_string(self.dir.path().join(self.path)).unwrap()
    }

    /// Apply `request` to the block at `index` (depth-first) through the shared
    /// edit door, save the page through the page host, and return the file bytes.
    fn apply(&self, index: &[usize], request: Edit) -> String {
        let store = std::sync::Arc::new(self.open());
        let writer = crate::test_host_writer::HostWriter::start(&store);
        let id = PageId::from(self.path);
        let read = store.page(&id).expect("the page opens");
        let mut doc = read.doc;
        let org = self.path.ends_with(".org");
        let mut block = &mut doc.blocks[index[0]];
        for &i in &index[1..] {
            block = &mut block.children[i];
        }
        block.raw = edit(&block.raw, org, request).unwrap();
        let outcome = writer.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &doc,
        );
        assert!(outcome.is_ok(), "save failed: {outcome:?}");
        drop(writer);
        store.close();
        self.bytes()
    }
}

fn id(value: Option<&str>) -> Edit {
    Edit::Property {
        key: "id".into(),
        value: value.map(Into::into),
    }
}

fn fold() -> Edit {
    Edit::Property {
        key: "collapsed".into(),
        value: Some("true".into()),
    }
}

fn scheduled() -> Edit {
    Edit::Planning {
        which: "Scheduled".into(),
        value: Some("<2026-11-12 Thu>".into()),
    }
}

#[test]
fn a_bare_task_stays_a_task_through_every_structural_edit() {
    // (file, source, block index, edit, bytes after, task raw after)
    type Case = (
        &'static str,
        &'static str,
        &'static [usize],
        fn() -> Edit,
        &'static str,
        &'static str,
    );
    let cases: &[Case] = &[
        (
            "pages/o.org",
            "* TODO\n* other\n",
            &[0],
            || id(Some("6679-abc")),
            "* TODO\n:PROPERTIES:\n:id: 6679-abc\n:END:\n* other\n",
            "TODO\n:PROPERTIES:\n:id: 6679-abc\n:END:",
        ),
        (
            "pages/o.org",
            "* parent\n** DONE\n*** child\n",
            &[0, 0],
            fold,
            "* parent\n** DONE\n:PROPERTIES:\n:collapsed: true\n:END:\n*** child\n",
            "DONE\n:PROPERTIES:\n:collapsed: true\n:END:",
        ),
        (
            "pages/o.org",
            "* TODO\n",
            &[0],
            scheduled,
            "* TODO\nSCHEDULED: <2026-11-12 Thu>\n",
            "TODO\nSCHEDULED: <2026-11-12 Thu>",
        ),
        (
            "pages/o.org",
            "* TODO\nSCHEDULED: <2026-11-12 Thu>\n",
            &[0],
            || id(Some("6679-abc")),
            "* TODO\nSCHEDULED: <2026-11-12 Thu>\n:PROPERTIES:\n:id: 6679-abc\n:END:\n",
            "TODO\nSCHEDULED: <2026-11-12 Thu>\n:PROPERTIES:\n:id: 6679-abc\n:END:",
        ),
        (
            "pages/m.md",
            "- TODO\n- other\n",
            &[0],
            || id(Some("6679-abc")),
            "- TODO\n  id:: 6679-abc\n- other\n",
            "TODO\nid:: 6679-abc",
        ),
        (
            "pages/m.md",
            "- parent\n\t- DONE\n\t\t- child\n",
            &[0, 0],
            fold,
            "- parent\n\t- DONE\n\t  collapsed:: true\n\t\t- child\n",
            "DONE\ncollapsed:: true",
        ),
    ];
    for (path, source, index, request, after, raw) in cases {
        let graph = Graph::new(path, source);
        // The task block's raw before the edit: the bare title, plus the
        // SCHEDULED line one case starts with.
        let before = if source.contains("SCHEDULED") {
            "TODO\nSCHEDULED: <2026-11-12 Thu>"
        } else {
            raw.split('\n').next().unwrap()
        };
        assert_eq!(tasks(&graph.open()), set(&[before]), "before: {source:?}");
        assert_eq!(&graph.apply(index, request()), after, "bytes: {source:?}");
        let reopened = graph.open();
        assert!(
            reopened.page(&PageId::from(*path)).is_ok(),
            "reopens: {after:?}"
        );
        assert_eq!(tasks(&reopened), set(&[raw]), "after: {after:?}");
    }
}

#[test]
fn a_logseq_bare_task_with_an_id_drawer_opens_edits_and_loses_its_id_as_a_task() {
    for (source, stripped) in [
        (
            "* TODO\n:PROPERTIES:\n:id: 6679-abc\n:END:\n* other\n",
            "* TODO\n* other\n",
        ),
        (
            "* a\n** DONE\n:PROPERTIES:\n:id: 6679-abc\n:END:\n",
            "* a\n** DONE\n",
        ),
        ("- TODO\n  id:: 6679-abc\n- other\n", "- TODO\n- other\n"),
    ] {
        let path = if source.starts_with('*') {
            "pages/o.org"
        } else {
            "pages/m.md"
        };
        let graph = Graph::new(path, source);
        let store = graph.open();
        let page = store.page(&PageId::from(path));
        assert!(page.is_ok(), "opens: {source:?}: {:?}", page.err());
        let bare = if source.contains("DONE") {
            "DONE"
        } else {
            "TODO"
        };
        assert_eq!(tasks(&store).len(), 1, "a task: {source:?}");
        store.close();
        let index: &[usize] = if source.starts_with("* a") {
            &[0, 0]
        } else {
            &[0]
        };
        // Copy strips the id; the empty task it leaves is still the task.
        assert_eq!(graph.apply(index, id(None)), stripped);
        assert_eq!(tasks(&graph.open()), set(&[bare]), "{stripped:?}");
    }
}

#[test]
fn prose_after_a_bare_marker_stays_prose_when_an_id_is_added() {
    // Logseq reads `TODO` followed by a body line as text, not a task; adding a
    // hidden row must not turn it into one.
    let graph = Graph::new("pages/o.org", "* TODO\nbody\n");
    assert!(tasks(&graph.open()).is_empty());
    assert_eq!(
        graph.apply(&[0], id(Some("6679-abc"))),
        "* TODO\n:PROPERTIES:\n:id: 6679-abc\n:END:\nbody\n"
    );
    assert!(tasks(&graph.open()).is_empty());
}

/// Review r1: a CRLF Org page, and blank lines beside the rows, keep the task
/// through open, an id edit and a reopen.
#[test]
fn crlf_and_blank_lines_beside_the_rows_keep_the_task() {
    for (path, source) in [
        (
            "pages/o.org",
            "* TODO\r\n:PROPERTIES:\r\n:id: 6679-abc\r\n:END:\r\n* other\r\n",
        ),
        (
            "pages/o.org",
            "* TODO\n:PROPERTIES:\n:id: 6679-abc\n:END:\n\n\n* other\n",
        ),
        ("pages/m.md", "- TODO\n\n  id:: 6679-abc\n- other\n"),
    ] {
        let graph = Graph::new(path, source);
        let store = graph.open();
        let page = store.page(&PageId::from(path));
        assert!(page.is_ok(), "opens: {source:?}: {:?}", page.err());
        assert_eq!(tasks(&store).len(), 1, "a task: {source:?}");
        store.close();
        // A fold is the edit that keeps the existing rows and adds one more.
        let after = graph.apply(&[0], fold());
        assert!(after.contains("collapsed"), "{after:?}");
        assert_eq!(tasks(&graph.open()).len(), 1, "after: {after:?}");
    }
    // CRLF: a bare `* TODO` line is a task as its LF twin is, before and after
    // Tine adds an id. (The drawer's own line endings are a separate matter of
    // the edit door: a one-line block carries no CRLF evidence.)
    let graph = Graph::new("pages/o.org", "* TODO\r\n* other\r\n");
    // The task's raw may or may not keep the line's CR (the CRLF Org fix strips
    // it); the user outcome is one task whose text is the bare marker.
    let bare = |store: &Store| -> BTreeSet<String> {
        tasks(store)
            .into_iter()
            .map(|raw| raw.trim_end_matches('\r').to_string())
            .collect()
    };
    assert_eq!(bare(&graph.open()), set(&["TODO"]));
    let after = graph.apply(&[0], id(Some("6679-abc")));
    assert!(after.starts_with("* TODO\r\n:PROPERTIES:"), "{after:?}");
    assert_eq!(tasks(&graph.open()).len(), 1, "{after:?}");
}

/// Review r2: removing the last row from a bare task with blank lines beside it
/// (an id stripped on copy, an Org task expanded) leaves it a task.
#[test]
fn removing_the_last_row_beside_blank_lines_keeps_the_task() {
    let unfold = || Edit::Property {
        key: "collapsed".into(),
        value: None,
    };
    for (path, source, request) in [
        (
            "pages/m.md",
            "- TODO\n\n  id:: 6679-abc\n- other\n",
            id(None),
        ),
        (
            "pages/o.org",
            "* TODO\n:PROPERTIES:\n:collapsed: true\n:END:\n\n* other\n",
            unfold(),
        ),
    ] {
        let graph = Graph::new(path, source);
        assert_eq!(tasks(&graph.open()).len(), 1, "before: {source:?}");
        let after = graph.apply(&[0], request);
        assert!(
            !after.contains("id::") && !after.contains("collapsed"),
            "{after:?}"
        );
        let store = graph.open();
        assert!(store.page(&PageId::from(path)).is_ok(), "opens: {after:?}");
        assert_eq!(tasks(&store).len(), 1, "after: {after:?}");
    }
}

#[test]
fn a_bare_task_after_a_leading_blank_line_keeps_its_task_through_an_id() {
    // Review r4: the block boundary skips leading blank lines (`\nTODO` is a
    // task); adding an id, reopening and removing the id keeps it one.
    let graph = Graph::new("pages/m.md", "- \n  TODO\n- other\n");
    let before = tasks(&graph.open());
    assert_eq!(before.len(), 1, "a task before: {before:?}");
    let with_id = graph.apply(&[0], id(Some("6679-abc")));
    assert!(with_id.contains("id:: 6679-abc"), "{with_id:?}");
    let reopened = graph.open();
    assert!(reopened.page(&PageId::from("pages/m.md")).is_ok());
    assert_eq!(tasks(&reopened).len(), 1, "a task with the id: {with_id:?}");
    reopened.close();
    let stripped = graph.apply(&[0], id(None));
    assert_eq!(tasks(&graph.open()), before, "{stripped:?}");
}
