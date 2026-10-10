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
        // Reads the graph-wide tag-target set (the plan's persisted
        // `tag_targets`, invalidated by any page's `tags::` move).
        "(all-page-tags)",
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
    // Tag pages, so `(all-page-tags)` has rows: B tags t0 now, t9 after the
    // closed edit below.
    for (rel, text) in [
        ("pages/t0.md", "- tagged zero\n"),
        ("pages/t9.md", "- tagged nine\n"),
    ] {
        fs::write(root.path().join(rel), text).unwrap();
        set_mtime(&root.path().join(rel), old());
    }
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

    // Closed edit: B stops linking A and moves its tag, C gains a TODO.
    fs::write(
        root.path().join("pages/B.md"),
        "alias:: Bee\ntags:: t9\n\n- DONE see nothing #t1\n  - child ((x))\n",
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
    assert!(
        edited.contains(&"q:(all-page-tags) => t9:tagged nine".to_owned()),
        "a loaded memo that reads tag targets is invalidated by a closed tags move: {edited:#?}"
    );
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
    let body = decode(&bytes, &header.root, &header.config_key).unwrap();
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
        encode(&header.root, header.config_key, &body).unwrap().0,
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
    let body = decode(&bytes, &header.root, &header.config_key).unwrap();
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
        (8, GOLDEN),
        "ADR 0070: the checkpoint body changed; bump FORMAT and re-pin GOLDEN"
    );
}

#[cfg(unix)]
const GOLDEN: &str = "eab33d188c073d3f2ff3cb185c82c07503eb5adb01204d95587d39fc529a4f38";

/// GH #623 (FORMAT 6): which cached pages carried a VCS anchor line travels
/// in the checkpoint, so a warm launch answers the conflict inventory with no
/// file read; edits made while closed are reconciled by the launch diff.
/// Unit cost: one `PathBuf` per anchor-carrying page (none in a clean graph),
/// no per-edit write (the checkpoint is a whole-generation write on its
/// existing cadence).
#[test]
fn the_anchor_flags_round_trip_and_follow_edits_made_while_closed() {
    let root = graph();
    let marked = "- a\n<<<<<<< HEAD\n- b\n>>>>>>> x\n";
    fs::write(root.path().join("pages/Marked.md"), marked).unwrap();
    set_mtime(&root.path().join("pages/Marked.md"), old());
    let state =
        |store: &Store, rel: &str| store.vcs_anchor_state(&crate::FileId::from(rel.to_owned()));
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    write_checkpoint(root.path(), &cp);

    let loaded = open_cp(root.path(), &cp);
    loaded.whole_graph_reconciled().unwrap();
    assert_eq!(load_outcome(&loaded), "loaded");
    assert_eq!(state(&loaded, "pages/Marked.md"), Some(true));
    assert_eq!(state(&loaded, "pages/A.md"), Some(false));
    assert_eq!(state(&loaded, "pages/C.org"), Some(false));
    assert_eq!(state(&loaded, "pages/Nope.md"), None, "uncached: unknown");
    loaded.close();

    // Closed edits: the marked page is resolved, a clean one gains an anchor.
    fs::write(root.path().join("pages/Marked.md"), "- resolved\n").unwrap();
    fs::write(
        root.path().join("pages/A.md"),
        ">>>>>>> x\n- links [[One]]\n",
    )
    .unwrap();
    for rel in ["pages/Marked.md", "pages/A.md"] {
        set_mtime(&root.path().join(rel), old());
    }
    let reloaded = open_cp(root.path(), &cp);
    reloaded.whole_graph_reconciled().unwrap();
    assert_eq!(load_outcome(&reloaded), "loaded");
    assert_eq!(state(&reloaded, "pages/Marked.md"), Some(false));
    assert_eq!(state(&reloaded, "pages/A.md"), Some(true));
    reloaded.close();
}

