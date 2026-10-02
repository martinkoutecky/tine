//! Launch checkpoint (ADR 0070): round trip, fallbacks, racy stamps, R5.
use super::*;
use std::time::{Duration, SystemTime};

fn set_mtime(path: &Path, when: SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

fn old() -> SystemTime {
    SystemTime::now() - Duration::from_secs(3600)
}

/// A small graph whose files are all outside the racy window.
fn graph() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    let files = [
        (
            "logseq/config.edn",
            "{:preferred-format :markdown}\n".to_owned(),
        ),
        ("pages/A.md", "- links [[One]]\n".to_owned()),
        (
            "pages/B.md",
            "alias:: Bee\ntags:: t0\n\n- TODO [#A] see [[A]] #t1\n  - child ((x))\n".to_owned(),
        ),
        (
            "pages/One.md",
            "title:: One\n- icon:: x\n- body\n".to_owned(),
        ),
        ("pages/C.org", "* heading [[A]]\n** child\n".to_owned()),
        ("journals/2026_01_02.md", "- day [[B]] [[Bee]]\n".to_owned()),
    ];
    for (rel, text) in files {
        let path = root.path().join(rel);
        fs::write(&path, text).unwrap();
        set_mtime(&path, old());
    }
    root
}

fn open_cp(root: &Path, cp: &Path) -> Store {
    Store::open(
        root,
        OpenOptions {
            watch: WatchMode::Poll,
            launch_checkpoint: Some(cp.to_path_buf()),
            ..Default::default()
        },
    )
    .unwrap()
    .0
}

fn load_outcome(store: &Store) -> String {
    store.diagnostics()["checkpoint"]["load"]["outcome"]
        .as_str()
        .unwrap_or("none")
        .to_owned()
}

fn parse_passes(store: &Store) -> u64 {
    store.diagnostics()["launch"]["loadPassesTotal"]
        .as_u64()
        .unwrap()
}

/// Open, reach Ready, write the checkpoint, close.
fn write_checkpoint(root: &Path, cp: &Path) {
    let store = open_cp(root, cp);
    store.whole_graph_reconciled().unwrap();
    let outcome = store.write_checkpoint_now();
    assert!(
        matches!(outcome, Some(CheckpointWrite::Written { .. })),
        "checkpoint not written: {outcome:?}"
    );
    store.close();
}

fn publisher(store: &Store) -> Publisher {
    Publisher {
        path: PathBuf::new(),
        graph: Arc::clone(&store.graph),
        writer: Arc::clone(&store.writer),
        load: Arc::clone(&store.load),
        changes: Arc::clone(&store.changes),
        watch: store.watch.core_for_load(),
        signal: Arc::default(),
    }
}

/// Everything a checkpoint would hold, generation number zeroed, as bytes.
fn captured(store: &Store) -> Vec<u8> {
    store.whole_graph_reconciled().unwrap();
    let (body, _) = publisher(store)
        .capture()
        .map_err(|e| e.to_string())
        .unwrap();
    let body = Body {
        graph: body.graph.without_generation(),
        ..body
    };
    postcard::to_stdvec(&body).unwrap()
}

fn uuids(store: &Store) -> Vec<(String, Vec<String>)> {
    fn walk(blocks: &[tine_core::doc::DocBlock], out: &mut Vec<String>) {
        for block in blocks {
            out.push(format!("{} {}", block.uuid, block.raw()));
            walk(&block.children, out);
        }
    }
    let view = store.whole_graph_reconciled().unwrap();
    let mut pages: Vec<_> = view
        .graph
        .pages
        .slots()
        .map(|(_, (entry, doc))| {
            let mut out = Vec::new();
            walk(&doc.roots, &mut out);
            (entry.rel_path_str().to_owned(), out)
        })
        .collect();
    pages.sort();
    pages
}

