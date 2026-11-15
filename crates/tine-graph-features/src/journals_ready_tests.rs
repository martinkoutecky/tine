use super::*;

/// Hl2_0 review hold: the feed carried the graph's config restriction in the
/// page's own `read_only`, so a Markdown journal stayed uneditable (and showed
/// the Org banner) after the config was repaired. Like the page adapter, the
/// feed reports only the file's intrinsic restriction; the config one is the
/// graph fact the frontend already holds.
#[test]
fn an_unreadable_config_does_not_mark_a_markdown_journal_read_only_in_the_feed() {
    let root = tempfile::tempdir().unwrap();
    for dir in ["journals", "logseq"] {
        std::fs::create_dir(root.path().join(dir)).unwrap();
    }
    std::fs::write(root.path().join("logseq/config.edn"), b"{:a \"\xff\"}\n").unwrap();
    std::fs::write(root.path().join("journals/2020_01_01.md"), "- day\n").unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    store.stop_watcher_for_tests();
    let page = feed_page_through(&store, 3, None, None, true).unwrap();
    let day = page
        .pages
        .iter()
        .find(|p| p.doc.rev.is_some())
        .expect("the journal file");
    assert!(
        !day.doc.read_only,
        "the Markdown file itself stays editable"
    );
    store.close();
}

/// REG-JOURNAL-STEP-WINDOW: the file's own day comes from its name,
/// so a body that failed to read at load (invalid UTF-8) keeps it too.
#[test]
fn a_missing_file_keeps_the_full_unions_virtual_identity_with_a_short_year_format() {
    for (references, expected, body) in [
        ("- [[1920-01-01]]\n", None, &b"- body\n"[..]),
        ("- [[1920-01-01]]\n", None, &b"- \xff\xfe body\n"[..]),
        (
            "- [[1920-01-01]] [[2020-01-01]]\n",
            Some("Jan 1st, 2020"),
            &b"- body\n"[..],
        ),
        (
            "- [[1920-01-01]] [[2020-01-01]]\n",
            Some("Jan 1st, 2020"),
            &b"- \xff\xfe body\n"[..],
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        for dir in ["journals", "pages", "logseq"] {
            std::fs::create_dir(root.path().join(dir)).unwrap();
        }
        std::fs::write(
            root.path().join("logseq/config.edn"),
            "{:journal/file-name-format \"yy_MM_dd\"}\n",
        )
        .unwrap();
        std::fs::write(root.path().join("pages/Plan.md"), references).unwrap();
        let path = root.path().join("journals/20_01_01.md");
        std::fs::write(&path, body).unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        store.stop_watcher_for_tests();
        store.whole_graph().unwrap();
        std::fs::remove_file(path).unwrap();
        let page = feed_page_through(&store, 3, Some(20000101), None, true).unwrap();
        assert_eq!(
            page.pages
                .iter()
                .map(|p| p.doc.title.as_str())
                .collect::<Vec<_>>(),
            expected.into_iter().collect::<Vec<_>>()
        );
        assert!(page.done);
        store.close();
    }
}

#[test]
fn bounded_day_inventory_matches_full_inventory_for_colliding_titles() {
    for title_format in ["yyyy", "yyyy-MM", "EEEE, dd.MM.yyyy"] {
        let root = tempfile::tempdir().unwrap();
        for dir in ["journals", "logseq"] {
            std::fs::create_dir(root.path().join(dir)).unwrap();
        }
        std::fs::write(
            root.path().join("logseq/config.edn"),
            format!("{{:journal/page-title-format \"{title_format}\"}}\n"),
        )
        .unwrap();
        for day in [19990101, 19991231, 20000101, 20000102, 20000201] {
            let date = JournalDate::from_ordinal(day);
            std::fs::write(
                root.path()
                    .join(format!("journals/{}.md", date.file_stem())),
                "- body\n",
            )
            .unwrap();
        }
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        store.stop_watcher_for_tests();
        let graph = store.whole_graph().unwrap();
        let full = indexed_days_through(&graph, Day(20010101));
        for before in [19990101, 20000101, 20000102, 20010101] {
            for limit in [0, 1, 3, 10] {
                let actual: Vec<_> = graph
                    .inventory(tine_store::InventoryScope::JournalFilesBefore {
                        before: Day(before),
                        limit,
                    })
                    .0
                    .iter()
                    .map(|entry| {
                        let tine_store::Resolved::Existing { id, .. } = &entry.target else {
                            panic!("file target");
                        };
                        (entry.day.unwrap(), id.clone())
                    })
                    .collect();
                let expected: Vec<_> = full
                    .iter()
                    .filter(|(day, _)| day.0 < before)
                    .take(limit)
                    .cloned()
                    .collect();
                assert_eq!(
                    actual, expected,
                    "format={title_format} before={before} limit={limit}"
                );
            }
        }
        store.close();
    }
}

#[test]
fn scroll_refills_across_failed_windows_and_preserves_through_count() {
    for through in [None, Some(20000108)] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("journals")).unwrap();
        let path = |n| root.path().join(format!("journals/2000_01_{n:02}.md"));
        for n in 1..=12 {
            std::fs::write(path(n), "- body\n").unwrap();
        }
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        store.stop_watcher_for_tests();
        store.whole_graph().unwrap();
        for n in 8..=12 {
            std::fs::write(path(n), [0xff]).unwrap();
        }
        std::fs::remove_file(path(7)).unwrap();
        let page = feed_page_through(&store, 3, Some(20000113), through, true).unwrap();
        let ids: Vec<_> = page.pages.iter().map(|p| p.id.as_str()).collect();
        let expected = if through.is_some() {
            vec![6, 5, 4, 3, 2]
        } else {
            vec![6, 5, 4]
        };
        assert_eq!(
            ids,
            expected
                .iter()
                .map(|n| format!("journals/2000_01_{n:02}.md"))
                .collect::<Vec<_>>()
        );
        assert_eq!(page.unreadable.len(), 5);
        assert_eq!(
            page.next_before_day,
            Some(20000100 + expected.last().unwrap())
        );
        assert!(!page.done);
        store.close();
    }
}

