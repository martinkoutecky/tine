//! GH #623 BR3: a page open costs the parse of that page, whatever the graph
//! is doing — loading, serving a launch checkpoint, or just after its file set
//! changed — and an edit saved while the graph loads survives the load.
use super::checkpoint::CheckpointWrite;
use super::*;
use crate::model::{GRAPH_LIST_CALLS, GRAPH_PREAMBLE_READS};
use std::sync::mpsc;
use std::time::Duration;

fn old() -> SystemTime {
    SystemTime::now() - Duration::from_secs(3600)
}

/// A graph of `pages` ordinary pages, files outside the racy window.
fn graph(pages: usize) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    fs::write(
        root.path().join("logseq/config.edn"),
        "{:preferred-format :markdown}\n",
    )
    .unwrap();
    for i in 0..pages {
        fs::write(
            root.path().join(format!("pages/P{i}.md")),
            format!("- page {i} [[P0]]\n"),
        )
        .unwrap();
    }
    fs::write(
        root.path().join("pages/Titled.md"),
        "title:: Named\n\n- t\n",
    )
    .unwrap();
    for entry in fs::read_dir(root.path().join("pages")).unwrap() {
        File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap()
            .set_modified(old())
            .unwrap();
    }
    root
}

fn open(root: &Path, checkpoint: Option<&Path>) -> Arc<Store> {
    Arc::new(
        Store::open(
            root,
            OpenOptions {
                watch: WatchMode::Poll,
                launch_checkpoint: checkpoint.map(Path::to_path_buf),
                ..Default::default()
            },
        )
        .unwrap()
        .0,
    )
}

/// `page_named(name)` — what the app's `get_page` asks before Ready — on
/// another thread, answered within two seconds or not.
fn get_page_promptly(store: &Arc<Store>, name: &str) -> Option<PageRead> {
    let (send, receive) = mpsc::channel();
    let (store, name) = (Arc::clone(store), name.to_owned());
    std::thread::spawn(move || {
        let _ = send.send(store.page_named(&name, PageKind::Page));
    });
    receive
        .recv_timeout(Duration::from_secs(2))
        .ok()
        .map(|read| read.unwrap().expect("page exists"))
}

fn save_text(store: &Store, read: &PageRead, text: &str) {
    let mut doc = read.doc.clone();
    doc.blocks[0].raw = text.into();
    assert!(
        matches!(
            store.save(
                crate::EditKind::SaveBlock,
                &read.id,
                SaveBase::Existing(read.rev.clone()),
                &doc
            ),
            SaveOutcome::Saved(_)
        ),
        "the save is accepted"
    );
}

/// The edit is in the published view, in a fresh page read and on disk.
fn assert_holds(store: &Store, root: &Path, name: &str, text: &str) {
    let view = store.whole_graph().unwrap();
    assert!(
        view.corpus()
            .pages
            .iter()
            .any(|page| page.name == name && page.document.roots[0].raw().contains(text)),
        "the graph view holds {name}'s edit"
    );
    let id = PageId::from(format!("pages/{name}.md").as_str());
    assert!(store.page(&id).unwrap().doc.blocks[0].raw.contains(text));
    assert!(fs::read_to_string(root.join(format!("pages/{name}.md")))
        .unwrap()
        .contains(text));
}

#[test]
fn a_page_open_after_the_file_set_changes_reads_no_other_file() {
    let root = graph(40);
    let store = open(root.path(), None);
    store.whole_graph().unwrap();
    // Today's journal, a created page, a sync delivery: the file set moves.
    fs::write(root.path().join("pages/Arrived.md"), "- new\n").unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    GRAPH_LIST_CALLS.with(|calls| calls.set(0));
    GRAPH_PREAMBLE_READS.with(|reads| reads.set(0));
    let read = store.page(&PageId::from("pages/P7.md")).unwrap();
    assert_eq!(read.doc.blocks[0].raw, "page 7 [[P0]]");
    // Two reads are the target's own preamble (its listing entry, then the
    // cache's reconcile); before the fix this was every page in the graph.
    let (walks, reads) = (
        GRAPH_LIST_CALLS.with(|calls| calls.get()),
        GRAPH_PREAMBLE_READS.with(|reads| reads.get()),
    );
    assert!(
        walks == 0 && reads <= 2,
        "{walks} walks, {reads} preamble reads: GH #623 BR3 / I-12: a page open after a file-set change must not walk the graph \
         or read other pages' preambles under the writer (~2.8 s on Windows for 11k pages); \
         canonicality comes from the published name index — exemplar Store::canonical_claim"
    );
    store.close();
}