fn backlink_pages(store: &Store, name: &str) -> Vec<String> {
    let view = store.whole_graph_reconciled().unwrap();
    let mut pages: Vec<String> = view
        .graph
        .backlinks_bounded(name, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES)
        .groups
        .iter()
        .map(|group| group.page.clone())
        .collect();
    pages.sort();
    pages
}

#[test]
fn a_loaded_checkpoint_equals_a_fresh_build() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    let written = open_cp(root.path(), &cp);
    let before = captured(&written);
    assert!(matches!(
        written.write_checkpoint_now(),
        Some(CheckpointWrite::Written { .. })
    ));
    written.close();

    let loaded = open_cp(root.path(), &cp);
    let after = captured(&loaded);
    assert_eq!(load_outcome(&loaded), "loaded");
    assert_eq!(parse_passes(&loaded), 0, "a loaded launch parses nothing");
    let fresh = Store::open(root.path(), Default::default()).unwrap().0;
    let built = captured(&fresh);
    assert!(
        before == after,
        "ADR 0070: install + capture must round-trip the generation"
    );
    assert!(
        after == built,
        "ADR 0070: a loaded checkpoint must equal a fresh build"
    );
    assert_eq!(
        uuids(&loaded),
        uuids(&fresh),
        "runtime block ids are reassigned as a parse assigns them"
    );
    assert_eq!(backlink_pages(&loaded, "A"), backlink_pages(&fresh, "A"));
    assert_eq!(
        backlink_pages(&loaded, "Bee"),
        backlink_pages(&fresh, "Bee")
    );
    assert!(!backlink_pages(&loaded, "A").is_empty());
}

/// Build every lazily built part of the current generation (ADR 0070: the
/// checkpoint writes them in whatever state they are in).
fn warm(store: &Store) {
    use crate::model::GraphRead;
    let view = store.whole_graph_reconciled().unwrap();
    let graph = &view.graph;
    graph.block_page_hint("x");
    graph.referenced_page_names();
    graph.page_aliases_with_owners();
    graph.alias_owner_paths("bee");
    graph.query_index().registry(&graph.pages);
    answers(store);
}

/// What the warm parts answer: backlinks (derived memo) and queries (query
/// memo, query index), as page + first line.
fn answers(store: &Store) -> Vec<String> {
    let view = store.whole_graph_reconciled().unwrap();
    let graph = &view.graph;
    let show = |label: &str, groups: &[tine_core::RefGroup]| {
        let rows: Vec<String> = groups
            .iter()
            .flat_map(|group| {
                group.blocks.iter().map(move |block| {
                    format!("{}:{}", group.page, block.raw.lines().next().unwrap_or(""))
                })
            })
            .collect();
        format!("{label} => {}", rows.join(" | "))
    };
    let mut out = Vec::new();
    for name in ["A", "Bee", "B", "One", "t1"] {
        let groups = graph.backlinks_bounded(name, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES);
        out.push(show(&format!("bl:{name}"), &groups.groups));
    }
    for query in [
        "(task TODO)",
        "[[A]]",
        "(page-property alias Bee)",
        "(priority A)",
    ] {
        let groups =
            graph.run_query_bounded(query, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES);
        out.push(show(&format!("q:{query}"), &groups.groups));
    }
    out
}