/// ADR 0070 (Martin, 2026-10-02: config by meaning, not bytes): a config
/// edited while Tine was closed falls the launch back to the initial build
/// when it moves a setting the build reads, and keeps the checkpoint
/// otherwise; either way the reconciled graph answers as a fresh build under
/// the new config.
#[test]
fn a_config_edit_while_closed_rebuilds_only_for_a_setting_the_build_reads() {
    use tine_core::config::Config;
    const BASE: &str = ":preferred-format :markdown";
    // (field, the edited config's entries, keyed)
    let cases: [(&str, String, bool); 33] = [
        (
            "journals_dir",
            format!("{BASE} :journals-directory \"days\""),
            true,
        ),
        (
            "pages_dir",
            format!("{BASE} :pages-directory \"notes\""),
            true,
        ),
        ("hidden", format!("{BASE} :hidden [\"pages/C.org\"]"), true),
        (
            "hidden_parse_failed_closed",
            format!("{BASE} :hidden [\"open"),
            true,
        ),
        (
            "block_hidden_properties",
            format!("{BASE} :block-hidden-properties #{{:icon}}"),
            true,
        ),
        (
            "separated_by_commas",
            format!("{BASE} :property/separated-by-commas #{{:icon}}"),
            true,
        ),
        (
            "ignored_page_references_keywords",
            format!("{BASE} :ignored-page-references-keywords #{{:tags}}"),
            true,
        ),
        (
            "property_pages_enabled",
            format!("{BASE} :property-pages/enabled? false"),
            true,
        ),
        (
            "property_pages_excludelist",
            format!("{BASE} :property-pages/excludelist #{{:icon}}"),
            true,
        ),
        (
            "favorites_page",
            format!("{BASE} :tine/favorites-page \"A\""),
            true,
        ),
        (
            "journal_file_name_format",
            format!("{BASE} :journal/file-name-format \"yyyy-MM-dd\""),
            true,
        ),
        (
            "journal_page_title_format",
            format!("{BASE} :journal/page-title-format \"yyyy-MM-dd\""),
            true,
        ),
        (
            "preferred_format",
            ":preferred-format :org".to_owned(),
            true,
        ),
        (
            "file_name_format",
            format!("{BASE} :file/name-format :triple-lowbar"),
            true,
        ),
        (
            "enable_search_remove_accents",
            format!("{BASE} :feature/enable-search-remove-accents? false"),
            true,
        ),
        (
            "preferred_workflow",
            format!("{BASE} :preferred-workflow :todo"),
            false,
        ),
        (
            "shortcuts",
            format!("{BASE} :shortcuts {{:editor/new-block \"alt+enter\"}}"),
            false,
        ),
        (
            "all_pages_public",
            format!("{BASE} :publishing/all-pages-public? true"),
            false,
        ),
        ("start_of_week", format!("{BASE} :start-of-week 1"), false),
        (
            "linked_references_collapsed_threshold",
            format!("{BASE} :ref/linked-references-collapsed-threshold 5"),
            false,
        ),
        (
            "default_journal_template",
            format!("{BASE} :default-templates {{:journals \"Daily\"}}"),
            false,
        ),
        (
            "default_home",
            format!("{BASE} :default-home {{:page \"A\"}}"),
            false,
        ),
        ("favorites", format!("{BASE} :favorites [\"A\"]"), false),
        (
            "mobile_gestures_disabled_in_block_with_tags",
            format!("{BASE} :mobile {{:gestures/disabled-in-block-with-tags [\"kanban\"]}}"),
            false,
        ),
        (
            "quick_capture_template_text",
            format!("{BASE} :quick-capture-templates {{:text \"{{text}}\"}}"),
            false,
        ),
        (
            "quick_capture_template_media",
            format!("{BASE} :quick-capture-templates {{:media \"{{url}}\"}}"),
            false,
        ),
        (
            "macros",
            format!("{BASE} :macros {{\"m\" \"[[A]] $1\"}}"),
            false,
        ),
        (
            "enable_timetracking",
            format!("{BASE} :feature/enable-timetracking? false"),
            false,
        ),
        (
            "show_brackets",
            format!("{BASE} :ui/show-brackets? false"),
            false,
        ),
        (
            "doc_mode_enter_for_new_block",
            format!("{BASE} :shortcut/doc-mode-enter-for-new-block? true"),
            false,
        ),
        (
            "logical_outdenting",
            format!("{BASE} :editor/logical-outdenting? true"),
            false,
        ),
        (
            "logbook",
            format!("{BASE} :logbook/settings {{:enabled-in-all-blocks true}}"),
            false,
        ),
        (
            "guide_announced",
            format!("{BASE} :tine/guide-announced? true"),
            false,
        ),
    ];
    let base = Config::parse(&format!("{{{BASE}}}"));
    for (field, entries, keyed) in cases {
        let edn = format!("{{{entries}}}\n");
        let edited = Config::parse(&edn);
        assert_ne!(
            format!("{base:?}"),
            format!("{edited:?}"),
            "{field}: the edit moves the setting"
        );
        assert_eq!(
            config_key(&base) != config_key(&edited),
            keyed,
            "{field}: classified in config_key"
        );
        // Cold checkpoint: the whole generation, byte for byte; warm one:
        // the persisted memos and lazy indexes answer as a fresh build.
        for warm_first in [false, true] {
            let root = graph();
            let dir = tempfile::tempdir().unwrap();
            let cp = dir.path().join("graph.bin");
            let written = open_cp(root.path(), &cp);
            written.whole_graph_reconciled().unwrap();
            if warm_first {
                warm(&written);
            }
            assert!(matches!(
                written.write_checkpoint_now(),
                Some(CheckpointWrite::Written { .. })
            ));
            written.close();
            let path = root.path().join("logseq/config.edn");
            fs::write(&path, &edn).unwrap();
            set_mtime(&path, old());

            let store = open_cp(root.path(), &cp);
            store.whole_graph_reconciled().unwrap();
            let fresh = Store::open(root.path(), Default::default()).unwrap().0;
            assert_eq!(
                load_outcome(&store),
                if keyed { "config" } else { "loaded" },
                "{field}: keyed settings rebuild, others keep the checkpoint"
            );
            if !warm_first {
                assert!(
                    captured(&store) == captured(&fresh),
                    "{field}: a config edit while closed plus reconcile equals a fresh build"
                );
            }
            assert_eq!(uuids(&store), uuids(&fresh), "{field}");
            assert_eq!(answers(&store), answers(&fresh), "{field}");
            fresh.close();
            store.close();
        }
    }
}

