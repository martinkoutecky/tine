//! Regression: full-text search reflects a marker toggle once the edited page is
//! saved back (the path a {{query}}-result edit takes).
use tine_core::{Graph, PageKind};

fn mk(tag: &str) -> std::path::PathBuf {
    // Unique per test (pid + tag) so parallel tests don't share a dir.
    let root = std::env::temp_dir().join(format!("tine-se-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    root
}

// `(sort-by priority)` must sort the WHOLE result set, not within each page —
// so priority-A tasks float to the top no matter which page they're on. The
// engine returns one block per group (in global order) when a sort is active.
#[test]
fn sort_by_priority_is_global_across_pages() {
    let root = mk("sortprio");
    std::fs::write(root.join("pages").join("P1.md"), "- TODO [#C] c-one\n- TODO [#A] a-one\n").unwrap();
    std::fs::write(root.join("pages").join("P2.md"), "- TODO [#B] b-two\n- TODO [#A] a-two\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let prio = |grp: &tine_core::RefGroup| {
        let raw = &grp.blocks[0].raw;
        raw[raw.find("[#").unwrap() + 2..].chars().next().unwrap()
    };
    let asc: Vec<char> = g.run_query("(and (task TODO) (sort-by priority asc))").iter().map(prio).collect();
    assert_eq!(asc, vec!['A', 'A', 'B', 'C'], "A floats to the top globally (across pages)");
    let desc: Vec<char> = g.run_query("(and (task TODO) (sort-by priority desc))").iter().map(prio).collect();
    assert_eq!(desc, vec!['C', 'B', 'A', 'A'], "descending sinks A to the bottom");
    let _ = std::fs::remove_dir_all(&root);
}

// Setting the first day of week writes config.edn `:start-of-week` (Logseq
// convention, 0=Monday … 6=Sunday) and round-trips on reopen — replacing an
// existing value and inserting when the key is absent.
#[test]
fn set_start_of_week_round_trips_through_config_edn() {
    let root = mk("sow");
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::write(root.join("logseq").join("config.edn"), "{:start-of-week 6}\n").unwrap();
    let g = Graph::open(&root);
    assert_eq!(g.meta().start_of_week, 6);
    g.set_start_of_week(0).expect("write start-of-week");
    assert_eq!(Graph::open(&root).meta().start_of_week, 0, "0 (Monday) persisted");

    // Insert into a config that has no :start-of-week key yet.
    std::fs::write(root.join("logseq").join("config.edn"), "{:preferred-workflow :todo}\n").unwrap();
    Graph::open(&root).set_start_of_week(2).expect("insert start-of-week");
    let g2 = Graph::open(&root);
    assert_eq!(g2.meta().start_of_week, 2);
    assert_eq!(g2.meta().preferred_workflow, "todo", "existing key preserved");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_reflects_toggle_on_named_page() {
    let root = mk("named");
    std::fs::write(root.join("pages").join("Tasks.md"), "- TODO ship the thing\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let mut dto = g.load_named("Tasks", PageKind::Page).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DOING");
    g.save_page(&dto, dto.rev.as_deref()).expect("save");
    assert!(!g.search("DOING", 20).is_empty(), "named: DOING found after save");
    assert!(g.search("TODO", 20).is_empty(), "named: TODO gone after toggle");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_reflects_toggle_on_journal_page() {
    let root = mk("journal");
    std::fs::write(root.join("journals").join("2026_06_16.md"), "- TODO ship the thing\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let title = g.run_query("(task TODO)")[0].page.clone();
    let mut dto = g.load_named(&title, PageKind::Journal).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DOING");
    g.save_page(&dto, dto.rev.as_deref()).expect("journal save");
    assert!(!g.search("DOING", 20).is_empty(), "journal: DOING found after save");
    assert!(g.search("TODO", 20).is_empty(), "journal: TODO gone after toggle");
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
    };
    g.save_page(&dto, None).expect("save new journal");

    let js = g.journals_desc();
    assert_eq!(js.len(), 2, "the freshly-created journal must appear in the feed");
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
    assert!(g.sync_file(&path).is_none(), "own write → suppressed (no phantom graph-changed)");

    // A genuine external change is still detected.
    std::fs::write(&path, "- DOING ship the thing\n- edited by hand\n").unwrap();
    assert!(g.sync_file(&path).is_some(), "external edit → detected");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn journal_content_days_distinguishes_empty() {
    let root = mk("contentdays");
    // Non-empty journal, an empty one (placeholder bullet), and a props-only one.
    std::fs::write(root.join("journals").join("2026_06_16.md"), "- went for a walk\n").unwrap();
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
    std::fs::write(root.join("pages").join("TODOs.md"), "- {{query (task TODO)}}\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();

    // Simulate the frontend save: load the page, append `id:: <uuid>` to the
    // query block's raw (what ensureStableBlockId does), save back.
    let mut dto = g.load_named("TODOs", PageKind::Page).unwrap().unwrap();
    let uuid = dto.blocks[0].id.clone();
    eprintln!("store uuid = {uuid}");
    dto.blocks[0].raw = format!("{}\nid:: {}", dto.blocks[0].raw, uuid);
    g.save_page(&dto, dto.rev.as_deref()).expect("save");

    eprintln!("--- file on disk ---\n{}", std::fs::read_to_string(root.join("pages").join("TODOs.md")).unwrap());

    // Reopen from scratch (fresh process would do this).
    let g2 = Graph::open(&root);
    g2.warm_cache();
    let dto2 = g2.load_named("TODOs", PageKind::Page).unwrap().unwrap();
    eprintln!("reloaded uuid = {}", dto2.blocks[0].id);
    assert_eq!(dto2.blocks[0].id, uuid, "block uuid stable across reload via id::");
    assert!(g2.resolve_block(&uuid).is_some(), "resolve_block finds it by id::");
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
    assert_eq!(total(&g.run_query("(task TODO)")), 2, "query must see the flipped task");
    assert_eq!(total(&g.backlinks("Tasks")), 0, "backlinks must see the removed link");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn write_highlights_preserves_externally_added_ones() {
    use tine_core::pdf::{Highlight, Position, Rect};
    let root = mk("hlmerge");
    let g = Graph::open(&root);
    let mk_hl = |id: &str, text: &str| {
        let r = Rect { top: 0.0, left: 0.0, width: 1.0, height: 1.0 };
        Highlight {
            id: id.into(),
            page: 1,
            position: Position { page: 1, bounding: r.clone(), rects: vec![r] },
            color: "yellow".into(),
            text: Some(text.into()),
            image: None,
        }
    };
    let ids = |g: &Graph| -> std::collections::HashSet<String> {
        g.read_highlights("paper.pdf").into_iter().map(|h| h.id).collect()
    };
    // Tine writes H1 (no baseline yet).
    g.write_highlights("paper.pdf", "Paper", &[mk_hl("H1", "one")], &[]).unwrap();
    // An external editor (OG) adds H2 to the same EDN.
    let edn_path = root.join("assets").join(format!("{}.edn", tine_core::pdf::asset_key("paper.pdf")));
    let mut both = tine_core::pdf::parse_highlights(&std::fs::read_to_string(&edn_path).unwrap());
    both.push(mk_hl("H2", "two"));
    std::fs::write(&edn_path, tine_core::pdf::write_highlights(&both)).unwrap();
    // Tine, baseline [H1], adds H3 and writes — H2 (external) must NOT be dropped.
    g.write_highlights("paper.pdf", "Paper", &[mk_hl("H1", "one"), mk_hl("H3", "three")], &["H1".into()]).unwrap();
    assert!(ids(&g).is_superset(&["H1", "H2", "H3"].map(String::from).into_iter().collect()), "got {:?}", ids(&g));

    // Now DELETE H2: baseline is everything currently on disk; current omits H2.
    let base: Vec<String> = ids(&g).into_iter().collect();
    g.write_highlights("paper.pdf", "Paper", &[mk_hl("H1", "one"), mk_hl("H3", "three")], &base).unwrap();
    let after = ids(&g);
    assert!(!after.contains("H2"), "deleted highlight must stay deleted: {after:?}");
    assert!(after.contains("H1") && after.contains("H3"), "kept ones must survive: {after:?}");
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

    let r = Rect { top: 0.0, left: 0.0, width: 1.0, height: 1.0 };
    let h = Highlight {
        id: "H1".into(),
        page: 1,
        position: Position { page: 1, bounding: r.clone(), rects: vec![r] },
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

    // Create B via a save (cache_upsert bumps cache_gen → memo invalidates).
    let mut b = g.load_named("A", PageKind::Page).unwrap().unwrap();
    b.name = "B".into();
    b.title = "B".into();
    b.rev = None;
    g.save_page(&b, None).unwrap();
    assert!(names(&g).contains(&"B".to_string()), "new page must appear: {:?}", names(&g));

    // Delete A → memo must drop it.
    g.delete_page("A", PageKind::Page).unwrap();
    assert!(!names(&g).contains(&"A".to_string()), "deleted page must disappear: {:?}", names(&g));
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
fn resolve_block_index_refreshes_after_cache_change() {
    let root = mk("blockidx");
    std::fs::write(root.join("pages").join("A.md"), "- alpha\n  id:: aaaa-1111\n").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    // First resolve builds the gen-keyed uuid index.
    assert_eq!(g.resolve_block("aaaa-1111").unwrap().page, "A");

    // A new page appears on disk; invalidate the cache as the watcher would.
    std::fs::write(root.join("pages").join("B.md"), "- beta\n  id:: bbbb-2222\n").unwrap();
    g.invalidate_cache();
    // The index must rebuild against the fresh cache — not serve a stale "not
    // found" for the new block, nor lose the old one.
    assert_eq!(g.resolve_block("bbbb-2222").unwrap().page, "B");
    assert_eq!(g.resolve_block("aaaa-1111").unwrap().page, "A");
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
        }],
        rev: None,
    };
    g.save_page(&dto, None).expect("save new journal");
    // It must land on the date-stem file, and reopening must show it in the feed.
    assert!(root.join("journals").join("2026_06_18.md").exists(), "stem-named file");
    assert!(!root.join("journals").join("Jun 18th, 2026.md").exists(), "no title-named file");
    let g2 = Graph::open(&root);
    assert!(
        g2.journals_desc().iter().any(|e| e.name == "Jun 18th, 2026"),
        "new journal appears in the feed after reload"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn migrate_renames_title_named_journal_files() {
    let root = mk("journalmigrate");
    // Simulate a previously-mis-saved journal (title as filename).
    std::fs::write(root.join("journals").join("Jun 18th, 2026.md"), "- TODO recovered\n").unwrap();
    let g = Graph::open(&root);
    let n = g.migrate_journal_filenames();
    assert_eq!(n, 1);
    assert!(root.join("journals").join("2026_06_18.md").exists(), "renamed to stem");
    assert!(!root.join("journals").join("Jun 18th, 2026.md").exists(), "title file gone");
    // Content preserved + now visible.
    assert!(g.journals_desc().iter().any(|e| e.name == "Jun 18th, 2026"));
    let _ = std::fs::remove_dir_all(&root);
}
