//! Integration tests against the on-disk demo graph (standard layout).

use tine_core::Graph;
use std::path::PathBuf;

fn demo_graph() -> Graph {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/demo-graph");
    Graph::open(root)
}

#[test]
fn lists_journals_and_pages() {
    let g = demo_graph();
    let pages = g.list_pages();
    assert!(pages.iter().any(|p| p.name == "logseq-claude"));
    let journals = g.journals_desc();
    // Newest first.
    assert_eq!(journals.first().map(|j| j.name.as_str()), Some("Jun 14th, 2026"));
    assert!(journals.len() >= 2);
}

#[test]
fn loads_a_page_with_nesting_and_properties() {
    let g = demo_graph();
    let entry = g.find_entry("logseq-claude", tine_core::PageKind::Page).unwrap();
    let dto = g.load_page(&entry).unwrap();
    assert_eq!(dto.pre_block.as_deref(), Some("title:: logseq-claude\ntags:: project, tooling"));
    // Has a nested child under the first block.
    assert!(dto.blocks[0].children.len() >= 1);
}

#[test]
fn backlinks_to_parameterized_complexity() {
    let g = demo_graph();
    let groups = g.backlinks("parameterized complexity");
    let pages: Vec<&str> = groups.iter().map(|gr| gr.page.as_str()).collect();
    // Referenced from the journal, logseq-claude, and n-fold IP.
    assert!(pages.contains(&"logseq-claude"), "pages: {pages:?}");
    assert!(pages.contains(&"n-fold IP"), "pages: {pages:?}");
}