/// Storage spec §5.4 (Martin, 2026-10-02, item 6): a launch diff that leaves
/// a path racy runs one follow-up full diff about 2 s later. Scenario: a file
/// edited just before launch is rewritten where no notification reaches the
/// watcher (a sync service writing to a network or FUSE mount); only that
/// follow-up sees it.
#[test]
fn a_launch_diff_that_leaves_a_path_racy_runs_one_follow_up_diff() {
    let open_notify = |root: &Path, cp: &Path| {
        let store = Store::open(
            root,
            OpenOptions {
                watch: WatchMode::Notify,
                launch_checkpoint: Some(cp.to_path_buf()),
                ..Default::default()
            },
        )
        .unwrap()
        .0;
        store
            .watch
            .core_for_load()
            .deaf
            .store(true, Ordering::Release);
        store
    };
    let follow_ups = |store: &Store| {
        store.diagnostics()["fullDiffs"]["recent"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|diff| diff["trigger"] == "racy_follow_up")
            .count()
    };
    // The OS watch was refused at some point (inotify limits on a loaded
    // host): the watcher then polls every cycle, which covers the rewrite by
    // itself and serves as the follow-up, so the follow-up is not observable.
    let polled = |store: &Store| {
        store.changes.watch_status.lock().unwrap().0.is_some()
            || store.diagnostics()["fullDiffs"]["recent"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diff| diff["trigger"] == "poll_cycle")
    };
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    write_checkpoint(root.path(), &cp);

    // Control: nothing racy at launch, nothing scheduled.
    let quiet = open_notify(root.path(), &cp);
    quiet.whole_graph_reconciled().unwrap();
    assert!(quiet
        .watch
        .core_for_load()
        .follow_up
        .lock()
        .unwrap()
        .is_none());
    quiet.close();

    // Edited while closed, just now: reread at launch, and racy.
    let a = root.path().join("pages/A.md");
    fs::write(&a, "- links [[Two]]\n").unwrap();
    let store = open_notify(root.path(), &cp);
    store.whole_graph_reconciled().unwrap();
    assert_eq!(load_outcome(&store), "loaded");
    assert_eq!(backlink_pages(&store, "Two"), vec!["A".to_owned()]);
    // The launch diff scheduled the follow-up. Timing is judged against that
    // deadline, not against Ready: on a loaded host the reconcile wait can
    // take most of the window.
    let due = *store.watch.core_for_load().follow_up.lock().unwrap();
    let Some(due) = due.filter(|due| *due > Instant::now() + Duration::from_millis(300)) else {
        // Already fired, or about to fire before the rewrite below could
        // land, or the watcher is polling: the catch is not observable here.
        // The schedule itself still happened, at most once.
        let wait = Instant::now() + Duration::from_secs(6);
        while follow_ups(&store) == 0 && !polled(&store) {
            assert!(
                Instant::now() < wait,
                "§5.4: the launch diff scheduled no follow-up"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(follow_ups(&store) <= 1);
        store.close();
        return;
    };
    // The OS watch installing after Ready reconciles once to close its install
    // gap (`DiffTrigger::WatchInstall`); on a slow host (the Windows runner) that
    // lands after the rewrite below and legitimately catches it first.
    let installs = |store: &Store| {
        store.diagnostics()["fullDiffs"]["recent"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|diff| diff["trigger"] == "watch_install")
            .count()
    };
    let installs_before = installs(&store);
    // Rewritten unseen by the (deaf) watcher.
    fs::write(&a, "- links [[Six]]\n").unwrap();
    let deadline = due + Duration::from_secs(4);
    while backlink_pages(&store, "Six").is_empty() {
        assert!(
            Instant::now() < deadline,
            "§5.4: no follow-up diff saw the unreported rewrite"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let caught = Instant::now();
    if polled(&store) || installs(&store) > installs_before {
        store.close();
        return;
    }
    assert!(
        caught + Duration::from_millis(50) >= due,
        "caught {:?} before the follow-up was due: something other than the follow-up saw it; diffs {}",
        due - caught,
        store.diagnostics()["fullDiffs"]["recent"]
    );
    assert_eq!(follow_ups(&store), 1);
    // One follow-up: it schedules none.
    std::thread::sleep(crate::watch::RACY_FOLLOW_UP);
    assert!(store
        .watch
        .core_for_load()
        .follow_up
        .lock()
        .unwrap()
        .is_none());
    assert_eq!(follow_ups(&store), 1);
    store.close();
}

/// ADR 0070 (Martin, 2026-10-02): a lazily built index or memo is a change.
/// A read-only session that builds one makes a checkpoint due on the ordinary
/// cadence, and the next launch serves it built; a session whose lazy state
/// the checkpoint already holds writes nothing.
#[test]
fn a_read_only_session_that_builds_a_lazy_index_is_checkpointed() {
    let signal = |store: &Store| Arc::clone(store.changes.checkpoint.get().expect("publisher"));
    // Look every 20 ms instead of every LAZY_POLL.
    let fast = |store: &Store| {
        let signal = signal(store);
        signal.state.lock().unwrap().poll = Duration::from_millis(20);
        signal.wake.notify_all();
    };
    let dirty = |store: &Store| signal(store).state.lock().unwrap().dirty_since.is_some();
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    // Cold build, checkpointed before anything lazy was built.
    write_checkpoint(root.path(), &cp);

    let session = open_cp(root.path(), &cp);
    session.whole_graph_reconciled().unwrap();
    assert_eq!(load_outcome(&session), "loaded");
    let (blocks, referenced, query_index, derived, queries) =
        session.whole_graph_reconciled().unwrap().graph.warm_parts();
    assert!(!(blocks || referenced || query_index) && derived == 0 && queries == 0);
    fast(&session);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !dirty(&session),
        "a session that builds nothing writes nothing"
    );

    // Reads only: no edit, no publication.
    warm(&session);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dirty(&session) {
        assert!(
            Instant::now() < deadline,
            "ADR 0070: a lazily built index or memo did not make a checkpoint due"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Due on the publication cadence: IDLE after the build was seen
    // (`the_cadence_is_idle_spaced_and_age_bounded` covers the rest).
    let wait = signal(&session)
        .state
        .lock()
        .unwrap()
        .wait(Instant::now())
        .expect("due");
    assert!(
        wait <= IDLE && wait + Duration::from_secs(2) >= IDLE,
        "{wait:?}"
    );
    // Take the due write now rather than waiting IDLE.
    assert!(matches!(
        session.write_checkpoint_now(),
        Some(CheckpointWrite::Written { .. })
    ));
    assert!(!dirty(&session));
    session.close();

    let next = open_cp(root.path(), &cp);
    next.whole_graph_reconciled().unwrap();
    assert_eq!(load_outcome(&next), "loaded");
    let (blocks, referenced, query_index, derived, queries) =
        next.whole_graph_reconciled().unwrap().graph.warm_parts();
    assert!(
        blocks && referenced && query_index && derived > 0 && queries > 0,
        "the next launch serves the lazily built state without rebuilding it"
    );
    fast(&next);
    warm(&next);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !dirty(&next),
        "lazy state the checkpoint already holds is not a change"
    );
    next.close();
}