/// ADR 0070 (Martin, 2026-10-02: memos are persisted): a warm generation
/// round-trips byte for byte, loads warm, and answers as a fresh build; a
/// file edited while closed invalidates through the ordinary carry rules.
#[test]
fn a_warm_checkpoint_loads_warm_and_answers_as_a_fresh_build() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    let written = open_cp(root.path(), &cp);
    warm(&written);
    let before = captured(&written);
    let warm_answers = answers(&written);
    assert!(matches!(
        written.write_checkpoint_now(),
        Some(CheckpointWrite::Written { .. })
    ));
    written.close();

    let loaded = open_cp(root.path(), &cp);
    let (blocks, referenced, query_index, derived, queries) =
        loaded.whole_graph_reconciled().unwrap().graph.warm_parts();
    assert!(
        blocks && referenced && query_index,
        "lazy indexes load built"
    );
    assert!(
        derived > 0 && queries > 0,
        "memos load warm: {derived} {queries}"
    );
    assert_eq!(load_outcome(&loaded), "loaded");
    let after = captured(&loaded);
    assert!(
        before == after,
        "ADR 0070: a warm generation must round-trip through the checkpoint"
    );
    let fresh = Store::open(root.path(), Default::default()).unwrap().0;
    assert_eq!(answers(&loaded), answers(&fresh));
    assert_eq!(answers(&loaded), warm_answers);
    fresh.close();
    loaded.close();

    // Closed edit: B stops linking A, C gains a TODO.
    fs::write(
        root.path().join("pages/B.md"),
        "alias:: Bee\ntags:: t0\n\n- DONE see nothing #t1\n  - child ((x))\n",
    )
    .unwrap();
    fs::write(
        root.path().join("pages/C.org"),
        "* TODO heading [[A]]\n** child\n",
    )
    .unwrap();
    for rel in ["pages/B.md", "pages/C.org"] {
        set_mtime(&root.path().join(rel), old());
    }
    let reloaded = open_cp(root.path(), &cp);
    let edited = answers(&reloaded);
    assert_eq!(load_outcome(&reloaded), "loaded");
    let fresh = Store::open(root.path(), Default::default()).unwrap().0;
    assert_ne!(edited, warm_answers, "the closed edit changes the answers");
    assert_eq!(edited, answers(&fresh));
}

#[test]
fn a_served_checkpoint_is_readable_before_ready_and_destructive_reads_wait() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    write_checkpoint(root.path(), &cp);
    let pause = root.path().join(".tine-test-pause-launch-diff");
    fs::write(&pause, b"").unwrap();
    let store = Arc::new(open_cp(root.path(), &cp));
    let view = store.whole_graph().unwrap();
    assert!(
        matches!(store.is_graph_ready(), Ok(false)),
        "served while still Loading"
    );
    assert!(!view.graph.pages.is_empty());
    let waiter = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || store.whole_graph_reconciled().map(|view| view.rev()))
    };
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !waiter.is_finished(),
        "a destructive read waits for the launch diff"
    );
    fs::remove_file(&pause).unwrap();
    let rev = waiter.join().unwrap().unwrap();
    assert!(rev > view.rev());
    assert!(matches!(store.is_graph_ready(), Ok(true)));
}

fn rewrite_header(cp: &Path, edit: impl FnOnce(&mut Header)) {
    let bytes = fs::read(cp).unwrap();
    let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut header: Header = postcard::from_bytes(&bytes[16..16 + len]).unwrap();
    edit(&mut header);
    let header = postcard::to_stdvec(&header).unwrap();
    let mut out = bytes[..12].to_vec();
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&bytes[16 + len..]);
    fs::write(cp, out).unwrap();
}

/// Replace the checkpoint's body (tests forge stamps the way an unseen
/// rewrite would leave them).
fn rewrite_body(cp: &Path, root: &Path, edit: impl FnOnce(&mut Body<PagesOut>)) {
    let bytes = fs::read(cp).unwrap();
    let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let header: Header = postcard::from_bytes(&bytes[16..16 + len]).unwrap();
    let body = decode(&bytes, &header.root, header.config_rev.as_ref()).unwrap();
    let mut body = Body {
        graph: body
            .graph
            .map_pages(|pages| PagesOut(pages.into_pages(root))),
        claimants: body.claimants,
        name_by_path: body.name_by_path,
        stamps: body.stamps,
        racy: body.racy,
    };
    edit(&mut body);
    fs::write(
        cp,
        encode(&header.root, header.config_rev, &body).unwrap().0,
    )
    .unwrap();
}