#[test]
fn a_page_open_before_ready_needs_neither_the_load_nor_the_writer() {
    let root = graph(5);
    let hold = root.path().join(".tine-test-pause-load");
    fs::write(&hold, "").unwrap();
    let store = open(root.path(), None);
    assert!(matches!(store.is_graph_ready(), Ok(false)));
    let read = get_page_promptly(&store, "P3");
    fs::remove_file(&hold).unwrap();
    let read = read.expect(
        "GH #623 BR3: a page some file is named for opens before the initial parse finishes",
    );
    assert_eq!(read.doc.blocks[0].raw, "page 3 [[P0]]");
    // A title-only name is still found by its claimant, not reported absent.
    let titled = store.page_named("Named", PageKind::Page).unwrap().unwrap();
    assert_eq!(titled.id.as_str(), "pages/Titled.md");
    store.close();
}

/// Kill-free race (crash/power-loss stand-in for "the app was busy loading"):
/// a page opened and saved while the initial parse has not started is in the
/// cache and on disk once the load completes.
#[test]
fn an_edit_saved_while_the_graph_loads_survives_the_load() {
    let root = graph(5);
    let hold = root.path().join(".tine-test-pause-load");
    fs::write(&hold, "").unwrap();
    let store = open(root.path(), None);
    let read = get_page_promptly(&store, "P2").expect("opens while loading");
    save_text(&store, &read, "edited while loading");
    assert!(matches!(store.is_graph_ready(), Ok(false)));
    fs::remove_file(&hold).unwrap();
    assert_holds(&store, root.path(), "P2", "edited while loading");
    store.close();
}

/// The checkpoint path: an external edit made while Tine was closed and an
/// edit saved while the checkpoint is served (launch diff still running) both
/// survive Ready, the next checkpoint and the next warm launch.
#[test]
fn an_edit_saved_while_a_checkpoint_is_served_is_reconciled() {
    let root = graph(5);
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    let first = open(root.path(), Some(&cp));
    first.whole_graph_reconciled().unwrap();
    assert!(matches!(
        first.write_checkpoint_now(),
        Some(CheckpointWrite::Written { .. })
    ));
    first.close();
    fs::write(root.path().join("pages/P1.md"), "- edited while closed\n").unwrap();
    let pause = root.path().join(".tine-test-pause-launch-diff");
    fs::write(&pause, "").unwrap();
    let store = open(root.path(), Some(&cp));
    store.whole_graph().unwrap();
    assert!(matches!(store.is_graph_ready(), Ok(false)), "served");
    let read = get_page_promptly(&store, "P2");
    let opened = read.is_some();
    let closed_edit = get_page_promptly(&store, "P1");
    let saver = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            if let Some(read) = read {
                save_text(&store, &read, "edited while served");
            }
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    fs::remove_file(&pause).unwrap();
    saver.join().unwrap();
    assert!(
        opened,
        "GH #623 BR3: a page opens while the launch diff holds the writer"
    );
    assert_eq!(
        closed_edit.unwrap().doc.blocks[0].raw,
        "edited while closed",
        "a served read parses disk, not the checkpoint"
    );
    store.whole_graph_reconciled().unwrap();
    assert_holds(&store, root.path(), "P2", "edited while served");
    assert_holds(&store, root.path(), "P1", "edited while closed");
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(CheckpointWrite::Written { .. })
    ));
    store.close();
    let warm = open(root.path(), Some(&cp));
    warm.whole_graph_reconciled().unwrap();
    assert_holds(&warm, root.path(), "P2", "edited while served");
    assert_holds(&warm, root.path(), "P1", "edited while closed");
    warm.close();
}
