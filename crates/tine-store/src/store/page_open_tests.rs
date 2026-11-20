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

fn save_text(store: &std::sync::Arc<Store>, read: &PageRead, text: &str) {
    let host_writer = crate::test_host_writer::HostWriter::start(store);
    let mut doc = read.doc.clone();
    doc.blocks[0].raw = text.into();
    host_writer
        .save(
            crate::EditKind::SaveBlock,
            &read.id,
            SaveBase::Existing(read.rev.clone()),
            &doc,
        )
        .unwrap();
}

/// The edit is in the published view, in a fresh page read and on disk.
fn assert_holds(store: &Store, root: &Path, name: &str, text: &str) {
    let view = store.whole_graph().unwrap();
    assert!(
        view.corpus(None, false)
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

/// A dated file delivered after Ready and read by path before the watcher
/// sees it claims its day from its own observation (state model S4): the
/// read opens a constant number of preambles (its own, and an ordinary
/// page's title check), whatever the graph's size (review
/// ca24f8f2c F2: a journal fell back to the cold claimant walk).
#[test]
fn a_dated_file_read_by_path_after_delivery_reads_no_other_file() {
    for (path, text) in [
        ("journals/2026_11_02.md", "- delivered\n"),
        ("pages/Diary.md", "title:: Nov 2nd, 2026\n\n- delivered\n"),
    ] {
        let root = graph(200);
        let store = open(root.path(), None);
        store.whole_graph().unwrap();
        fs::write(root.path().join(path), text).unwrap();
        GRAPH_LIST_CALLS.with(|calls| calls.set(0));
        GRAPH_PREAMBLE_READS.with(|reads| reads.set(0));
        let read = store.page(&PageId::from(path)).unwrap();
        assert_eq!(read.doc.blocks[0].raw, "delivered");
        let (walks, reads) = (
            GRAPH_LIST_CALLS.with(|calls| calls.get()),
            GRAPH_PREAMBLE_READS.with(|reads| reads.get()),
        );
        assert!(
            walks == 0 && reads <= 3,
            "{path}: {walks} walks, {reads} preamble reads; a delivered dated file must not walk the graph"
        );
        store.close();
    }
}

/// Reviews db344b16e F1 and cf7ede011 F1, model check 2 §4: on a folding
/// volume a discovery candidate reaches a late journal spelled otherwise on
/// disk, by case (`X …` asked, `x …` on disk; `Nov 2nd, 2026` asked,
/// `NOV 2nd, 2026` on disk) or by Unicode composition (`Á …` asked, `A\u{301} …`
/// on disk). A hard link stands for the asked spelling on Linux and the folding
/// seam answers canonicalization and the write-target probes as such a volume
/// does. The journal is found and published once, at its disk spelling.
#[test]
fn a_late_journal_reached_through_another_spelling_is_one_claimant_at_its_disk_spelling() {
    for (formats, title, disk, asked) in [
        (
            Some(("X yyyy_MM_dd", "x yyyy_MM_dd")),
            "x 2026_11_02",
            "x 2026_11_02.md",
            "X 2026_11_02.md",
        ),
        (
            Some(("\u{c1} yyyy_MM_dd", "A\u{301} yyyy_MM_dd")),
            "A\u{301} 2026_11_02",
            "A\u{301} 2026_11_02.md",
            "\u{c1} 2026_11_02.md",
        ),
        (
            None,
            "Nov 2nd, 2026",
            "NOV 2nd, 2026.md",
            "Nov 2nd, 2026.md",
        ),
    ] {
        let root = graph(3);
        if let Some((file_format, title_format)) = formats {
            fs::write(
                root.path().join("logseq/config.edn"),
                format!(
                    "{{:preferred-format :markdown :journal/file-name-format \"{file_format}\" \
                     :journal/page-title-format \"{title_format}\"}}\n"
                ),
            )
            .unwrap();
        }
        let store = open(root.path(), None);
        store.whole_graph().unwrap();
        // On Linux the watcher would list the hard link as a second file.
        store.watch.stop_background_for_test();
        let journals = root.path().join("journals");
        fs::write(journals.join(disk), "- late\n").unwrap();
        if !journals.join(asked).exists() {
            fs::hard_link(journals.join(disk), journals.join(asked)).unwrap();
        }
        let read = crate::path_identity::with_folding_alias(
            &journals.join(asked),
            &journals.join(disk),
            || store.page_named(title, PageKind::Journal),
        );
        let read = read.unwrap().expect("the day's file is found");
        assert_eq!(read.id.as_str(), format!("journals/{disk}"));
        let view = store.whole_graph().unwrap();
        let conflicts = view.inventory(InventoryScope::ConflictingFiles(None));
        assert!(conflicts.0.is_empty(), "{disk}: one file, one claimant");
        store.close();
    }
}

/// Review b98963b0d F1: late discovery probes only what graph discovery would
/// list. A directory, symlink, FIFO or socket under the day's file name, and a
/// hidden or nonportable candidate name, is no hit: the day's page and journal
/// lookups answer at once, without an error.
#[cfg(unix)]
#[test]
fn late_discovery_skips_entries_graph_discovery_would_not_list() {
    use std::os::unix::net::UnixListener;
    for (case, file_format) in [
        ("dir", None),
        ("symlink", None),
        ("fifo", None),
        ("socket", None),
        ("hidden", Some(".yyyy_MM_dd")),
        ("nonportable", Some("yyyy_MM_dd?")),
    ] {
        let root = graph(3);
        if let Some(format) = file_format {
            fs::write(
                root.path().join("logseq/config.edn"),
                format!("{{:preferred-format :markdown :journal/file-name-format \"{format}\"}}\n"),
            )
            .unwrap();
        }
        let store = open(root.path(), None);
        store.whole_graph().unwrap();
        store.watch.stop_background_for_test();
        let journals = root.path().join("journals");
        let day = journals.join("2026_11_02.md");
        let _listener = match case {
            "dir" => fs::create_dir(&day).map(|_| None).unwrap(),
            "symlink" => {
                fs::write(root.path().join("elsewhere.md"), "- outside\n").unwrap();
                std::os::unix::fs::symlink(root.path().join("elsewhere.md"), &day).unwrap();
                None
            }
            "fifo" => {
                let made = std::process::Command::new("mkfifo")
                    .arg(&day)
                    .status()
                    .unwrap();
                assert!(made.success());
                None
            }
            "socket" => Some(UnixListener::bind(&day).unwrap()),
            "hidden" => {
                fs::write(journals.join(".2026_11_02.md"), "- hidden\n").unwrap();
                None
            }
            _ => {
                fs::write(journals.join("2026_11_02?.md"), "- nonportable\n").unwrap();
                None
            }
        };
        for kind in [PageKind::Journal, PageKind::Page] {
            let (send, receive) = mpsc::channel();
            let asking = Arc::clone(&store);
            std::thread::spawn(move || {
                let _ = send.send(
                    asking
                        .page_named("Nov 2nd, 2026", kind)
                        .map(|read| read.map(|r| r.id)),
                );
            });
            let answer = receive
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("{case} {kind:?}: the lookup blocked"));
            assert!(answer.is_ok(), "{case} {kind:?}: {answer:?}");
            assert_eq!(answer.unwrap(), None, "{case} {kind:?}: no hit");
        }
        store.close();
    }
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