#[test]
fn an_unusable_checkpoint_falls_back_to_a_full_build() {
    type Damage = fn(&Path, &Path);
    let cases: [(&str, &str, Damage); 10] = [
        ("missing", "missing", |cp, _| fs::remove_file(cp).unwrap()),
        ("empty", "format", |cp, _| fs::write(cp, b"").unwrap()),
        ("torn inside the preamble", "format", |cp, _| {
            let bytes = fs::read(cp).unwrap();
            fs::write(cp, &bytes[..14]).unwrap()
        }),
        ("truncated", "length", |cp, _| {
            let bytes = fs::read(cp).unwrap();
            fs::write(cp, &bytes[..bytes.len() / 2]).unwrap()
        }),
        ("flipped payload byte", "checksum", |cp, _| {
            let mut bytes = fs::read(cp).unwrap();
            let at = bytes.len() - 3;
            bytes[at] ^= 0x40;
            fs::write(cp, bytes).unwrap()
        }),
        ("magic", "format", |cp, _| {
            let mut bytes = fs::read(cp).unwrap();
            bytes[0] = b'X';
            fs::write(cp, bytes).unwrap()
        }),
        ("format version", "format", |cp, _| {
            let mut bytes = fs::read(cp).unwrap();
            bytes[8..12].copy_from_slice(&(FORMAT + 1).to_le_bytes());
            fs::write(cp, bytes).unwrap()
        }),
        ("parser", "parser", |cp, _| {
            rewrite_header(cp, |header| header.parser = "lsdoc v0.0.0".into())
        }),
        ("root", "root", |cp, _| {
            rewrite_header(cp, |header| header.root = PathBuf::from("/elsewhere"))
        }),
        ("config edited while closed", "config", |_, root| {
            let path = root.join("logseq/config.edn");
            fs::write(
                &path,
                "{:preferred-format :markdown\n :journal/page-title-format \"yyyy-MM-dd\"}\n",
            )
            .unwrap();
            set_mtime(&path, old());
        }),
    ];
    for (case, token, damage) in cases {
        let root = graph();
        let dir = tempfile::tempdir().unwrap();
        let cp = dir.path().join("graph.bin");
        write_checkpoint(root.path(), &cp);
        damage(&cp, root.path());
        let store = open_cp(root.path(), &cp);
        store.whole_graph_reconciled().unwrap();
        assert_eq!(load_outcome(&store), token, "{case}");
        assert!(
            parse_passes(&store) >= 1,
            "{case}: falls back to a full build"
        );
        let fresh = Store::open(root.path(), Default::default()).unwrap().0;
        assert_eq!(
            captured(&store),
            captured(&fresh),
            "{case}: the full build is complete"
        );
        // The full build replaces the unusable checkpoint.
        assert!(matches!(
            store.write_checkpoint_now(),
            Some(CheckpointWrite::Written { .. })
        ));
        store.close();
        let reopened = open_cp(root.path(), &cp);
        reopened.whole_graph_reconciled().unwrap();
        assert_eq!(load_outcome(&reopened), "loaded", "{case}: replaced");
    }
}

#[test]
fn edits_made_while_closed_are_reconciled_before_ready() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    write_checkpoint(root.path(), &cp);
    // Modified (another size), created, removed.
    fs::write(
        root.path().join("pages/A.md"),
        "- now links [[Two]] instead\n",
    )
    .unwrap();
    fs::write(root.path().join("pages/New.md"), "- new [[One]]\n").unwrap();
    fs::remove_file(root.path().join("pages/C.org")).unwrap();
    let store = open_cp(root.path(), &cp);
    let fresh = Store::open(root.path(), Default::default()).unwrap().0;
    // Slot numbering legitimately differs after deltas; compare content.
    assert_eq!(uuids(&store), uuids(&fresh));
    assert_eq!(load_outcome(&store), "loaded");
    for name in ["A", "One", "Two", "B", "Bee", "New"] {
        assert_eq!(
            backlink_pages(&store, name),
            backlink_pages(&fresh, name),
            "{name}"
        );
    }
    assert_eq!(backlink_pages(&store, "Two"), vec!["A".to_owned()]);
    assert_eq!(backlink_pages(&store, "One"), backlink_pages(&fresh, "One"));
    let paths: Vec<String> = uuids(&store).into_iter().map(|(path, _)| path).collect();
    assert!(paths.iter().any(|p| p == "pages/New.md"), "{paths:?}");
    assert!(!paths.iter().any(|p| p == "pages/C.org"), "{paths:?}");
}