#[test]
fn gf4_ready_feed_matches_legacy_days_bodies_and_cursors() {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals"] {
        std::fs::create_dir(root.path().join(dir)).unwrap();
    }
    let today = JournalDate::today();
    let fmt = JournalFormat::new(None, None);
    for (date, body) in [
        (today, ""),
        (JournalDate::from_ordinal(20000101), "- old body\n"),
        (JournalDate::from_ordinal(20990103), "- future body\n"),
    ] {
        std::fs::write(
            root.path()
                .join(format!("journals/{}.md", fmt.file_stem(date))),
            body,
        )
        .unwrap();
    }
    std::fs::write(
        root.path().join("pages/Plan.md"),
        "- [[2000-01-03]] [[2000-01-02]] [[2099-01-03]] [[2099-01-01]]\n",
    )
    .unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let initial = feed_page(&store, 20, None).unwrap();
    assert!(
        initial.references_pending,
        "first request must leave graph additions to completion even when Ready"
    );
    assert_eq!(
        initial.pages.len(),
        2,
        "first request reads saved days only"
    );
    // Frozen pre-GF4 algorithm: blocking inventory, descending union,
    // direct body reads with virtual fallback, and first-page Upcoming.
    let legacy = |limit, before_day| {
        let as_of_day = today.ordinal_key();
        let (entries, unreadable) = feed_days_through(&store, Day(as_of_day)).unwrap();
        let referenced: std::collections::BTreeSet<_> = store
            .whole_graph()
            .unwrap()
            .inventory(tine_store::InventoryScope::ReferencedJournals)
            .0
            .iter()
            .filter_map(|e| e.day)
            .collect();
        let mut days: BTreeMap<_, _> = entries.into_iter().collect();
        for day in referenced.iter().filter(|d| d.0 <= as_of_day) {
            days.entry(*day)
                .or_insert_with(|| store.journal_id(*day, false));
        }
        let by_id: BTreeMap<_, _> = days.iter().map(|(day, id)| (id.clone(), *day)).collect();
        let mut page = collect_feed_page(
            days.into_iter().rev().collect(),
            limit,
            before_day,
            as_of_day,
            |id| {
                Ok(match store.page(id) {
                    Ok(read) => Loaded::Page(feed_read(read)),
                    Err(StoreError::NotFound(_)) if referenced.contains(&by_id[id]) => {
                        Loaded::Page(virtual_day(&store, by_id[id]))
                    }
                    Err(StoreError::NotFound(_)) => Loaded::Gone,
                    Err(error) => Loaded::Unreadable(store_error(error).to_string()),
                })
            },
        )
        .unwrap();
        if before_day.is_none() {
            page.unreadable.splice(0..0, unreadable);
            page.upcoming = referenced
                .into_iter()
                .filter(|d| d.0 > as_of_day)
                .map(|d| virtual_day(&store, d))
                .collect();
        }
        page
    };
    let wire = |page: FeedPage<FeedRead>| {
        serde_json::json!({
            "pages": page.pages.into_iter().map(|p| (p.id.as_str().to_owned(), p.doc)).collect::<Vec<_>>(),
            "upcoming": page.upcoming.into_iter().map(|p| (p.id.as_str().to_owned(), p.doc)).collect::<Vec<_>>(),
            "cursor": page.next_before_day, "done": page.done, "as_of_day": page.as_of_day, "unreadable": page.unreadable,
        })
    };
    for limit in [0, 1, 3, 20] {
        for before in [
            None,
            Some(today.ordinal_key()),
            Some(20000103),
            Some(20000101),
        ] {
            let new = feed_page_through(&store, limit, before, None, true).unwrap();
            assert!(!new.references_pending);
            assert_eq!(wire(new), wire(legacy(limit, before)));
        }
    }
    let extended = feed_page_through(&store, 1, None, Some(20000101), true).unwrap();
    assert_eq!(extended.pages.len(), 4);
    assert!(extended.done);
    store.close();
}