#[test]
fn publishes_only_public_pages() {
    let root = std::env::temp_dir().join(format!("tine-publish-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("pages").join("Shared.md"),
        "public:: true\n\n- hello, see [[Secret]]\n",
    )
    .unwrap();
    std::fs::write(root.join("pages").join("Secret.md"), "- private notes\n").unwrap();

    let g = Graph::open(&root);
    let (dir, n) = g.publish_html().unwrap();
    assert_eq!(n, 1, "only the public page is published");
    let p = std::fs::read_to_string(format!("{dir}/shared.html")).unwrap();
    assert!(p.contains("<h1 class=\"page\">Shared</h1>"));
    assert!(p.contains("<a class=\"ref\""), "should link [[refs]]");
    // The private page must not be exported.
    assert!(!std::path::Path::new(&format!("{dir}/secret.html")).exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn search_cache_reflects_saves_and_deletes() {
    use tine_core::model::{BlockDto, PageDto, PageKind};

    // Isolated temp graph so we can mutate it freely.
    let root = std::env::temp_dir().join(format!("tine-cache-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages").join("Seed.md"), "- a seed block\n").unwrap();

    let g = Graph::open(&root);
    // Warms the cache on first search.
    assert_eq!(g.search("zonkwort", 10).len(), 0, "token absent initially");

    // Saving a page with the token must be visible to a subsequent search
    // without any disk re-scan (cache upsert).
    let page = PageDto {
        name: "Fresh".into(),
        kind: PageKind::Page,
        title: "Fresh".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "contains zonkwort here".into(),
            ..Default::default()
            }],
        rev: None,
    };
    g.save_page(&page, None).unwrap();
    let hits = g.search("zonkwort", 10);
    assert_eq!(hits.len(), 1, "saved page should be searchable");
    assert_eq!(hits[0].page, "Fresh");

    // Deleting the page removes it from the cache too.
    g.delete_page("Fresh", PageKind::Page).unwrap();
    assert_eq!(g.search("zonkwort", 10).len(), 0, "deleted page should drop out");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn search_ignores_hidden_property_metadata() {
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-search-meta-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);

    // A block whose only occurrence of "qzxmeta" is in a property line (like an
    // id:: uuid or hl-color::) must NOT match — the user can't see it.
    let page = PageDto {
        name: "Meta".into(),
        kind: PageKind::Page,
        title: "Meta".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "a perfectly ordinary block\nsome-prop:: qzxmeta".into(),
            ..Default::default()
        }],
        rev: None,
    };
    g.save_page(&page, None).unwrap();
    assert_eq!(
        g.search("qzxmeta", 10).len(),
        0,
        "token only in a property line should not be a search hit"
    );
    // But the visible body is still searchable.
    assert_eq!(g.search("ordinary", 10).len(), 1, "visible body still matches");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_preserves_file_format_no_churn() {
    use tine_core::model::PageKind;

    let root = std::env::temp_dir().join(format!("tine-fmt-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Logseq style: no trailing newline. Plus one with a newline, and a
    // space-indented file (Tine emits tabs by default — must preserve spaces).
    let no_nl = "- alpha\n\t- beta";
    let with_nl = "- gamma\n";
    let spaces = "- root\n  - two-space child\n    - grandchild";
    std::fs::write(root.join("pages").join("A.md"), no_nl).unwrap();
    std::fs::write(root.join("pages").join("B.md"), with_nl).unwrap();
    std::fs::write(root.join("pages").join("C.md"), spaces).unwrap();

    let g = Graph::open(&root);
    // Load then save unchanged must be byte-identical (no churn): each file's
    // trailing-newline + indent convention is preserved.
    for name in ["A", "B", "C"] {
        let dto = g.load_named(name, PageKind::Page).unwrap().unwrap();
        g.save_page(&dto, dto.rev.as_deref()).unwrap();
    }
    assert_eq!(std::fs::read_to_string(root.join("pages").join("A.md")).unwrap(), no_nl);
    assert_eq!(std::fs::read_to_string(root.join("pages").join("B.md")).unwrap(), with_nl);
    assert_eq!(std::fs::read_to_string(root.join("pages").join("C.md")).unwrap(), spaces);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_refuses_to_clobber_external_change() {
    use tine_core::model::PageKind;

    let root = std::env::temp_dir().join(format!("tine-conflict-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();

    let g = Graph::open(&root);
    // Build the cache (Tine now "knows" N = "- one"), then load it for editing.
    g.search("one", 10);
    let dto = g.load_named("N", PageKind::Page).unwrap().unwrap();

    // An external writer (another app / Syncthing) changes the file.
    std::fs::write(&path, "- EXTERNAL EDIT").unwrap();

    // Saving the now-stale page must fail with a conflict and NOT overwrite.
    let err = g.save_page(&dto, dto.rev.as_deref()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "- EXTERNAL EDIT");

    // "Keep mine" force-saves over it.
    g.force_save_page(&dto).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("one"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_conflicts_when_file_deleted_externally() {
    use tine_core::model::PageKind;
    let root = std::env::temp_dir().join(format!("tine-del-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();
    let g = Graph::open(&root);
    g.search("one", 10); // warm cache
    let dto = g.load_named("N", PageKind::Page).unwrap().unwrap();

    // The file is deleted on disk (Syncthing / Logseq) after we loaded it.
    std::fs::remove_file(&path).unwrap();

    // Saving must conflict, NOT silently resurrect the deleted note.
    let err = g.save_page(&dto, dto.rev.as_deref()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(!path.exists(), "deleted file must stay deleted on a conflicting save");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn load_reflects_external_change_then_save_is_clean() {
    use tine_core::model::PageKind;
    let root = std::env::temp_dir().join(format!("tine-reconcile-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let _ = g.load_named("N", PageKind::Page).unwrap().unwrap(); // cache built

    // External writer changes the file; the 3s watcher hasn't run yet.
    std::fs::write(&path, "- TWO external").unwrap();

    // load_page must reconcile and serve the NEW content (not the stale cache),
    // and the rev it returns must match disk so a save doesn't spuriously conflict.
    let dto = g.load_named("N", PageKind::Page).unwrap().unwrap();
    assert!(dto.blocks[0].raw.contains("TWO external"), "load reflects external change");
    g.save_page(&dto, dto.rev.as_deref()).expect("save of freshly-loaded current content is clean");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn consecutive_self_saves_do_not_conflict() {
    // Regression: inserting a SCHEDULED date then deleting it (consecutive saves
    // with no external writer) must not raise a spurious "changed on disk"
    // conflict. Each save returns the new baseline rev, which the next save passes
    // back (exactly what the frontend does) — so our own write is never mistaken
    // for an external edit.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-selfsave-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);
    let mk = |raw: &str| PageDto {
        name: "D".into(),
        kind: PageKind::Page,
        title: "D".into(),
        pre_block: None,
        blocks: vec![BlockDto { id: "b1".into(), raw: raw.into(), ..Default::default() }],
        rev: None,
    };
    // 1) date picker inserts a SCHEDULED line (page is new — no baseline yet).
    let r1 = g.save_page(&mk("TODO task\nSCHEDULED: <2026-06-16 Tue>"), None).unwrap();
    // 2) user deletes the inserted text — must NOT be read as an external edit.
    let r2 = g
        .save_page(&mk("TODO task"), Some(&r1))
        .expect("no spurious conflict after our own save");
    // 3) and a further edit still saves cleanly.
    g.save_page(&mk("TODO task edited"), Some(&r2)).expect("no spurious conflict");
    assert!(std::fs::read_to_string(root.join("pages").join("D.md")).unwrap().contains("edited"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn sync_file_detects_external_change_and_suppresses_self() {
    use tine_core::model::PageKind;

    let root = std::env::temp_dir().join(format!("tine-sync-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("S.md");
    std::fs::write(&path, "- before").unwrap();

    let g = Graph::open(&root);
    g.search("before", 10); // build the cache (S = "- before")

    // No external change yet → sync reports nothing.
    assert!(g.sync_file(&path).is_none());

    // External edit → sync reports the entry and refreshes the cache.
    std::fs::write(&path, "- after the change").unwrap();
    let changed = g.sync_file(&path).expect("external change detected");
    assert_eq!(changed.name, "S");
    assert_eq!(changed.kind, PageKind::Page);
    assert_eq!(g.search("after", 10).len(), 1, "cache updated to new content");
    assert_eq!(g.search("before", 10).len(), 0);

    // Re-syncing the same content is a no-op (self-write suppression).
    assert!(g.sync_file(&path).is_none());

    // Deletion is reported and drops it from the cache.
    std::fs::remove_file(&path).unwrap();
    assert!(g.forget_file(&path).is_some());
    assert_eq!(g.search("after", 10).len(), 0);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn noop_save_does_not_bump_cache_generation() {
    // A save whose serialized bytes match disk (focus/blur, forced flush of an
    // unchanged page) must NOT bump cache_gen — that key invalidates every
    // memoized query/backlink/derived result, so a no-op re-save would force a
    // whole-graph requery on every open dashboard. A real edit still bumps it.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-noopgen-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);
    g.search("x", 10); // build the cache
    let mk = |raw: &str| PageDto {
        name: "N".into(),
        kind: PageKind::Page,
        title: "N".into(),
        pre_block: None,
        blocks: vec![BlockDto { id: "b1".into(), raw: raw.into(), ..Default::default() }],
        rev: None,
    };
    let r1 = g.save_page(&mk("hello"), None).unwrap();
    let gen1 = g.cache_generation();
    // Re-save byte-identical content (no-op) with the returned baseline.
    let r2 = g.save_page(&mk("hello"), Some(&r1)).unwrap();
    assert_eq!(r1, r2, "rev must be stable across a no-op save");
    assert_eq!(g.cache_generation(), gen1, "no-op save must not bump cache_gen");
    // A real edit DOES bump it.
    g.save_page(&mk("hello world"), Some(&r2)).unwrap();
    assert!(g.cache_generation() > gen1, "a real edit must bump cache_gen");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn recent_writes_entry_is_consumed_on_first_match() {
    // The self-write guard map is consumed once the watcher observes the write, so
    // it stays bounded AND a later external write restoring those exact bytes is
    // not silently suppressed. After a save we drop the page from the parse cache
    // (so cache-comparison can't also suppress): the FIRST sync is suppressed via
    // recent_writes, the SECOND sees a change (proving the entry was consumed).
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-rwconsume-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);
    g.search("x", 10);
    let page = PageDto {
        name: "C".into(),
        kind: PageKind::Page,
        title: "C".into(),
        pre_block: None,
        blocks: vec![BlockDto { id: "b1".into(), raw: "noted".into(), ..Default::default() }],
        rev: None,
    };
    g.save_page(&page, None).unwrap();
    let path = root.join("pages").join("C.md");
    assert!(g.forget_file(&path).is_some(), "page should have been cached");
    assert!(g.sync_file(&path).is_none(), "first sync suppressed via recent_writes");
    assert!(
        g.sync_file(&path).is_some(),
        "entry must be consumed: a second look at the now-uncached page is a real change"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn watcher_suppresses_own_write_when_parse_cache_lags() {
    // Regression for the false "changed on disk" conflict seen during normal
    // typing (no external writer): the file watcher reads files outside the cache
    // lock, so in the window between a save's atomic rename and its cache_upsert it
    // can read disk-ahead-of-cache and flag Tine's own write as an external change.
    // `recent_writes` lets it recognize our own bytes regardless of cache state.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-selfwrite-lag-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);
    g.search("x", 10); // build the (empty) cache

    let path = root.join("pages").join("W.md");
    let page = PageDto {
        name: "W".into(),
        kind: PageKind::Page,
        title: "W".into(),
        pre_block: None,
        // A multi-line block ending in a `>` quote line — the shape that surfaced
        // the bug (shift-enter then ">").
        blocks: vec![BlockDto { id: "b1".into(), raw: "hello\n> quote".into(), ..Default::default() }],
        rev: None,
    };
    g.save_page(&page, None).unwrap();

    // Simulate the rename→cache_upsert gap: drop the page from the parse cache so
    // the cache no longer reflects the file — exactly the state the 3s poller can
    // read into mid-save.
    assert!(g.forget_file(&path).is_some(), "page should have been cached by the save");

    // The watcher polling now must STILL recognize our own write and emit nothing.
    assert!(
        g.sync_file(&path).is_none(),
        "Tine's own save must not be reported as an external change"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn query_between_filters_by_journal_date() {
    let root = std::env::temp_dir().join(format!("tine-between-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("journals").join("2022_06_15.md"), "- TODO [[scs]] recent\n").unwrap();
    std::fs::write(root.join("journals").join("2019_01_01.md"), "- TODO [[scs]] old\n").unwrap();

    let g = Graph::open(&root);
    let groups = g.run_query(
        "(and (task TODO) (and [[scs]] (between [[Jan 1st, 2021]] [[Jan 1st, 2100]])))",
    );
    let raws: Vec<String> = groups
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(raws.iter().any(|r| r.contains("recent")), "in-range journal matches: {raws:?}");
    assert!(!raws.iter().any(|r| r.contains("old")), "out-of-range journal excluded: {raws:?}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_page_moves_file_and_updates_refs() {
    let root = std::env::temp_dir().join(format!("tine-rename-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("Old Name.md"), "- the page body\n").unwrap();
    std::fs::write(root.join("pages").join("Other.md"), "- see [[Old Name]] and #[[Old Name]]\n").unwrap();
    std::fs::write(root.join("journals").join("2026_06_15.md"), "- ref [[Old Name]]\n").unwrap();

    let g = Graph::open(&root);
    g.rename_page("Old Name", "New Name").unwrap();

    // File moved.
    assert!(!root.join("pages").join("Old Name.md").exists());
    assert!(root.join("pages").join("New Name.md").exists());
    // References rewritten everywhere.
    let other = std::fs::read_to_string(root.join("pages").join("Other.md")).unwrap();
    assert!(other.contains("[[New Name]]"), "{other}");
    assert!(other.contains("#[[New Name]]"), "{other}");
    assert!(!other.contains("Old Name"), "{other}");
    let journal = std::fs::read_to_string(root.join("journals").join("2026_06_15.md")).unwrap();
    assert!(journal.contains("[[New Name]]"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn query_and_not_includes_everything_except_excluded() {
    // (and (task TODO) (not [[X]])) must return ALL TODO blocks that don't
    // reference [[X]] — regression for "NOT excludes right but drops others".
    let root = std::env::temp_dir().join(format!("tine-not-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages").join("P.md"),
        "- TODO alpha\n- TODO beta [[X]]\n- TODO gamma\n- DONE delta\n").unwrap();
    let g = Graph::open(&root);
    let raws: Vec<String> = g
        .run_query("(and (task TODO) (not [[X]]))")
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(raws.iter().any(|r| r.contains("alpha")), "{raws:?}");
    assert!(raws.iter().any(|r| r.contains("gamma")), "{raws:?}");
    assert!(!raws.iter().any(|r| r.contains("beta")), "X-referencing excluded: {raws:?}");
    assert!(!raws.iter().any(|r| r.contains("delta")), "non-TODO excluded: {raws:?}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn page_aliases_resolve_and_collect_backlinks() {
    use tine_core::model::PageKind;
    let root = std::env::temp_dir().join(format!("tine-alias-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Canonical page "Parameterized Complexity" with an alias "PC".
    std::fs::write(root.join("pages").join("Parameterized Complexity.md"),
        "alias:: PC\n\n- the canonical page\n").unwrap();
    // One page links the canonical name, another links the alias.
    std::fs::write(root.join("pages").join("A.md"), "- see [[Parameterized Complexity]]\n").unwrap();
    std::fs::write(root.join("pages").join("B.md"), "- via [[PC]]\n").unwrap();

    let g = Graph::open(&root);
    // Loading the alias resolves to the canonical page.
    let dto = g.load_named("PC", PageKind::Page).unwrap().expect("alias resolves");
    assert!(dto.blocks.iter().any(|b| b.raw.contains("canonical page")));
    // Backlinks of the canonical page include the alias-referencing page.
    let pages: Vec<String> = g.backlinks("Parameterized Complexity").iter().map(|gr| gr.page.clone()).collect();
    assert!(pages.contains(&"A".to_string()), "{pages:?}");
    assert!(pages.contains(&"B".to_string()), "alias ref counted: {pages:?}");
    // Backlinks queried via the alias name also resolve to the canonical set.
    let via_alias: Vec<String> = g.backlinks("PC").iter().map(|gr| gr.page.clone()).collect();
    assert!(via_alias.contains(&"A".to_string()) && via_alias.contains(&"B".to_string()), "{via_alias:?}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_favorites_round_trips_in_config_edn() {
    let root = std::env::temp_dir().join(format!("tine-fav-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Existing config with another key that must be preserved + a COMMENTED
    // :favorites decoy that must NOT be edited (the write must target the real key).
    std::fs::write(root.join("logseq").join("config.edn"),
        "{;; :favorites [\"example\"]\n :preferred-workflow :now\n :journals-directory \"journals\"}\n").unwrap();

    let g = Graph::open(&root);
    g.set_favorites(&["Inbox".into(), "Reading List".into()]).unwrap();
    // Re-open and confirm favorites parsed back + the other key survived.
    let g2 = Graph::open(&root);
    assert_eq!(g2.meta().favorites, vec!["Inbox".to_string(), "Reading List".to_string()]);
    let cfg = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap();
    assert!(cfg.contains(":journals-directory"), "other keys preserved: {cfg}");
    assert!(cfg.contains(";; :favorites [\"example\"]"), "commented decoy untouched: {cfg}");

    // Updating again replaces (not appends) the vector.
    g2.set_favorites(&["Only One".into()]).unwrap();
    assert_eq!(Graph::open(&root).meta().favorites, vec!["Only One".to_string()]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_preferred_workflow_round_trips_in_config_edn() {
    let root = std::env::temp_dir().join(format!("tine-wf-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Existing config: another key + a *commented* decoy that must NOT be edited.
    std::fs::write(
        root.join("logseq").join("config.edn"),
        "{;; :preferred-workflow :todo (example)\n :preferred-workflow :now\n :start-of-week 1}\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    assert_eq!(g.meta().preferred_workflow, "now");
    g.set_preferred_workflow("todo").unwrap();

    let g2 = Graph::open(&root);
    assert_eq!(g2.meta().preferred_workflow, "todo");
    let cfg = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap();
    assert!(cfg.contains(":start-of-week 1"), "other keys preserved: {cfg}");
    // The commented decoy line is untouched (still says :todo (example)).
    assert!(cfg.contains(";; :preferred-workflow :todo (example)"), "comment preserved: {cfg}");

    // Flipping back replaces (not appends) the keyword.
    g2.set_preferred_workflow("now").unwrap();
    assert_eq!(Graph::open(&root).meta().preferred_workflow, "now");
    let cfg = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap();
    assert_eq!(cfg.matches(":preferred-workflow :now").count(), 1, "no duplicate key: {cfg}");

    // Inserting into a config that lacks the key entirely.
    std::fs::write(root.join("logseq").join("config.edn"), "{:start-of-week 0}\n").unwrap();
    let g3 = Graph::open(&root);
    g3.set_preferred_workflow("todo").unwrap();
    assert_eq!(Graph::open(&root).meta().preferred_workflow, "todo");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn deleted_journal_is_not_served_from_stale_cache() {
    use tine_core::PageKind;
    let root = std::env::temp_dir().join(format!("tine-del-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let jpath = root.join("journals").join("2026_06_18.md");
    std::fs::write(&jpath, "- hello\n").unwrap();

    let g = Graph::open(&root);
    // Warm the whole-graph cache so the journal is held in memory.
    let entries = g.journals_desc();
    assert_eq!(entries.len(), 1);
    let entry = entries[0].clone();
    assert!(g.load_page(&entry).is_ok());

    // External delete (OG Logseq / Syncthing) before the watcher reconciles.
    std::fs::remove_file(&jpath).unwrap();

    // load_page must report NotFound, NOT serve the cached copy — serving it with
    // a null rev would let a subsequent save recreate the deleted file.
    let err = g.load_page(&entry).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    // load_named treats the vanished page as absent (Ok(None)), not an error.
    assert!(g.load_named(&entry.name, PageKind::Journal).unwrap().is_none());
    // The stale entry was evicted, so the feed no longer lists it.
    assert!(g.journals_desc().is_empty(), "deleted journal must drop out of the feed");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn query_open_tasks() {
    let g = demo_graph();
    let groups = g.run_query("(task TODO DOING)");
    let raws: Vec<String> = groups
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(raws.iter().any(|r| r.starts_with("TODO Ship the M0")), "got: {raws:?}");
    assert!(raws.iter().any(|r| r.starts_with("DOING Wire up")), "got: {raws:?}");
    // A DONE task must not match.
    assert!(!raws.iter().any(|r| r.contains("DONE Validate")), "got: {raws:?}");
}
