//! GH #543: a cold parse survives unrelated page opens, and a listing with
//! one unreadable page does not reparse the healthy ones.

use super::*;

/// Opening an unchanged page during the cold parse publishes it and moves the
/// cache generation. The parse must still install: discarding it made the
/// next reader parse the whole graph again (GH #543).
#[test]
fn gh543_cold_parse_survives_an_unchanged_page_open() {
    let dir = scratch("gh543-cold-parse-unchanged");
    for index in 0..3 {
        fs::write(
            dir.join("pages").join(format!("Existing{index}.md")),
            "- unchanged\n",
        )
        .unwrap();
    }
    let graph = Arc::new(Graph::open(&dir));
    graph
        .attach_direct_projection(dir.join("private/projection.sqlite"))
        .unwrap();
    let pause = Arc::new(PageBuildTestPause::new());
    *graph.page_build_test.owner_pause.lock().unwrap() = Some(Arc::clone(&pause));
    let warmer = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.warm_cache_cancellable(|| false))
    };
    pause.reached.wait();
    let entry = graph
        .entry_for_path(&dir.join("pages/Existing0.md"))
        .unwrap();
    graph.load_page(&entry).unwrap();
    pause.release.wait();
    let completed = warmer.join().unwrap();
    *graph.page_build_test.owner_pause.lock().unwrap() = None;
    let first_parses = graph.page_build_parses_test();
    graph.with_pages(|_| ());
    let total_parses = graph.page_build_parses_test();
    graph
        .wait_for_direct_projection_for_test(std::time::Duration::from_secs(5))
        .unwrap();
    graph.detach_direct_projection(std::time::Duration::from_secs(5));
    let _ = fs::remove_dir_all(&dir);
    assert!(completed, "the cold pass was discarded");
    assert_eq!(total_parses, first_parses, "a second whole-graph parse ran");
}

/// An edit that lands after the cold parse read the page is real drift: the
/// parse holds the old bytes and must not install over the edit.
#[test]
fn gh543_cold_parse_still_yields_to_an_edit_after_it_read_the_page() {
    let dir = scratch("gh543-cold-parse-edited");
    fs::write(dir.join("pages/Existing.md"), "- unchanged\n").unwrap();
    let graph = Graph::open(&dir);
    let permit = graph.admit_retained_graph_text_writer().unwrap();
    let flight = PageBuildFlight::new(
        graph.cache_generation(),
        graph
            .cache_structural_gen
            .load(std::sync::atomic::Ordering::Acquire),
    );
    let built = graph.load_all_pages_with_permit(&permit);
    drop(permit);
    let entry = graph
        .entry_for_path(&dir.join("pages/Existing.md"))
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let base = page.rev.clone().unwrap();
    page.blocks[0].raw = "changed".into();
    graph.save_page(&page, Some(&base)).unwrap();

    assert_eq!(
        graph.install_built(&flight, built),
        PageCacheInstallOutcome::GenerationDrift
    );
    let _ = fs::remove_dir_all(&dir);
}

/// GH #543 (indexing audit IT-07): with a page that cannot be read and no
/// ready index, listing pages revalidated the failed path by reading and
/// parsing EVERY page, and did so again after every save moved the
/// generation. One unreadable file (a sync mid-delivery, a bad encoding) made
/// each listing a whole-graph parse at 10k pages.
#[test]
fn gh543_listing_with_an_unreadable_page_does_not_reparse_healthy_pages() {
    let dir = scratch("gh543-unreadable-listing");
    for index in 0..6 {
        fs::write(
            dir.join("pages").join(format!("Healthy{index}.md")),
            format!("- healthy {index}\n"),
        )
        .unwrap();
    }
    let database = dir.join("private/projection.sqlite");
    {
        let first = Graph::open(&dir);
        first.attach_direct_projection(database.clone()).unwrap();
        first.warm_cache();
        assert!(first
            .wait_for_direct_projection_for_test(Duration::from_secs(30))
            .is_ok());
        crate::direct_projection::release_projection(&first);
    }
    // Between sessions: one page changes, one becomes unreadable. The reopen
    // keeps the older image and withholds readiness until the unreadable
    // page can join a complete inventory.
    fs::write(
        dir.join("pages/Healthy0.md"),
        "- changed between sessions\n",
    )
    .unwrap();
    fs::write(dir.join("pages/Healthy1.md"), [0xff, 0xfe, 0xfd]).unwrap();
    let graph = Graph::open(&dir);
    graph.attach_direct_projection(database).unwrap();
    graph.warm_cache();
    graph.direct_projection_test().unwrap().wait_drained_test();

    let first_listing = graph.list_pages();
    assert!(first_listing.iter().all(|entry| entry.name != "healthy1"));
    let entry = graph
        .entry_for_path(&dir.join("pages/Healthy2.md"))
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    page.blocks[0].raw = "edited while a sibling is unreadable".into();
    graph.save_page(&page, page.rev.as_deref()).unwrap();

    // The watcher delivers the unreadable file again, rewritten and still bad:
    // it drops the listing memo so the next listing revalidates that path.
    fs::write(dir.join("pages/Healthy1.md"), [0xff, 0xfe]).unwrap();
    graph.sync_file(&dir.join("pages/Healthy1.md"));
    assert!(!graph.direct_projection_ready_test());
    assert_eq!(
        graph.page_index_failures(),
        vec!["pages/Healthy1.md".to_owned()]
    );
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|count| count.set(0));
    let listed = graph.list_pages();
    let parses = GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get);
    let oracle = Graph::open(&dir).list_pages();
    graph.detach_direct_projection(Duration::from_secs(5));
    let names = |entries: &[PageEntry]| {
        let mut names = entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.rel_path.clone()))
            .collect::<Vec<_>>();
        names.sort();
        names
    };
    assert_eq!(
        names(&listed),
        names(&oracle),
        "the exact listing, from the cache"
    );
    assert!(listed.iter().all(|entry| entry.name != "healthy1"));
    assert_eq!(
        parses, 0,
        "one still-unreadable page made the listing reparse {parses} healthy pages"
    );
}