/// Cost audit 2026-11-02 row 8 (D-10): a Journals scroll step on a loaded
/// graph pages through the view's day index, never the journals directory,
/// and answers exactly what the per-step listing answered; the first page
/// still lists the directory once (unreadable entries, unpublished files).
#[test]
fn scroll_steps_page_from_the_day_index_and_match_the_listing() {
    for journals_enabled in [true, false] {
        let root = tempfile::tempdir().unwrap();
        for dir in ["pages", "journals", "journals/archive", "logseq"] {
            std::fs::create_dir_all(root.path().join(dir)).unwrap();
        }
        if !journals_enabled {
            std::fs::write(
                root.path().join("logseq/config.edn"),
                "{:feature/enable-journals? false}\n",
            )
            .unwrap();
        }
        let fmt = JournalFormat::new(None, None);
        let mut date = JournalDate::from_ordinal(20000101);
        for i in 0..120 {
            // Gaps: every third day has no file.
            if i % 3 != 2 {
                std::fs::write(
                    root.path()
                        .join(format!("journals/{}.md", fmt.file_stem(date))),
                    format!("- day {i}\n"),
                )
                .unwrap();
            }
            date = date.add_days(1);
        }
        let write = |rel: &str, body: &[u8]| std::fs::write(root.path().join(rel), body).unwrap();
        // A journal in a subdirectory, a duplicate-day spelling, an
        // unparseable name, a hidden file, a non-text file and a future day.
        write("journals/archive/1999_12_30.md", b"- archived\n");
        write("journals/2000-01-02.md", b"- dash twin\n");
        write("journals/notes.md", b"- not a day\n");
        write("journals/.1999_12_01.md", b"- hidden\n");
        write("journals/1999_12_02.txt", b"- not text\n");
        write("journals/2099_01_01.md", b"- future\n");
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            std::fs::write(
                root.path()
                    .join("journals")
                    .join(std::ffi::OsStr::from_bytes(b"1999_11_\xff.md")),
                "- bad name\n",
            )
            .unwrap();
        }
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let wire = |page: FeedPage<FeedRead>| {
            serde_json::json!({
                "pages": page.pages.into_iter().map(|p| (p.id.as_str().to_owned(), p.doc)).collect::<Vec<_>>(),
                "upcoming": page.upcoming.into_iter().map(|p| (p.id.as_str().to_owned(), p.doc)).collect::<Vec<_>>(),
                "cursor": page.next_before_day, "done": page.done, "unreadable": page.unreadable,
            })
        };
        // The former step: the same request answered from the listing.
        let legacy_step = |limit, before: i64| {
            let as_of_day = JournalDate::today().ordinal_key();
            let (entries, _) = feed_days_through(&store, Day(as_of_day)).unwrap();
            let days: BTreeMap<_, _> = entries.into_iter().collect();
            collect_feed_page(
                days.into_iter().rev().collect(),
                limit,
                Some(before),
                as_of_day,
                |id| {
                    Ok(match store.page(id) {
                        Ok(read) => Loaded::Page(feed_read(read)),
                        Err(StoreError::NotFound(_)) => Loaded::Gone,
                        Err(error) => Loaded::Unreadable(store_error(error).to_string()),
                    })
                },
            )
            .unwrap()
        };
        let listed = journal_listings();
        let opened = feed_page_through(&store, 3, None, None, true).unwrap();
        assert_eq!(journal_listings() - listed, 1, "the first page lists once");
        assert_eq!(
            opened.unreadable.len(),
            usize::from(cfg!(unix)),
            "{:?}",
            opened.unreadable
        );
        let mut cursor = opened.next_before_day;
        let mut steps = 0;
        while let Some(before) = cursor {
            let listed = journal_listings();
            let step = feed_page_through(&store, 3, Some(before), None, true).unwrap();
            assert_eq!(
                journal_listings() - listed,
                0,
                "a scroll step reads no directory (journals enabled: {journals_enabled})"
            );
            cursor = step.next_before_day;
            assert_eq!(
                wire(step),
                wire(legacy_step(3, before)),
                "step before {before}"
            );
            steps += 1;
        }
        assert!(
            steps > 20,
            "the feed paged through the whole history ({steps} steps)"
        );

        // A journal written by another program reaches the view by the
        // ordinary rescan, and the next step shows it.
        write("journals/1999_06_15.md", b"- appeared\n");
        store.refresh(tine_store::Depth::Stamps).unwrap();
        let step = feed_page_through(&store, 200, Some(19990701), None, true).unwrap();
        assert_eq!(wire(step), wire(legacy_step(200, 19990701)));
        store.close();
    }
}