/// `rel` under `root` in the form the store records it: `Store::open`
/// canonicalizes the root (on Windows a `\\?\` path, where `/` is not a
/// separator), and the walk joins native components onto it.
fn stored_path(root: &Path, rel: &str) -> PathBuf {
    fs::canonicalize(root)
        .unwrap()
        .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Rewrite `rel` with same-size bytes and its old mtime, then make the
/// checkpoint's stamp for it match the new file exactly (an unseen rewrite:
/// a sync client preserving mtimes on a filesystem without ctime, or one
/// landing within the timestamp granule). `racy` sets its stored racy flag.
fn unseen_rewrite(root: &Path, cp: &Path, rel: &str, bytes: &str, racy: bool) {
    let path = stored_path(root, rel);
    let before = fs::metadata(&path).unwrap();
    assert_eq!(before.len() as usize, bytes.len());
    fs::write(&path, bytes).unwrap();
    set_mtime(&path, before.modified().unwrap());
    rewrite_body(cp, root, |body| {
        let at = body.stamps.iter().position(|(p, _)| *p == path).unwrap();
        let old_rev = body.stamps[at].1.rev().cloned();
        body.stamps[at].1 = crate::watch::stamp_metadata(&path)
            .unwrap()
            .with_rev(old_rev);
        body.racy.retain(|p| *p != path);
        if racy {
            body.racy.push(path.clone());
            body.racy.sort();
        }
    });
}

#[test]
fn a_racy_stamp_persists_and_forces_a_reread_after_reload() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    // A future mtime stays racy (§5.4) however long the test takes.
    let path = stored_path(root.path(), "pages/A.md");
    set_mtime(&path, SystemTime::now() + Duration::from_secs(3600));
    write_checkpoint(root.path(), &cp);
    let bytes = fs::read(&cp).unwrap();
    let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let header: Header = postcard::from_bytes(&bytes[16..16 + len]).unwrap();
    let body = decode(&bytes, &header.root, header.config_rev.as_ref()).unwrap();
    assert!(
        body.racy.contains(&path),
        "storage spec §5.4: the racy flag is persisted"
    );
    unseen_rewrite(root.path(), &cp, "pages/A.md", "- links [[Two]]\n", true);
    let store = open_cp(root.path(), &cp);
    assert_eq!(backlink_pages(&store, "Two"), vec!["A".to_owned()]);
    assert!(backlink_pages(&store, "One").is_empty());
    assert_eq!(load_outcome(&store), "loaded");
}

/// R5 (ADR 0070, accepted): a same-size rewrite outside the racy window that
/// leaves mtime, size, identity and ctime unchanged is not seen by the launch
/// diff, as it is not by any stat diff. Page opens still read disk, and
/// Rescan rebuilds the derived state.
#[test]
fn r5_an_unseen_rewrite_is_served_until_rescan_but_page_reads_see_disk() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    write_checkpoint(root.path(), &cp);
    unseen_rewrite(root.path(), &cp, "pages/A.md", "- links [[Two]]\n", false);
    let store = open_cp(root.path(), &cp);
    assert_eq!(
        backlink_pages(&store, "One"),
        vec!["A".to_owned()],
        "R5: stale until Rescan"
    );
    let read = store.page(&PageId::from("pages/A.md")).unwrap();
    assert_eq!(
        read.doc.blocks[0].raw, "links [[Two]]",
        "a page open reads disk"
    );
    assert_eq!(read.rev, FileRev::from_bytes(b"- links [[Two]]\n"));
    store.rebuild_graph().unwrap();
    assert_eq!(backlink_pages(&store, "Two"), vec!["A".to_owned()]);
    assert!(backlink_pages(&store, "One").is_empty());
}

