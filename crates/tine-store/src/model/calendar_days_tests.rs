use super::*;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::write(&path, text).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000))
        .unwrap();
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals"] {
        std::fs::create_dir(root.path().join(dir)).unwrap();
    }
    root
}

#[test]
fn calendar_reads_probe_no_pages_or_blocks_at_any_graph_size() {
    let mut probes = Vec::new();
    for count in [200, 2000] {
        let root = fixture();
        for i in 0..count {
            if i % 2 == 0 {
                write(root.path(), &format!("pages/P{i}.md"), "- ordinary\n");
            } else {
                let j = i / 2;
                let path = format!(
                    "journals/{}_{:02}_{:02}.md",
                    2000 + j / 336,
                    j / 28 % 12 + 1,
                    j % 28 + 1
                );
                let body = if j < 3 {
                    "- content\n"
                } else if j % 2 == 0 {
                    "- \n  - \n"
                } else {
                    "- \n  property:: value\n"
                };
                write(root.path(), &path, body);
            }
        }
        let store = crate::Store::open(root.path(), Default::default())
            .unwrap()
            .0;
        let view = store.whole_graph().unwrap();
        take_work();
        let start = std::time::Instant::now();
        let days = view.journal_content_days();
        let elapsed = start.elapsed();
        let work = take_work();
        eprintln!(
            "calendar pages={count} days={} page/predicate/block_probes={work:?} ns={}",
            days.len(),
            elapsed.as_nanos()
        );
        assert_eq!(days.len(), 3);
        probes.push(work);
        assert!(elapsed < std::time::Duration::from_millis(10));
        store.close();
    }
    assert_eq!(probes, [(0, 0, 0); 2]);
}

fn assert_days(store: &crate::Store, count: usize) -> Vec<i64> {
    let view = store.whole_graph().unwrap();
    let days = view.graph.journal_content_days();
    let oracle = view
        .graph
        .pages
        .iter()
        .filter(|(entry, _)| entry.kind == PageKind::Journal)
        .filter_map(|(entry, doc)| entry.date_key.filter(|_| doc_has_content(&doc.roots)))
        .collect::<Vec<_>>();
    assert_eq!(days, oracle, "preserve active selection and slot order");
    assert_eq!(days.len(), count);
    days
}

#[test]
fn content_days_follow_edits_duplicates_snapshots_and_warm_checkpoint() {
    let root = fixture();
    let a = "journals/2026_01_01.md";
    let b = "journals/2026_01_02.md";
    let c = "journals/2026_01_03.md";
    write(root.path(), a, "- parent\n");
    write(root.path(), b, "- \n  - nested\n");
    write(root.path(), c, "- \n  prop:: only\n");
    write(root.path(), "journals/2026_01_04.md", "- \n  -   \n");
    write(
        root.path(),
        "journals/2026_01_05.org",
        "* \n:PROPERTIES:\n:memo: only\n:END:\n",
    );
    let cp = root.path().join("launch.bin");
    let open = || {
        crate::Store::open(
            root.path(),
            crate::OpenOptions {
                watch: crate::WatchMode::Poll,
                launch_checkpoint: Some(cp.clone()),
                ..Default::default()
            },
        )
        .unwrap()
        .0
    };
    let store = open();
    let held = store.whole_graph().unwrap();
    let initial = assert_days(&store, 2);
    write(root.path(), a, "- \n  prop:: only\n");
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_days(&store, 1);
    write(root.path(), c, "- \n  - added\n");
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_days(&store, 2);
    std::fs::remove_file(root.path().join(b)).unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_days(&store, 1);
    std::fs::rename(
        root.path().join(c),
        root.path().join("journals/2026_01_06.md"),
    )
    .unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_days(&store, 1);
    // Same-day files use the existing active-page winner. Removing it must
    // promote its untouched sibling, rather than dropping the day permanently.
    write(
        root.path(),
        "journals/2026_01_06.org",
        "* duplicate content\n",
    );
    store.refresh(crate::Depth::Stamps).unwrap();
    let expected = assert_days(&store, 1);
    let winner = store
        .whole_graph()
        .unwrap()
        .graph
        .pages
        .iter()
        .find(|(entry, _)| entry.kind == PageKind::Journal && entry.date_key == Some(expected[0]))
        .unwrap()
        .0
        .path
        .clone();
    let blank = if winner.extension().unwrap() == "org" {
        "* \n"
    } else {
        "- \n"
    };
    write(
        root.path(),
        winner.strip_prefix(root.path()).unwrap().to_str().unwrap(),
        blank,
    );
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_days(&store, 0);
    std::fs::remove_file(&winner).unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_eq!(assert_days(&store, 1), expected);
    assert_eq!(held.graph.journal_content_days(), initial);
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(crate::store::checkpoint::CheckpointWrite::Written { .. })
    ));
    store.close();
    let store = open();
    assert_eq!(assert_days(&store, 1), expected);
    assert_eq!(
        store.diagnostics()["checkpoint"]["load"]["outcome"],
        "loaded"
    );
    store.close();
}