/// Before/after numbers for row 8 on a generated graph
/// (`scripts/generate-realistic-graph.mjs`), never a real one:
/// `TINE_PERF_GRAPH=<root> cargo test -p tine-graph-features --lib
/// realistic_graph_scroll_steps -- --ignored --nocapture`.
#[test]
#[ignore = "measurement on a generated graph named by TINE_PERF_GRAPH"]
fn realistic_graph_scroll_steps() {
    let Ok(root) = std::env::var("TINE_PERF_GRAPH") else {
        return;
    };
    let store = Store::open(std::path::Path::new(&root), Default::default())
        .unwrap()
        .0;
    store.whole_graph().unwrap();
    let as_of_day = JournalDate::today().ordinal_key();
    let opened = feed_page_through(&store, 3, None, None, true).unwrap();
    let mut cursors = Vec::new();
    let mut cursor = opened.next_before_day;
    while let Some(before) = cursor {
        if cursors.len() == 40 {
            break;
        }
        cursors.push(before);
        cursor = feed_page_through(&store, 3, Some(before), None, true)
            .unwrap()
            .next_before_day;
    }
    // The former step's day list: a directory listing per step.
    let t = std::time::Instant::now();
    for _ in &cursors {
        std::hint::black_box(feed_days_through(&store, Day(as_of_day)).unwrap());
    }
    let old_days = t.elapsed() / cursors.len() as u32;
    let t = std::time::Instant::now();
    for &before in &cursors {
        let (entries, _) = feed_days_through(&store, Day(as_of_day)).unwrap();
        let days: BTreeMap<_, _> = entries.into_iter().collect();
        let page = collect_feed_page(
            days.into_iter().rev().collect(),
            3,
            Some(before),
            as_of_day,
            |id| {
                Ok(match store.page(id) {
                    Ok(read) => Loaded::Page(feed_read(read)),
                    Err(StoreError::NotFound(_)) => Loaded::Gone,
                    Err(error) => Loaded::Unreadable(store_error(error).to_string()),
                })
            },
        )
        .unwrap();
        std::hint::black_box(page);
    }
    let old_step = t.elapsed() / cursors.len() as u32;
    let graph = store.whole_graph().unwrap();
    let t = std::time::Instant::now();
    for &before in &cursors {
        std::hint::black_box(
            graph.inventory(tine_store::InventoryScope::JournalFilesBefore {
                before: Day(before),
                limit: 4,
            }),
        );
    }
    let new_days = t.elapsed() / cursors.len() as u32;
    drop(graph);
    let listed = journal_listings();
    let t = std::time::Instant::now();
    for &before in &cursors {
        std::hint::black_box(feed_page_through(&store, 3, Some(before), None, true).unwrap());
    }
    let new_step = t.elapsed() / cursors.len() as u32;
    println!(
        "8 scroll step: {} steps; old day list (directory listing) {old_days:?}/step, old step (listing + 3 page reads) {old_step:?}/step; new day list (day index) {new_days:?}/step; whole new step (day index + referenced-journal inventory + 3 page reads) {new_step:?}/step; listings during steps {}",
        cursors.len(),
        journal_listings() - listed
    );
    store.close();
}

