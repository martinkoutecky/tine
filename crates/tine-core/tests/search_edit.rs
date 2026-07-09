//! Regression: full-text search reflects a marker toggle once the edited page is
//! saved back (the path a {{query}}-result edit takes).
use tine_core::model::atomic_copy;
use tine_core::{Graph, PageKind};

fn mk(tag: &str) -> std::path::PathBuf {
    // Unique per test (pid + tag) so parallel tests don't share a dir.
    let root = std::env::temp_dir().join(format!("tine-se-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    root
}

// `(sort-by priority)` must sort the WHOLE result set, not within each page — so
// priority-A tasks float to the top no matter which page they're on. Read the
// global order ACROSS blocks (a sort may coalesce adjacent same-page results into
// one group — see sort_coalesces_consecutive_same_page_results).
#[test]
fn sort_by_priority_is_global_across_pages() {
    let root = mk("sortprio");
    std::fs::write(
        root.join("pages").join("P1.md"),
        "- TODO [#C] c-one\n- TODO [#A] a-one\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("P2.md"),
        "- TODO [#B] b-two\n- TODO [#A] a-two\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let prios = |q: &str| -> Vec<char> {
        g.run_query(q)
            .iter()
            .flat_map(|grp| grp.blocks.iter())
            .map(|b| {
                b.raw[b.raw.find("[#").unwrap() + 2..]
                    .chars()
                    .next()
                    .unwrap()
            })
            .collect()
    };
    assert_eq!(
        prios("(and (task TODO) (sort-by priority asc))"),
        vec!['A', 'A', 'B', 'C'],
        "A floats to the top globally"
    );
    assert_eq!(
        prios("(and (task TODO) (sort-by priority desc))"),
        vec!['C', 'B', 'A', 'A'],
        "descending sinks A to the bottom"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// `(sort-by modified …)` orders results on ONE recency axis: journal pages by the
// day they represent (stable — NOT their file mtime), other pages by file mtime.
// A page written "now" (>> any 2020 journal day) is the most recent, so `desc`
// (the "Newest first" preset) floats it above the journal days, newest journal
// next — journal and non-journal todos interleaved on a single timeline.
#[test]
fn sort_by_modified_interleaves_journal_and_pages() {
    let root = mk("sortmod");
    std::fs::write(
        root.join("journals").join("2020_01_01.md"),
        "- TODO j-old\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2020_01_02.md"),
        "- TODO j-new\n",
    )
    .unwrap();
    std::fs::write(root.join("pages").join("Proj.md"), "- TODO p-now\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let tag = |grp: &tine_core::RefGroup| {
        grp.blocks[0]
            .raw
            .split_whitespace()
            .last()
            .unwrap()
            .to_string()
    };
    let desc: Vec<String> = g
        .run_query("(and (task TODO) (sort-by modified desc))")
        .iter()
        .map(tag)
        .collect();
    assert_eq!(
        desc,
        vec!["p-now", "j-new", "j-old"],
        "newest first: page(mtime now) > 2020-01-02 > 2020-01-01"
    );
    let asc: Vec<String> = g
        .run_query("(and (task TODO) (sort-by modified asc))")
        .iter()
        .map(tag)
        .collect();
    assert_eq!(
        asc,
        vec!["j-old", "j-new", "p-now"],
        "oldest first reverses"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// `(sort-by deadline)` orders by the DEADLINE planning date — soonest first in
// ascending order; tasks without a deadline sort last (the `~` sentinel). All three
// live on one page, so they coalesce under a single heading (one group, blocks in
// sorted order) — read across blocks, not groups.
#[test]
fn sort_by_deadline_soonest_first() {
    let root = mk("sortdead");
    std::fs::write(
        root.join("pages").join("D.md"),
        "- TODO later\n  DEADLINE: <2026-12-01 Tue>\n- TODO soon\n  DEADLINE: <2026-01-05 Mon>\n- TODO none\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let groups = g.run_query("(and (task TODO) (sort-by deadline asc))");
    assert_eq!(groups.len(), 1, "same-page results share one heading");
    let asc: Vec<String> = groups[0]
        .blocks
        .iter()
        .map(|b| {
            b.raw
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .last()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        asc,
        vec!["soon", "later", "none"],
        "soonest deadline first; no-deadline last"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// The user's case (Jul 4 2026): several matching todos on ONE journal day must
// render under a SINGLE page heading, not repeat it per block — and a sort keeps
// document order within that page.
#[test]
fn sort_coalesces_consecutive_same_page_results() {
    let root = mk("sortcoalesce");
    std::fs::write(
        root.join("journals").join("2020_02_02.md"),
        "- TODO a1\n- TODO a2\n- TODO a3\n",
    )
    .unwrap();
    std::fs::write(root.join("journals").join("2020_01_01.md"), "- TODO b1\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let groups = g.run_query("(and (task TODO) (sort-by modified desc))");
    assert_eq!(
        groups.len(),
        2,
        "consecutive same-page results share one heading (2, not 4)"
    );
    assert_eq!(
        groups[0].blocks.len(),
        3,
        "the 3 same-day todos are under one group"
    );
    let first_day: Vec<String> = groups[0]
        .blocks
        .iter()
        .map(|b| b.raw.split_whitespace().last().unwrap().to_string())
        .collect();
    assert_eq!(
        first_day,
        vec!["a1", "a2", "a3"],
        "within-page document order kept even under desc"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// Coalescing merges only ADJACENT same-page runs: a page whose blocks sort to
// DIFFERENT positions (an A and a C task under a priority sort) still appears at
// each rank — it is not collapsed into one heading.
#[test]
fn sort_does_not_over_merge_nonadjacent_same_page() {
    let root = mk("sortsplit");
    std::fs::write(
        root.join("pages").join("P.md"),
        "- TODO [#A] pa\n- TODO [#C] pc\n",
    )
    .unwrap();
    std::fs::write(root.join("pages").join("Q.md"), "- TODO [#B] qb\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let groups = g.run_query("(and (task TODO) (sort-by priority asc))");
    let seq: Vec<(String, usize)> = groups
        .iter()
        .map(|g| (g.page.clone(), g.blocks.len()))
        .collect();
    assert_eq!(
        seq,
        vec![
            ("P".to_string(), 1),
            ("Q".to_string(), 1),
            ("P".to_string(), 1)
        ],
        "P split across its A and C ranks; only adjacent same-page runs merge"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// Setting the first day of week writes config.edn `:start-of-week` (Logseq
// convention, 0=Monday … 6=Sunday) and round-trips on reopen — replacing an
// existing value and inserting when the key is absent.
#[test]
fn set_start_of_week_round_trips_through_config_edn() {
    let root = mk("sow");
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::write(
        root.join("logseq").join("config.edn"),
        "{:start-of-week 6}\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    assert_eq!(g.meta().start_of_week, 6);
    g.set_start_of_week(0).expect("write start-of-week");
    assert_eq!(
        Graph::open(&root).meta().start_of_week,
        0,
        "0 (Monday) persisted"
    );

    // Insert into a config that has no :start-of-week key yet.
    std::fs::write(
        root.join("logseq").join("config.edn"),
        "{:preferred-workflow :todo}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_start_of_week(2)
        .expect("insert start-of-week");
    let g2 = Graph::open(&root);
    assert_eq!(g2.meta().start_of_week, 2);
    assert_eq!(
        g2.meta().preferred_workflow,
        "todo",
        "existing key preserved"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_reflects_toggle_on_named_page() {
    let root = mk("named");
    std::fs::write(
        root.join("pages").join("Tasks.md"),
        "- TODO ship the thing\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let mut dto = g.load_named("Tasks", PageKind::Page).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DOING");
    g.save_page(&dto, dto.rev.as_deref()).expect("save");
    assert!(
        !g.search("DOING", 20).is_empty(),
        "named: DOING found after save"
    );
    assert!(
        g.search("TODO", 20).is_empty(),
        "named: TODO gone after toggle"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_reflects_toggle_on_journal_page() {
    let root = mk("journal");
    std::fs::write(
        root.join("journals").join("2026_06_16.md"),
        "- TODO ship the thing\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let title = g.run_query("(task TODO)")[0].page.clone();
    let mut dto = g.load_named(&title, PageKind::Journal).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DOING");
    g.save_page(&dto, dto.rev.as_deref()).expect("journal save");
    assert!(
        !g.search("DOING", 20).is_empty(),
        "journal: DOING found after save"
    );
    assert!(
        g.search("TODO", 20).is_empty(),
        "journal: TODO gone after toggle"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// The watcher must recognize Tine's own writes: after save_page, sync_file on
// journals_desc reads from the warmed cache (perf); a brand-new journal created
// after warming must still appear in the feed — its cache entry must carry a
// date_key, else today's freshly-created page would silently vanish.
#[test]
fn new_journal_appears_in_journals_desc_via_cache() {
    let root = mk("newjournal");
    std::fs::write(root.join("journals").join("2026_06_16.md"), "- old day\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache(); // build the cache BEFORE the new journal exists
    assert_eq!(g.journals_desc().len(), 1);

    let dto = tine_core::model::PageDto {
        name: "Jun 18th, 2026".into(),
        kind: PageKind::Journal,
        title: "Jun 18th, 2026".into(),
        pre_block: None,
        blocks: vec![tine_core::model::BlockDto {
            raw: "a new task".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        guide: false,
    };
    g.save_page(&dto, None).expect("save new journal");

    let js = g.journals_desc();
    assert_eq!(
        js.len(),
        2,
        "the freshly-created journal must appear in the feed"
    );
    assert_eq!(js[0].name, "Jun 18th, 2026", "newest day sorts first");
    let _ = std::fs::remove_dir_all(&root);
}

// that file returns None (no phantom external-change → no false conflict).
#[test]
fn own_write_is_suppressed_by_watcher() {
    let root = mk("selfwrite");
    let path = root.join("pages").join("Notes.md");
    // A block WITHOUT id:: (the common case): cache uuid is generated, disk has none.
    std::fs::write(&path, "- TODO ship the thing\n- another line\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();

    // Before any edit, an unchanged file is already suppressed.
    assert!(g.sync_file(&path).is_none(), "unchanged file → suppressed");

    // Edit + save through the normal path.
    let mut dto = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DOING");
    g.save_page(&dto, dto.rev.as_deref()).expect("save");

    // The watcher polling this file must see it as OUR write, not external.
    assert!(
        g.sync_file(&path).is_none(),
        "own write → suppressed (no phantom graph-changed)"
    );

    // A genuine external change is still detected.
    std::fs::write(&path, "- DOING ship the thing\n- edited by hand\n").unwrap();
    assert!(g.sync_file(&path).is_some(), "external edit → detected");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn journal_content_days_distinguishes_empty() {
    let root = mk("contentdays");
    // Non-empty journal, an empty one (placeholder bullet), and a props-only one.
    std::fs::write(
        root.join("journals").join("2026_06_16.md"),
        "- went for a walk\n",
    )
    .unwrap();
    std::fs::write(root.join("journals").join("2026_06_15.md"), "- \n").unwrap();
    std::fs::write(root.join("journals").join("2026_06_14.md"), "title:: x\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let days = g.journal_content_days();
    assert!(days.contains(&20260616), "non-empty day present: {days:?}");
    assert!(!days.contains(&20260615), "empty bullet day absent");
    assert!(!days.contains(&20260614), "props-only day absent");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn frontend_added_id_survives_reload_and_resolves() {
    let root = mk("idpersist");
    std::fs::write(
        root.join("pages").join("TODOs.md"),
        "- {{query (task TODO)}}\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    g.warm_cache();

    // Simulate the frontend save: load the page, append `id:: <uuid>` to the
    // query block's raw (what ensureStableBlockId does), save back.
    let mut dto = g.load_named("TODOs", PageKind::Page).unwrap().unwrap();
    let uuid = dto.blocks[0].id.clone();
    eprintln!("store uuid = {uuid}");
    dto.blocks[0].raw = format!("{}\nid:: {}", dto.blocks[0].raw, uuid);
    g.save_page(&dto, dto.rev.as_deref()).expect("save");

    eprintln!(
        "--- file on disk ---\n{}",
        std::fs::read_to_string(root.join("pages").join("TODOs.md")).unwrap()
    );

    // Reopen from scratch (fresh process would do this).
    let g2 = Graph::open(&root);
    g2.warm_cache();
    let dto2 = g2.load_named("TODOs", PageKind::Page).unwrap().unwrap();
    eprintln!("reloaded uuid = {}", dto2.blocks[0].id);
    assert_eq!(
        dto2.blocks[0].id, uuid,
        "block uuid stable across reload via id::"
    );
    assert!(
        g2.resolve_block(&uuid).is_some(),
        "resolve_block finds it by id::"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn memoized_query_and_backlinks_invalidate_after_edit() {
    let root = mk("querycache");
    std::fs::write(root.join("pages").join("Tasks.md"), "- TODO a\n- DONE b\n").unwrap();
    std::fs::write(root.join("pages").join("Note.md"), "- see [[Tasks]]\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let total = |r: &[tine_core::RefGroup]| r.iter().map(|g| g.blocks.len()).sum::<usize>();

    // Prime the memo: one open TODO, one backlink to Tasks.
    assert_eq!(total(&g.run_query("(task TODO)")), 1);
    assert_eq!(total(&g.backlinks("Tasks")), 1);

    // Edit Tasks (flip DONE→TODO) and Note (drop the [[Tasks]] link) via saves.
    let mut tasks = g.load_named("Tasks", PageKind::Page).unwrap().unwrap();
    tasks.blocks[1].raw = "TODO b".into();
    g.save_page(&tasks, tasks.rev.as_deref()).unwrap();
    let mut note = g.load_named("Note", PageKind::Page).unwrap().unwrap();
    note.blocks[0].raw = "no link anymore".into();
    g.save_page(&note, note.rev.as_deref()).unwrap();

    // The memo MUST reflect the edits, not serve the primed results.
    assert_eq!(
        total(&g.run_query("(task TODO)")),
        2,
        "query must see the flipped task"
    );
    assert_eq!(
        total(&g.backlinks("Tasks")),
        0,
        "backlinks must see the removed link"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn write_highlights_preserves_externally_added_ones() {
    use tine_core::pdf::{Highlight, Position, Rect};
    let root = mk("hlmerge");
    let g = Graph::open(&root);
    let mk_hl = |id: &str, text: &str| {
        let r = Rect {
            top: 0.0,
            left: 0.0,
            width: 1.0,
            height: 1.0,
        };
        Highlight {
            id: id.into(),
            page: 1,
            position: Position {
                page: 1,
                bounding: r.clone(),
                rects: vec![r],
            },
            color: "yellow".into(),
            text: Some(text.into()),
            image: None,
        }
    };
    let ids = |g: &Graph| -> std::collections::HashSet<String> {
        g.read_highlights("paper.pdf")
            .into_iter()
            .map(|h| h.id)
            .collect()
    };
    // Tine writes H1 (no baseline yet).
    g.write_highlights("paper.pdf", "Paper", &[mk_hl("H1", "one")], &[])
        .unwrap();
    // An external editor (OG) adds H2 to the same EDN.
    let edn_path = root
        .join("assets")
        .join(format!("{}.edn", tine_core::pdf::asset_key("paper.pdf")));
    let mut both = tine_core::pdf::parse_highlights(&std::fs::read_to_string(&edn_path).unwrap());
    both.push(mk_hl("H2", "two"));
    std::fs::write(&edn_path, tine_core::pdf::write_highlights(&both, "")).unwrap();
    // Tine, baseline [H1], adds H3 and writes — H2 (external) must NOT be dropped.
    g.write_highlights(
        "paper.pdf",
        "Paper",
        &[mk_hl("H1", "one"), mk_hl("H3", "three")],
        &["H1".into()],
    )
    .unwrap();
    assert!(
        ids(&g).is_superset(&["H1", "H2", "H3"].map(String::from).into_iter().collect()),
        "got {:?}",
        ids(&g)
    );

    // Now DELETE H2: baseline is everything currently on disk; current omits H2.
    let base: Vec<String> = ids(&g).into_iter().collect();
    g.write_highlights(
        "paper.pdf",
        "Paper",
        &[mk_hl("H1", "one"), mk_hl("H3", "three")],
        &base,
    )
    .unwrap();
    let after = ids(&g);
    assert!(
        !after.contains("H2"),
        "deleted highlight must stay deleted: {after:?}"
    );
    assert!(
        after.contains("H1") && after.contains("H3"),
        "kept ones must survive: {after:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn highlight_write_is_not_seen_as_external_change() {
    // Saving a highlight rewrites the hls__ notes page, which is a normal watched
    // page. A watcher poll after the write must not raise a false "changed on disk"
    // against it — post-write, disk_revs reflects the write and suppresses the poll.
    use tine_core::pdf::{asset_key, hls_page_name, Highlight, Position, Rect};
    let root = mk("hlself");
    let g = Graph::open(&root);
    g.search("x", 10); // build the cache

    let r = Rect {
        top: 0.0,
        left: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let h = Highlight {
        id: "H1".into(),
        page: 1,
        position: Position {
            page: 1,
            bounding: r.clone(),
            rects: vec![r],
        },
        color: "yellow".into(),
        text: Some("noted".into()),
        image: None,
    };
    g.write_highlights("paper.pdf", "Paper", &[h], &[]).unwrap();

    let hls_path = root
        .join("pages")
        .join(format!("{}.md", hls_page_name(&asset_key("paper.pdf"))));
    assert!(
        g.sync_file(&hls_path).is_none(),
        "highlight write must not be reported as an external change"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn list_pages_memo_reflects_new_and_deleted_pages() {
    let root = mk("listmemo");
    std::fs::write(root.join("pages").join("A.md"), "- a\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let names = |g: &Graph| {
        let mut v: Vec<String> = g.list_pages().into_iter().map(|e| e.name).collect();
        v.sort();
        v
    };
    assert_eq!(names(&g), vec!["A"]); // primes the memo

    // Create B via a save (cache_upsert bumps cache_gen → memo invalidates). A
    // brand-new page carries no path, so the save resolves the file by name (a
    // loaded page keeps its own path and saves back to that file, #21).
    let mut b = g.load_named("A", PageKind::Page).unwrap().unwrap();
    b.name = "B".into();
    b.title = "B".into();
    b.rev = None;
    b.path = String::new();
    g.save_page(&b, None).unwrap();
    assert!(
        names(&g).contains(&"B".to_string()),
        "new page must appear: {:?}",
        names(&g)
    );

    // Delete A → memo must drop it.
    g.delete_page("A", PageKind::Page).unwrap();
    assert!(
        !names(&g).contains(&"A".to_string()),
        "deleted page must disappear: {:?}",
        names(&g)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn delete_page_moves_to_trash_recoverable() {
    let root = mk("deltrash");
    std::fs::write(root.join("pages").join("Doomed.md"), "- keep me\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    g.delete_page("Doomed", PageKind::Page).expect("delete");

    // Gone from pages/, no longer resolvable...
    assert!(!root.join("pages").join("Doomed.md").exists());
    assert!(g.load_named("Doomed", PageKind::Page).unwrap().is_none());
    // ...but recoverable from the local trash (content intact).
    let trash = root.join("logseq").join(".tine-trash");
    let trashed: Vec<_> = std::fs::read_dir(&trash).unwrap().flatten().collect();
    assert_eq!(trashed.len(), 1, "deleted file should be in the trash");
    let body = std::fs::read_to_string(trashed[0].path()).unwrap();
    assert!(body.contains("keep me"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn delete_page_errors_when_trash_path_is_file_and_keeps_page_cached() {
    let root = mk("deltrash-blocked");
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::write(root.join("logseq").join(".tine-trash"), "not a dir").unwrap();
    std::fs::write(root.join("pages").join("Doomed.md"), "- keep me\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();

    let err = g
        .delete_page("Doomed", PageKind::Page)
        .expect_err("trash path is blocked");
    assert!(
        err.to_string().contains(".tine-trash"),
        "error should name the trash path: {err}"
    );
    assert!(
        root.join("pages").join("Doomed.md").is_file(),
        "source page survives"
    );
    let dto = g
        .load_named("Doomed", PageKind::Page)
        .unwrap()
        .expect("page still loads");
    assert_eq!(dto.blocks[0].raw, "keep me");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn trash_journal_file_errors_when_trash_path_is_file_and_keeps_source() {
    let root = mk("journaltrash-blocked");
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::write(root.join("logseq").join(".tine-trash"), "not a dir").unwrap();
    let journal = root.join("journals").join("2026_06_20.md");
    std::fs::write(&journal, "- journal body\n").unwrap();
    let g = Graph::open(&root);

    let err = g
        .trash_journal_file("2026_06_20.md")
        .expect_err("trash path is blocked");
    assert!(
        err.to_string().contains(".tine-trash"),
        "error should name the trash path: {err}"
    );
    assert!(journal.is_file(), "source journal survives");
    assert_eq!(
        std::fs::read_to_string(&journal).unwrap(),
        "- journal body\n"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn trash_asset_errors_when_trash_path_is_file_and_keeps_source() {
    let root = mk("assettrash-blocked");
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("logseq").join(".tine-trash"), "not a dir").unwrap();
    let asset = root.join("assets").join("clip.png");
    std::fs::write(&asset, b"asset bytes").unwrap();
    let g = Graph::open(&root);

    let err = g
        .trash_asset("clip.png")
        .expect_err("trash path is blocked");
    assert!(
        err.to_string().contains(".tine-trash"),
        "error should name the trash path: {err}"
    );
    assert!(asset.is_file(), "source asset survives");
    assert_eq!(std::fs::read(&asset).unwrap(), b"asset bytes");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn atomic_copy_is_public_and_replaces_destination_contents() {
    let root = mk("atomic-copy");
    let src = root.join("pages").join("source.edn");
    let dst = root.join("pages").join("dest.edn");
    std::fs::write(&src, "{:ok true}\n").unwrap();
    std::fs::write(&dst, "old").unwrap();

    atomic_copy(&src, &dst).expect("atomic copy");
    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "{:ok true}\n");
    let leftovers: Vec<_> = std::fs::read_dir(root.join("pages"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".import.tmp"))
        .collect();
    assert!(leftovers.is_empty(), "atomic copy temp should not leak");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn page_icons_answer_from_cached_pages_with_page_key_lookup() {
    let root = mk("page-icons-cache");
    std::fs::write(
        root.join("pages").join("IconPage.md"),
        "icon:: star\nalias:: Icon Alias\n- body\n",
    )
    .unwrap();
    std::fs::write(root.join("pages").join("NoIcon.md"), "- body\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    std::fs::rename(root.join("pages"), root.join("pages.offline")).unwrap();

    let icons = g.page_icons(&[
        "iconpage".to_string(),
        "Icon Alias".to_string(),
        "NoIcon".to_string(),
        "Missing".to_string(),
    ]);
    assert_eq!(icons.get("iconpage").map(String::as_str), Some("star"));
    assert_eq!(icons.get("Icon Alias").map(String::as_str), Some("star"));
    assert!(!icons.contains_key("NoIcon"));
    assert!(!icons.contains_key("Missing"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_blocks_uses_indexed_hinted_page_lookup() {
    let src = include_str!("../src/query.rs");
    assert!(
        !src.contains("pages.iter().find(|(e, _)| &e.name == page)"),
        "hinted resolve_blocks lookup must not linearly scan all pages per hinted page"
    );
}

#[test]
fn run_advanced_query_uses_generation_keyed_memo_cache() {
    let src = include_str!("../src/model.rs");
    assert!(
        src.contains("fn advanced_memo("),
        "advanced queries should have a dedicated memo cache"
    );
    assert!(
        !src.contains("Not memoized (invoked on demand)"),
        "stale non-memoized advanced-query comment should be gone"
    );
}

#[test]
fn resolve_block_index_refreshes_after_cache_change() {
    let root = mk("blockidx");
    std::fs::write(
        root.join("pages").join("A.md"),
        "- alpha\n  id:: aaaa-1111\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    // First resolve builds the gen-keyed uuid index.
    assert_eq!(g.resolve_block("aaaa-1111").unwrap().page, "A");

    // A new page appears on disk; invalidate the cache as the watcher would.
    std::fs::write(
        root.join("pages").join("B.md"),
        "- beta\n  id:: bbbb-2222\n",
    )
    .unwrap();
    g.invalidate_cache();
    // The index must rebuild against the fresh cache — not serve a stale "not
    // found" for the new block, nor lose the old one.
    assert_eq!(g.resolve_block("bbbb-2222").unwrap().page, "B");
    assert_eq!(g.resolve_block("aaaa-1111").unwrap().page, "A");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_blocks_batch_resolves_across_pages_with_duplicates() {
    // The batch resolver groups hinted ids by page (each hinted page scanned
    // once) and falls back to a single whole-graph scan for the rest. It must:
    //  - resolve ids that live on different pages,
    //  - resolve several ids on the SAME page,
    //  - resolve an id via its persisted `id::` as well as its assigned uuid,
    //  - return None (not a panic) for an unknown id,
    //  - be positional & per-input (a repeated input uuid resolves each time).
    let root = mk("resolvebatch");
    std::fs::write(
        root.join("pages").join("A.md"),
        "- alpha one\n  id:: aaaa-1111\n- alpha two\n  id:: aaaa-2222\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("B.md"),
        "- beta\n  id:: bbbb-3333\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    g.warm_cache();

    let req = vec![
        "aaaa-1111".to_string(), // page A
        "bbbb-3333".to_string(), // page B
        "aaaa-2222".to_string(), // page A again (same page, second id)
        "nope-0000".to_string(), // unknown
        "aaaa-1111".to_string(), // duplicate input
    ];
    let out = g.resolve_blocks(&req);
    assert_eq!(out.len(), req.len(), "one result slot per input");
    assert_eq!(out[0].as_ref().unwrap().page, "A");
    assert_eq!(
        out[0].as_ref().unwrap().blocks[0].raw,
        "alpha one\nid:: aaaa-1111"
    );
    assert_eq!(out[1].as_ref().unwrap().page, "B");
    assert_eq!(out[2].as_ref().unwrap().page, "A");
    assert_eq!(
        out[2].as_ref().unwrap().blocks[0].raw,
        "alpha two\nid:: aaaa-2222"
    );
    assert!(out[3].is_none(), "unknown id resolves to None, not a panic");
    assert_eq!(
        out[4].as_ref().unwrap().page,
        "A",
        "duplicate input resolves again"
    );

    // Batch agrees with N single resolves (same semantics, just one pass).
    for u in &req {
        assert_eq!(
            g.resolve_blocks(std::slice::from_ref(u))
                .into_iter()
                .next()
                .unwrap()
                .map(|r| r.page),
            g.resolve_block(u).map(|r| r.page),
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_journal_saved_with_date_stem_not_title() {
    let root = mk("journalname");
    let g = Graph::open(&root);
    g.warm_cache();
    // Save a brand-new journal by its title (no file yet).
    let dto = tine_core::model::PageDto {
        name: "Jun 18th, 2026".into(),
        kind: PageKind::Journal,
        title: "Jun 18th, 2026".into(),
        pre_block: None,
        blocks: vec![tine_core::model::BlockDto {
            id: String::new(),
            raw: "TODO carried task".into(),
            collapsed: false,
            children: vec![],
            breadcrumb: vec![],
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        guide: false,
    };
    g.save_page(&dto, None).expect("save new journal");
    // It must land on the date-stem file, and reopening must show it in the feed.
    assert!(
        root.join("journals").join("2026_06_18.md").exists(),
        "stem-named file"
    );
    assert!(
        !root.join("journals").join("Jun 18th, 2026.md").exists(),
        "no title-named file"
    );
    let g2 = Graph::open(&root);
    assert!(
        g2.journals_desc()
            .iter()
            .any(|e| e.name == "Jun 18th, 2026"),
        "new journal appears in the feed after reload"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn migrate_renames_title_named_journal_files() {
    let root = mk("journalmigrate");
    // Simulate a previously-mis-saved journal (title as filename).
    std::fs::write(
        root.join("journals").join("Jun 18th, 2026.md"),
        "- TODO recovered\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    let n = g.migrate_journal_filenames();
    assert_eq!(n, 1);
    assert!(
        root.join("journals").join("2026_06_18.md").exists(),
        "renamed to stem"
    );
    assert!(
        !root.join("journals").join("Jun 18th, 2026.md").exists(),
        "title file gone"
    );
    // Content preserved + now visible.
    assert!(g.journals_desc().iter().any(|e| e.name == "Jun 18th, 2026"));
    let _ = std::fs::remove_dir_all(&root);
}

// CRLF graphs (e.g. a graph edited on Windows) round-trip safely: parsing strips
// the `\r` so it never pollutes content, an UNCHANGED save stays byte-identical
// (no Syncthing churn / no LF flip), and a real edit keeps the file's CRLF.
#[test]
fn crlf_files_round_trip_without_churn() {
    let root = mk("crlf");
    let original = "title:: Win\r\n\r\n- TODO ship it\r\n- second line\r\n";
    let path = root.join("pages").join("Win.md");
    std::fs::write(&path, original).unwrap();
    let g = Graph::open(&root);
    g.warm_cache();

    let dto = g.load_named("Win", PageKind::Page).unwrap().unwrap();
    // (1) no stray CR leaks into the in-memory model
    assert!(
        dto.pre_block.as_deref().map_or(true, |p| !p.contains('\r')),
        "pre-block CR"
    );
    for b in &dto.blocks {
        assert!(!b.raw.contains('\r'), "block raw carries a CR: {:?}", b.raw);
    }
    // (2) an unchanged save is byte-identical — CRLF preserved, no churn
    g.save_page(&dto, dto.rev.as_deref()).expect("no-op save");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        original,
        "unchanged save must keep the exact CRLF bytes"
    );
    // (3) a real edit keeps CRLF (only the changed line differs, no lone LF)
    let mut dto2 = g.load_named("Win", PageKind::Page).unwrap().unwrap();
    dto2.blocks[0].raw = dto2.blocks[0].raw.replace("ship it", "shipped");
    g.save_page(&dto2, dto2.rev.as_deref()).expect("edit save");
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("shipped"), "edit applied: {after:?}");
    assert!(after.contains("\r\n"), "edited file keeps CRLF: {after:?}");
    assert_eq!(
        after.matches('\n').count(),
        after.matches("\r\n").count(),
        "no lone LF mixed in: {after:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