/// Without the forged stamp, the same unseen-by-mtime rewrite is caught: on
/// Unix by ctime, elsewhere by the stored stamp's identity or mtime granule.
#[cfg(unix)]
#[test]
fn a_same_size_same_mtime_rewrite_is_caught_by_ctime() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    write_checkpoint(root.path(), &cp);
    let path = root.path().join("pages/A.md");
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    fs::write(&path, "- links [[Two]]\n").unwrap();
    set_mtime(&path, before);
    let store = open_cp(root.path(), &cp);
    assert_eq!(backlink_pages(&store, "Two"), vec!["A".to_owned()]);
}

#[test]
fn rescan_replaces_the_checkpoint() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    write_checkpoint(root.path(), &cp);
    unseen_rewrite(root.path(), &cp, "pages/A.md", "- links [[Two]]\n", false);
    let store = open_cp(root.path(), &cp);
    store.whole_graph_reconciled().unwrap();
    store.rebuild_graph().unwrap();
    // The Rescan's request is answered; wait for it through a second one.
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(CheckpointWrite::Written { .. })
    ));
    store.close();
    let reopened = open_cp(root.path(), &cp);
    assert_eq!(backlink_pages(&reopened, "Two"), vec!["A".to_owned()]);
    assert_eq!(load_outcome(&reopened), "loaded");
}

#[test]
fn a_killed_write_leaves_the_previous_checkpoint_usable() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    write_checkpoint(root.path(), &cp);
    // A crash mid-write leaves only a temp sibling (atomic_file's temp name).
    let temp = crate::atomic_file::temp_path(&cp, 1, "crash");
    fs::write(&temp, b"TINECKPT partial").unwrap();
    let store = open_cp(root.path(), &cp);
    store.whole_graph_reconciled().unwrap();
    assert_eq!(load_outcome(&store), "loaded");
}

#[test]
fn the_idle_publisher_writes_after_an_edit() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    let store = open_cp(root.path(), &cp);
    store.whole_graph_reconciled().unwrap();
    // A cold launch's first checkpoint is prompt (FIRST_IDLE), not IDLE.
    let deadline = std::time::Instant::now() + FIRST_IDLE * 4;
    while !cp.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        cp.exists(),
        "the first Ready publication of a cold launch is checkpointed after FIRST_IDLE"
    );
    assert_eq!(
        store.diagnostics()["checkpoint"]["last"]["outcome"],
        "written"
    );
}

#[test]
fn the_cadence_is_idle_spaced_and_age_bounded() {
    let t0 = Instant::now();
    let at = |secs: u64| t0 + Duration::from_secs(secs);
    let mut state = SignalState::default();
    assert_eq!(state.wait(t0), None, "nothing dirty, nothing due");

    // Cold launch: due FIRST_IDLE after the last publication.
    state.dirty_since = Some(t0);
    state.last_publication = Some(t0);
    assert_eq!(state.wait(t0), Some(FIRST_IDLE));
    assert_eq!(state.wait(at(5)), Some(Duration::ZERO));

    // After a write at 5 s: an edit at 10 s waits IDLE, then the spacing.
    state.first = false;
    state.last_write = Some(at(5));
    state.dirty_since = Some(at(10));
    state.last_publication = Some(at(10));
    assert_eq!(state.wait(at(10)), Some(Duration::from_secs(295)));
    assert_eq!(state.wait(at(100)), Some(Duration::from_secs(205)));
    assert_eq!(state.wait(at(305)), Some(Duration::ZERO));

    // Long after the last write, quiet time alone decides.
    state.last_publication = Some(at(1000));
    state.dirty_since = Some(at(1000));
    assert_eq!(state.wait(at(1000)), Some(IDLE));
    assert_eq!(state.wait(at(1060)), Some(Duration::ZERO));

    // Continuous editing: due MAX_AGE after the change, never sooner than
    // MIN_INTERVAL after the last write.
    state.last_write = Some(at(2000));
    state.dirty_since = Some(at(2000));
    state.last_publication = Some(at(2590));
    assert_eq!(state.wait(at(2590)), Some(Duration::from_secs(10)));
    state.last_publication = Some(at(2600));
    assert_eq!(state.wait(at(2600)), Some(Duration::ZERO));
}