/// Audit row 9 numbers on a generated graph, never a real one: what one
/// Journals feed restart costs, the reads `Page.tsx` `restartJournalFeed` and
/// `completeJournalFeed` issue after a journal edit fired `days` (first page
/// without references, then the reference-inclusive merge through the oldest
/// loaded day). `TINE_PERF_GRAPH=<root> cargo test --release -p
/// tine-graph-features --lib realistic_graph_feed_restart -- --ignored
/// --nocapture`.
#[test]
#[ignore = "measurement on a generated graph named by TINE_PERF_GRAPH"]
fn realistic_graph_feed_restart() {
    let Ok(root) = std::env::var("TINE_PERF_GRAPH") else {
        return;
    };
    let store = Store::open(std::path::Path::new(&root), Default::default())
        .unwrap()
        .0;
    store.whole_graph().unwrap();
    let opened = feed_page_through(&store, 3, None, None, true).unwrap();
    let oldest = opened.pages.last().map(|page| page.doc.name.clone());
    let through = oldest
        .and_then(|name| format(&store).parse(&name))
        .map(|day| day.ordinal_key());
    let rounds = 9;
    let listed = journal_listings();
    let t = std::time::Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(feed_page_through(&store, 3, None, None, false).unwrap());
        std::hint::black_box(feed_page_through(&store, 3, None, through, true).unwrap());
    }
    println!(
        "9 feed restart: {:?}/restart (first page + reference merge), {} journal listings per restart",
        t.elapsed() / rounds,
        (journal_listings() - listed) / rounds as usize
    );
    store.close();
}