#[test]
fn publication_evaluates_only_the_changed_journal_once() {
    let root = fixture();
    write(root.path(), "journals/2026_01_01.md", "- content\n");
    write(root.path(), "journals/2026_01_02.md", "- other\n");
    write(root.path(), "pages/A.md", "- ordinary\n");
    let store = crate::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    let view = store.whole_graph().unwrap();
    let graph = &view.graph;
    for body in ["- changed\n", "- \n  property:: only\n", "- \n  - nested\n"] {
        let mut pages = (*graph.pages).clone();
        let rel = "journals/2026_01_01.md";
        let slot = *pages.positions.get(rel).unwrap();
        let entry = pages.get(slot).unwrap().0.clone();
        pages.set(slot, (entry, Arc::new(tine_core::doc::parse(body))));
        take_work();
        let days = CalendarDays::capture(Some(graph), &Arc::new(pages), &[rel.into()]);
        let work = take_work();
        assert_eq!((work.0, work.1), (1, 1));
        assert_eq!(work.2, if body.contains("nested") { 2 } else { 1 });
        assert_eq!(days.0.len(), if body.contains("only") { 1 } else { 2 });
        eprintln!("publication body={body:?} page/predicate/block_probes={work:?}");
    }
    take_work();
    CalendarDays::capture(Some(graph), &graph.pages, &["pages/A.md".into()]);
    assert_eq!(take_work(), (1, 0, 0));
    CalendarDays::capture(Some(graph), &graph.pages, &[]);
    assert_eq!(take_work(), (0, 0, 0));
    assert_eq!(graph.calendar_days.0.len(), 2);
    store.close();
}

#[test]
fn synthetic_10k_memory_and_enumeration_budget() {
    let doc = Arc::new(tine_core::doc::parse("- content\n"));
    let pages = Arc::new(Pages::from(
        (0..10_000)
            .map(|i| {
                let journal = i < 3000;
                let rel = format!("pages/P{i}.md");
                (
                    PageEntry {
                        name: format!("P{i}"),
                        kind: if journal {
                            PageKind::Journal
                        } else {
                            PageKind::Page
                        },
                        date_key: journal.then_some(20000101 + i),
                        path: PathBuf::from(&rel),
                        rel_path: Some(rel.into()),
                    },
                    Arc::clone(&doc),
                )
            })
            .collect::<Vec<_>>(),
    ));
    take_work();
    let days = CalendarDays::capture(None, &pages, &[]);
    assert_eq!(take_work(), (10_000, 3000, 3000));
    assert_eq!(days.0.len(), 3000);
    let bytes = days.0.allocation_bytes();
    assert_eq!(bytes, 264_008);
    let mut ns = Vec::new();
    for _ in 0..51 {
        let start = std::time::Instant::now();
        let result = days.0.values().copied().collect::<Vec<_>>();
        let elapsed = start.elapsed();
        assert_eq!(result.len(), 3000);
        assert!(elapsed < std::time::Duration::from_millis(10));
        std::hint::black_box(result);
        ns.push(elapsed.as_nanos());
    }
    assert_eq!(take_work(), (0, 0, 0));
    ns.sort_unstable();
    eprintln!("synthetic pages=10000 days=3000 requested_allocation_bytes={bytes} bytes_per_day=88 p50_ns={} p95_ns={}", ns[25], ns[48]);
}