#[test]
fn a_launch_served_from_a_checkpoint_keeps_the_ordinary_cadence() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    let written = open_cp(root.path(), &cp);
    written.whole_graph_reconciled().unwrap();
    assert!(matches!(
        written.write_checkpoint_now(),
        Some(CheckpointWrite::Written { .. })
    ));
    written.close();
    let store = open_cp(root.path(), &cp);
    store.whole_graph_reconciled().unwrap();
    assert_eq!(load_outcome(&store), "loaded");
    let signal = store.changes.checkpoint.get().expect("publisher running");
    assert!(
        !signal.state.lock().unwrap().first,
        "only a cold launch writes its first checkpoint after FIRST_IDLE"
    );
}

#[test]
fn the_parser_tag_matches_the_lsdoc_pin() {
    let manifest = include_str!("../../../tine-core/Cargo.toml");
    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("lsdoc"))
        .expect("lsdoc dependency");
    let tag = line
        .split("tag = \"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(
        PARSER,
        format!("lsdoc {tag}"),
        "ADR 0070: bump checkpoint::PARSER with the lsdoc pin, so a checkpoint parsed \
         by another parser is rebuilt instead of served"
    );
}

/// The encoded body of a fixed graph. Any change to what a checkpoint holds
/// or how it is encoded changes these bytes: bump `FORMAT` and re-pin the
/// digest together (ADR 0070: an old checkpoint must never be decoded under a
/// new meaning). Unix only: the body holds the platform's absolute paths.
#[cfg(unix)]
#[test]
fn the_golden_body_is_pinned_to_format() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let fixed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    // One file: parallel loading assigns page slots in completion order, so a
    // multi-file fixture has no stable byte image (the order is semantically
    // irrelevant; `a_loaded_checkpoint_equals_a_fresh_build` covers many files).
    for (rel, text) in [(
        "pages/A.md",
        "alias:: Ay\nicon:: x\n\n- TODO [#A] [[B]] #t ((x)) `c` [[Ay]]\n  - child SCHEDULED: <2026-01-03 Sat>\n",
    )] {
        fs::write(root.join(rel), text).unwrap();
        set_mtime(&root.join(rel), fixed);
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    // Warm: the image covers the lazily built half and both memos too.
    warm(&store);
    let (mut body, _) = publisher(&store).capture().unwrap();
    // Machine-dependent: inode identity and ctime.
    body.stamps.clear();
    body.racy.clear();
    let body = Body {
        graph: body
            .graph
            .without_generation()
            .with_alias_shards_merged()
            .at_day(0),
        ..body
    };
    let mut bytes = postcard::to_stdvec(&body).unwrap();
    // The temp root, replaced by a same-length name so length prefixes hold.
    let root_bytes = root.to_string_lossy().into_owned().into_bytes();
    let stand_in = vec![b'r'; root_bytes.len()];
    let mut at = 0;
    while let Some(found) = bytes[at..]
        .windows(root_bytes.len())
        .position(|window| window == root_bytes.as_slice())
    {
        let start = at + found;
        bytes[start..start + root_bytes.len()].copy_from_slice(&stand_in);
        at = start + root_bytes.len();
    }
    let digest: String = sha256(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        (FORMAT, digest.as_str()),
        (4, GOLDEN),
        "ADR 0070: the checkpoint body changed; bump FORMAT and re-pin GOLDEN"
    );
}

#[cfg(unix)]
const GOLDEN: &str = "68a2d0055b8a78e639c7853b7fbc03fda9789c8a1f0c85962a88bd3c0458ca31";
