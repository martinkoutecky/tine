//! A-H2 (G), R1 and R2 (REVIEW-AH2-AW1-plan R6): every installed row about
//! a held page, its listing, its claimants and the published snapshot's
//! roots come from its owner's last published bytes. `hold` and `publish`
//! are the only calls whose API differs at the pre-A-H2 baseline.

use crate::model::{content_rev, PageKind};
use crate::{Depth, PageId, Resolved, Store};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

fn graph(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Arc<Store>) {
    let temp = tempfile::tempdir().unwrap();
    for area in ["pages", "journals"] {
        fs::create_dir_all(temp.path().join(area)).unwrap();
    }
    for (rel, bytes) in files {
        fs::write(temp.path().join(rel), bytes).unwrap();
    }
    let store = Arc::new(Store::open(temp.path(), Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (temp, store)
}

#[test]
fn active_page_list_materialization_survives_body_only_edits() {
    let (root, store) = graph(&[
        ("pages/A.md", b"title:: Shared\n- first\n"),
        ("pages/Z.md", b"title:: Shared\n- inactive\n"),
    ]);
    store.stop_watcher_for_tests();
    let first = store.whole_graph().unwrap().test_read_snapshot();
    let list = first.page_list_arc();
    assert_eq!(list.len(), 1);
    let count = super::ACTIVE_LIST_MATERIALIZATIONS.with(std::cell::Cell::get);
    fs::write(
        root.path().join("pages/A.md"),
        "title:: Shared\n- edited body\n",
    )
    .unwrap();
    store.refresh(Depth::Bytes).unwrap();
    let second = store.whole_graph().unwrap().test_read_snapshot();
    assert!(Arc::ptr_eq(&first.list, &second.list));
    let after = second.page_list_arc();
    assert_eq!(
        super::ACTIVE_LIST_MATERIALIZATIONS.with(std::cell::Cell::get),
        count
    );
    assert!(Arc::ptr_eq(&list, &after));
    // A title change promotes the unchanged sibling and invalidates the list.
    fs::write(
        root.path().join("pages/A.md"),
        "title:: Elsewhere\n- edited body\n",
    )
    .unwrap();
    store.refresh(Depth::Bytes).unwrap();
    let third = store.whole_graph().unwrap().test_read_snapshot();
    assert_eq!(third.page_list_arc().len(), 2);
    assert_eq!(
        super::ACTIVE_LIST_MATERIALIZATIONS.with(std::cell::Cell::get),
        count + 1
    );
    store.close();
}

/// Hold `key` with no host running and nothing published yet.
fn hold(store: &Store, key: &str) {
    let _writer = store.writer.lock().unwrap();
    store.graph.hold_unhosted(key);
}

/// Hold `key` as a page open does ([`super::Graph::hold_current`]), after
/// `edit` ran under the same writer (no watcher reconcile in between).
fn hold_open(store: &Store, key: &str, edit: impl FnOnce()) {
    let _writer = store.writer.lock().unwrap();
    edit();
    store.graph.held.spellings().spell(key, key);
    store.graph.hold_current(key.into());
    store.publish_retired();
}

/// The owner's publication of held `key` (None: no file).
fn publish(store: &Store, key: &str, bytes: Option<&str>) {
    let _writer = store.writer.lock().unwrap();
    let bytes = bytes.map(|bytes| Arc::from(bytes.as_bytes()));
    assert!(store.graph.publish_owned(key, bytes, None).is_ok());
}

/// The canonical claimant the published snapshot resolves `name` to.
fn resolves(store: &Store, name: &str, journal: bool) -> Option<(String, Vec<String>)> {
    match store.whole_graph().unwrap().resolve(name, journal) {
        Resolved::Existing { id, others } => Some((
            id.as_str().to_owned(),
            others.iter().map(|id| id.as_str().to_owned()).collect(),
        )),
        _ => None,
    }
}

fn page(store: &Store, name: &str) -> Option<String> {
    resolves(store, name, false).map(|(id, _)| id)
}

/// Every name an installed or memoized inventory gives `path`: the page
/// list, the claimants and the snapshot inventory's two outputs.
fn inventory_names(store: &Store, path: &Path) -> Vec<String> {
    let graph = &store.graph;
    let mut names: Vec<String> = graph
        .list_pages_shared()
        .iter()
        .filter(|entry| entry.path == path)
        .map(|entry| format!("list {}", entry.name))
        .collect();
    let (list, claimants) = graph.snapshot_name_index();
    names.extend(
        list.iter()
            .filter(|entry| entry.path == path)
            .map(|entry| format!("snapshot list {}", entry.name)),
    );
    let mut claimed: Vec<String> = claimants
        .iter()
        .filter(|(_, entries)| entries.iter().any(|entry| entry.path == path))
        .map(|((kind, key), _)| format!("snapshot claimant {kind:?} {key}"))
        .collect();
    claimed.sort();
    names.extend(claimed);
    names
}

/// H1 #1: a forced rebuild publishes a held page under the name and the
/// document of its owner's bytes, in the published roots and in both of
/// the snapshot inventory's outputs, while its file says something else.
#[test]
fn snapshot_names_a_held_page_from_owner_bytes() {
    let (_temp, store) = graph(&[("pages/a.md", b"title:: Owner\n- owner body\n")]);
    store.hold_page(&PageId::from("pages/a.md")).unwrap();
    let path = store.graph.root.join("pages/a.md");
    fs::write(&path, "title:: Disk\n- disk body\n").unwrap();
    store.refresh(Depth::Rebuild).unwrap();
    assert_eq!(
        page(&store, "Owner").as_deref(),
        Some("pages/a.md"),
        "(G): the published snapshot lost the owner's name"
    );
    assert_eq!(
        page(&store, "Disk"),
        None,
        "(G): the published snapshot names a held page from its file"
    );
    let corpus = store.whole_graph().unwrap().corpus(None, false);
    let docs: Vec<(String, String)> = corpus
        .pages
        .iter()
        .filter(|page| page.id.as_str() == "pages/a.md")
        .map(|page| (page.name.clone(), page.document.roots[0].raw().to_owned()))
        .collect();
    assert_eq!(docs.len(), 1, "{docs:?}");
    assert_eq!(docs[0].0, "Owner");
    assert!(docs[0].1.contains("owner body"), "{docs:?}");
    assert_eq!(
        inventory_names(&store, &path),
        [
            "list Owner",
            "snapshot list Owner",
            "snapshot claimant Page owner"
        ],
        "(G): an inventory named the held page from its file"
    );
    assert!(store
        .graph
        .find_claimants("Disk", PageKind::Page)
        .is_empty());
    store.close();
}

/// H1 #2: a held page's file that cannot be named records no installed
/// discovery error: the owner's bytes name it.
#[test]
fn a_held_page_leaves_no_installed_discovery_error() {
    let (_temp, store) = graph(&[("pages/a.md", b"title:: Owner\n- a\n")]);
    store.hold_page(&PageId::from("pages/a.md")).unwrap();
    let path = store.graph.root.join("pages/a.md");
    fs::write(&path, b"title:: \xff\xfe\n- b\n").unwrap();
    store.refresh(Depth::Rebuild).unwrap();
    store.graph.snapshot_name_index();
    let unreadable: Vec<String> = store
        .graph
        .unreadable_pages()
        .iter()
        .map(|(id, reason)| format!("{} {reason}", id.as_str()))
        .collect();
    assert!(
        unreadable.is_empty(),
        "(G): the held page's file installed an error: {unreadable:?}"
    );
    assert_eq!(page(&store, "Owner").as_deref(), Some("pages/a.md"));
    store.close();
}

type Pause = Arc<(Mutex<(bool, bool)>, Condvar)>;

fn wait_paused(pause: &Pause) {
    let (state, ready) = &**pause;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut state = state.lock().unwrap();
    while !state.0 {
        assert!(Instant::now() < deadline, "the build never paused");
        state = ready
            .wait_timeout(state, Duration::from_millis(50))
            .unwrap()
            .0;
    }
}

/// R1 freshness: an owner publication between a build's reads and its
/// install wins; the build never installs what it derived before it.
#[test]
fn an_owner_publication_during_a_build_stays_current() {
    let (_temp, store) = graph(&[("pages/a.md", b"- one\n"), ("pages/b.md", b"- b\n")]);
    store.hold_page(&PageId::from("pages/a.md")).unwrap();
    let path = store.graph.root.join("pages/a.md");
    let pause: Pause = Arc::new((Mutex::new((false, false)), Condvar::new()));
    *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&pause));
    let build = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || store.graph.rebuild_cache_cancellable(|| false))
    };
    wait_paused(&pause);
    publish(&store, "pages/a.md", Some("- two\n"));
    pause.0.lock().unwrap().1 = true;
    pause.1.notify_all();
    build.join().unwrap();
    *store.graph.warm_after_first_page_pause.lock().unwrap() = None;
    assert_eq!(
        store.graph.cached_rev(&path),
        Some(content_rev("- two\n")),
        "R1: a build installed the owner's earlier bytes over its publication"
    );
    assert!(store.graph.rebuild_cache_cancellable(|| false));
    assert_eq!(store.graph.cached_rev(&path), Some(content_rev("- two\n")));
    store.close();
}

/// R1: a new hold whose first publication never lands leaves nothing
/// about the page installed, through a rebuild and a published snapshot.
#[test]
fn a_hold_with_no_publication_installs_no_row() {
    let (_temp, store) = graph(&[("pages/a.md", b"title:: Disk\n- a\n")]);
    let path = store.graph.root.join("pages/a.md");
    assert_eq!(page(&store, "Disk").as_deref(), Some("pages/a.md"));
    hold(&store, "pages/a.md");
    assert_eq!(
        store.graph.cached_rev(&path),
        None,
        "R1: a new hold left the page's disk row current"
    );
    assert_eq!(inventory_names(&store, &path), Vec::<String>::new());
    store.refresh(Depth::Rebuild).unwrap();
    assert_eq!(store.graph.cached_rev(&path), None, "after a rebuild");
    assert_eq!(page(&store, "Disk"), None, "the published snapshot");
    assert!(store
        .graph
        .find_claimants("Disk", PageKind::Page)
        .is_empty());
    store.close();
}

/// (G) as amended 2026-10-11 (PERF-FIXES D4): an open's hold of a file
/// whose bytes are the installed row's keeps that row and publishes
/// nothing; a rebuild under the hold derives the same row.
#[test]
fn an_opens_hold_of_an_unchanged_file_keeps_its_row() {
    let body = "title:: Disk\n- a\n";
    let (_temp, store) = graph(&[("pages/a.md", body.as_bytes())]);
    let path = store.graph.root.join("pages/a.md");
    let rev = store.whole_graph().unwrap().rev();
    hold_open(&store, "pages/a.md", || {});
    assert_eq!(store.graph.cached_rev(&path), Some(content_rev(body)));
    assert_eq!(page(&store, "Disk").as_deref(), Some("pages/a.md"));
    assert_eq!(
        store.whole_graph().unwrap().rev(),
        rev,
        "D4: an unchanged open published a snapshot"
    );
    store.refresh(Depth::Rebuild).unwrap();
    assert_eq!(store.graph.cached_rev(&path), Some(content_rev(body)));
    assert_eq!(page(&store, "Disk").as_deref(), Some("pages/a.md"));
    store.close();
}

/// REVIEW-3a4 #2 under the amended (G): an open's hold of a file that
/// changed behind the index withholds the page until its owner publishes:
/// no view names it from the stale row or from bytes the owner has not
/// read.
#[test]
fn an_opens_hold_of_a_changed_file_installs_no_row() {
    let (temp, store) = graph(&[("pages/a.md", b"title:: Disk\n- a\n")]);
    let path = store.graph.root.join("pages/a.md");
    hold_open(&store, "pages/a.md", || {
        fs::write(temp.path().join("pages/a.md"), "title:: Edited\n- e\n").unwrap();
    });
    assert_eq!(store.graph.cached_rev(&path), None);
    assert_eq!(page(&store, "Disk"), None, "the stale row stayed current");
    assert_eq!(page(&store, "Edited"), None, "a name the owner never read");
    publish(&store, "pages/a.md", Some("title:: Edited\n- e\n"));
    assert_eq!(
        store.graph.cached_rev(&path),
        Some(content_rev("title:: Edited\n- e\n"))
    );
    store.close();
}

/// Claim `key` (an operation's page) whose owner observed `observed`.
fn hold_claimed(store: &Store, key: &str, observed: Option<&str>) {
    let _writer = store.writer.lock().unwrap();
    store.graph.held.spellings().spell(key, key);
    let observed = observed.map(|bytes| Arc::from(bytes.as_bytes()));
    store.graph.hold_observed(key.into(), observed);
    store.publish_retired();
}

/// A claim (an operation's page) whose owner observed the installed row's
/// bytes keeps the row, as an unchanged open does ((G) as amended, extended to
/// claims): nothing is published at the hold, and the owner's first
/// publication of those bytes changes nothing. Before, every page a rename or
/// delete took on was published as removed and then re-added: one snapshot and
/// one inventory change each, and a Concord record for a page being deleted.
#[test]
fn a_claim_of_the_rows_bytes_keeps_its_row() {
    let body = "title:: Disk\n- a\n";
    let (_temp, store) = graph(&[("pages/a.md", body.as_bytes())]);
    let path = store.graph.root.join("pages/a.md");
    let rev = store.whole_graph().unwrap().rev();
    hold_claimed(&store, "pages/a.md", Some(body));
    assert_eq!(page(&store, "Disk").as_deref(), Some("pages/a.md"));
    assert_eq!(
        store.whole_graph().unwrap().rev(),
        rev,
        "an unchanged claim published a snapshot"
    );
    publish(&store, "pages/a.md", Some(body));
    assert_eq!(store.whole_graph().unwrap().rev(), rev);
    assert_eq!(store.graph.cached_rev(&path), Some(content_rev(body)));
    store.close();
}

/// A claim whose owner observed other bytes than the row's, or nothing,
/// withholds the page until the owner publishes (REVIEW-3a4 #2): no view names
/// it from the stale row.
#[test]
fn a_claim_of_other_bytes_installs_no_row() {
    for observed in [Some("title:: Edited\n- e\n"), None] {
        let (_temp, store) = graph(&[("pages/a.md", b"title:: Disk\n- a\n")]);
        let path = store.graph.root.join("pages/a.md");
        hold_claimed(&store, "pages/a.md", observed);
        assert_eq!(store.graph.cached_rev(&path), None);
        assert_eq!(page(&store, "Disk"), None, "the stale row stayed current");
        let published = observed.unwrap_or("title:: Disk\n- a\n");
        publish(&store, "pages/a.md", Some(published));
        assert_eq!(store.graph.cached_rev(&path), Some(content_rev(published)));
        store.close();
    }
}

/// R1: an owner's absent publication removes every row about the page,
/// although its file is still on disk.
#[test]
fn an_owner_absence_removes_every_row() {
    let (_temp, store) = graph(&[("pages/a.md", b"title:: Owner\n- a\n")]);
    store.hold_page(&PageId::from("pages/a.md")).unwrap();
    let path = store.graph.root.join("pages/a.md");
    publish(&store, "pages/a.md", None);
    assert!(path.exists());
    assert_eq!(store.graph.cached_rev(&path), None);
    assert_eq!(inventory_names(&store, &path), Vec::<String>::new());
    store.refresh(Depth::Rebuild).unwrap();
    assert_eq!(store.graph.cached_rev(&path), None, "after a rebuild");
    assert_eq!(page(&store, "Owner"), None, "the published snapshot");
    store.close();
}

/// R1: a disk disappearance never removes a held page the owner indexed.
#[test]
fn a_disk_disappearance_keeps_a_held_page() {
    let (_temp, store) = graph(&[("pages/a.md", b"title:: Owner\n- a\n")]);
    store.hold_page(&PageId::from("pages/a.md")).unwrap();
    let path = store.graph.root.join("pages/a.md");
    fs::remove_file(&path).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    store.refresh(Depth::Bytes).unwrap();
    assert_eq!(
        store.graph.cached_rev(&path),
        Some(content_rev("title:: Owner\n- a\n"))
    );
    assert_eq!(page(&store, "Owner").as_deref(), Some("pages/a.md"));
    store.close();
}

/// R2: a removal is named by its previous snapshot name, not by a read of
/// the gone file: after the cache is evicted, a deleted page's title no
/// longer resolves.
#[test]
fn a_removal_after_cache_eviction_unnames_the_page() {
    let (_temp, store) = graph(&[("pages/a.md", b"title:: Old\n- a\n")]);
    assert_eq!(page(&store, "Old").as_deref(), Some("pages/a.md"));
    store.graph.invalidate_cache();
    fs::remove_file(store.graph.root.join("pages/a.md")).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(
        page(&store, "Old"),
        None,
        "R2: a deleted page's title still resolves"
    );
    store.close();
}

/// R2 neighbour: two files claiming one title; deleting one after an
/// eviction leaves the other the only claimant.
#[test]
fn a_removed_duplicate_claimant_leaves_the_other() {
    let (_temp, store) = graph(&[
        ("pages/a.md", b"title:: Same\n- a\n"),
        ("pages/b.md", b"title:: Same\n- b\n"),
    ]);
    assert!(resolves(&store, "Same", false).is_some_and(|(_, others)| others.len() == 1));
    store.graph.invalidate_cache();
    fs::remove_file(store.graph.root.join("pages/a.md")).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(
        resolves(&store, "Same", false),
        Some(("pages/b.md".to_owned(), Vec::new())),
        "R2: the deleted claimant still claims the title"
    );
    store.close();
}

/// R2 neighbour: journal twins (one day in two formats) and a shadow
/// (a title-named file of the same day, a later claimant). Deleting a twin
/// after an eviction leaves the other twin the day's only claimant;
/// deleting the shadow leaves the canonical file the only one.
#[test]
fn journal_twins_and_shadows_keep_their_claimants() {
    let (_temp, store) = graph(&[
        ("journals/2026_06_26.md", b"- md twin\n"),
        ("journals/2026_06_26.org", b"* org twin\n"),
        ("journals/2026_06_27.md", b"- canonical\n"),
        ("journals/Jun 27th, 2026.md", b"- shadow\n"),
    ]);
    let (day, _) = store.graph.snapshot_name_index();
    let title = |stem: &str| {
        day.iter()
            .find(|entry| {
                entry
                    .rel_path_str()
                    .starts_with(&format!("journals/{stem}"))
            })
            .map(|entry| entry.name.clone())
            .unwrap()
    };
    let (twins, next) = (title("2026_06_26"), title("2026_06_27"));
    let claimants = |name: &str| resolves(&store, name, true);
    let canonical = "journals/2026_06_27.md".to_owned();
    assert_eq!(
        claimants(&next),
        Some((
            canonical.clone(),
            vec!["journals/Jun 27th, 2026.md".to_owned()]
        ))
    );
    store.graph.invalidate_cache();
    fs::remove_file(store.graph.root.join("journals/2026_06_26.md")).unwrap();
    fs::remove_file(store.graph.root.join("journals/Jun 27th, 2026.md")).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(
        claimants(&twins),
        Some(("journals/2026_06_26.org".to_owned(), Vec::new())),
        "R2: a removed twin still claims its day"
    );
    assert_eq!(
        claimants(&next),
        Some((canonical, Vec::new())),
        "R2: a removed shadow still claims its day"
    );
    store.close();
}

/// REVIEW-3a4 #4: a direct read's discovery error decided before a hold
/// never lands after it. The read pauses between its decision and its
/// record while a writer holds the page and publishes its owner's bytes.
#[test]
fn a_discovery_error_never_lands_after_a_hold() {
    let (_temp, store) = graph(&[("pages/a.md", b"title:: Owner\n- a\n")]);
    let path = store.graph.root.join("pages/a.md");
    fs::write(&path, b"title:: \xff\xfe\n- b\n").unwrap();
    let pause: Pause = Arc::new((Mutex::new((false, false)), Condvar::new()));
    *store.graph.discovery_record_pause.lock().unwrap() = Some(Arc::clone(&pause));
    let read = {
        let (store, path) = (Arc::clone(&store), path.clone());
        std::thread::spawn(move || drop(store.graph.entry_for_path(&path)))
    };
    wait_paused(&pause);
    *store.graph.discovery_record_pause.lock().unwrap() = None;
    let holder = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            hold(&store, "pages/a.md");
            publish(&store, "pages/a.md", Some("title:: Owner\n- owner\n"));
            let _writer = store.writer.lock().unwrap();
            store.publish_retired();
        })
    };
    // The holder runs to its end, or waits for the read's decision.
    let deadline = Instant::now() + Duration::from_millis(300);
    while !holder.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    pause.0.lock().unwrap().1 = true;
    pause.1.notify_all();
    read.join().unwrap();
    holder.join().unwrap();
    let unreadable: Vec<String> = store
        .graph
        .unreadable_pages()
        .iter()
        .map(|(id, reason)| format!("{} {reason}", id.as_str()))
        .collect();
    assert!(
        unreadable.is_empty(),
        "REVIEW-3a4 #4: a disk discovery error landed after the hold: {unreadable:?}"
    );
    store.close();
}

/// REVIEW-3a4 #5: an owner publication installs what its bytes say; a
/// Document of other content never reaches the row, and one of the same
/// content keeps its runtime identities (R8).
#[test]
fn an_owner_publication_installs_its_bytes_not_a_foreign_document() {
    let (_temp, store) = graph(&[("pages/a.md", b"- a\n")]);
    let path = store.graph.root.join("pages/a.md");
    hold(&store, "pages/a.md");
    let bodies = || {
        store
            .graph
            .with_cached(&path, |row| {
                let doc = &row.expect("the owner's row").1;
                doc.roots
                    .iter()
                    .map(|block| (block.raw().to_owned(), block.uuid.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap()
    };
    let owner = "title:: Owner\n- owner bytes\n";
    let foreign = tine_core::doc::parse("- foreign document\n");
    {
        let _writer = store.writer.lock().unwrap();
        let bytes = Some(Arc::from(owner.as_bytes()));
        assert!(store
            .graph
            .publish_owned("pages/a.md", bytes, Some(&foreign))
            .is_ok());
    }
    let installed: Vec<String> = bodies().into_iter().map(|(raw, _)| raw).collect();
    assert_eq!(
        installed,
        ["owner bytes"],
        "REVIEW-3a4 #5: an owner publication installed a foreign Document"
    );
    let saved_body = "title:: Owner\n- saved\n  - child\n";
    let mut saved = tine_core::doc::parse(saved_body);
    saved.roots[0].uuid = "saved-root".into();
    saved.roots[0].children[0].uuid = "saved-child".into();
    {
        let _writer = store.writer.lock().unwrap();
        let bytes = Some(Arc::from(saved_body.as_bytes()));
        assert!(store
            .graph
            .publish_owned("pages/a.md", bytes, Some(&saved))
            .is_ok());
    }
    let child = store.graph.with_cached(&path, |row| {
        row.unwrap().1.roots[0].children[0].uuid.to_string()
    });
    assert_eq!(bodies()[0].1, "saved-root", "R8: the saved root's identity");
    assert_eq!(
        child.as_deref(),
        Some("saved-child"),
        "R8: the child's identity"
    );
    store.close();
}

/// Review N1 of og-perf-publish: a journal the publication's raw file list
/// does not name (here an owner's bytes, no file yet, taken in from the
/// retired rows) joins the day index with the page list, so the day's id is
/// that journal, not a proposed path.
#[test]
fn a_journal_only_the_capture_names_joins_the_day_index() {
    let (_temp, store) = graph(&[("pages/a.md", b"- a\n")]);
    let key = "journals/Jun 1st, 2031.md";
    hold(&store, key);
    publish(&store, key, Some("- the day\n"));
    {
        let _writer = store.writer.lock().unwrap();
        store
            .changes
            .publish(crate::Origin::External, Vec::new(), false, Vec::new());
    }
    let view = store.whole_graph().unwrap();
    let listed = (view.list.iter()).find(|entry| entry.date_key == Some(20310601));
    assert_eq!(
        listed.and_then(|entry| entry.rel_path.clone()),
        Some(PageId::from(key)),
        "the page list names the day"
    );
    assert_eq!(store.journal_id(crate::Day(20310601), false).as_str(), key);
    drop(view);
    store.close();
}

/// REG-NAME-LOOKUP-DURING-RELEASE-KEEPS-PAGE: a host's release (and an
/// open's hold) moves the held map and no generation. A listing walk whose
/// certify read the map on both sides of a release dropped the page (its
/// disk row was the owner's when checked, and no owner was left to list
/// it), and the memo it installed stood for the whole generation: the name
/// found nothing, the page list lost the page, and a new file claiming the
/// name passed the twin guard. Forced at both memo installers.
#[test]
fn a_name_walk_during_a_release_keeps_the_released_page() {
    use crate::{EditKind, Refusal, SaveBase, Why};
    let (temp, store) = graph(&[
        ("pages/a.md", b"title:: Renamed\n- a\n"),
        ("pages/b.md", b"- b\n"),
    ]);
    let path = store.graph.root.join("pages/a.md");
    let mut wrong = Vec::new();
    for reader in ["claimants", "list"] {
        // An open's hold of the unchanged file: no generation moves.
        hold_open(&store, "pages/a.md", || {});
        // Both memos walk at this generation (dropping a memo is safe).
        store.graph.forget_name_memos();
        let pause: Pause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.graph.certify_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let walk = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || match reader {
                "claimants" => (store.graph.find_claimants("Renamed", PageKind::Page))
                    .into_iter()
                    .map(|entry| entry.path)
                    .collect::<Vec<_>>(),
                _ => (store.graph.list_pages_shared().iter())
                    .filter(|entry| entry.name == "Renamed")
                    .map(|entry| entry.path.clone())
                    .collect(),
            })
        };
        // The walk has checked its disk rows' authority; the release lands
        // before it lists the owners' rows.
        wait_paused(&pause);
        *store.graph.certify_pause.lock().unwrap() = None;
        {
            let _writer = store.writer.lock().unwrap();
            store.graph.release("pages/a.md");
        }
        pause.0.lock().unwrap().1 = true;
        pause.1.notify_all();
        if walk.join().unwrap() != [path.clone()] {
            wrong.push(format!("{reader}: the walk lost the released page"));
        }
        // The memos at this same generation.
        if store
            .graph
            .find_entry("Renamed", PageKind::Page)
            .map(|entry| entry.path)
            != Some(path.clone())
        {
            wrong.push(format!("{reader}: the name finds nothing"));
        }
        if !(store.graph.list_pages_shared().iter()).any(|entry| entry.path == path) {
            wrong.push(format!("{reader}: the page list lost the page"));
        }
        // A new file claiming the name is the page's twin.
        let twin = store.graph.root.join("pages/renamed.md");
        let dto = (store.graph.page_dto_for_bytes(&twin, b"- twin\n"))
            .unwrap()
            .unwrap();
        let mut tx = store.page_plan(Some(EditKind::SaveBlock));
        tx.save_page(
            &[EditKind::SaveBlock],
            &PageId::from("pages/renamed.md"),
            SaveBase::CreateNew,
            &dto,
        );
        if !matches!(tx.plan(), Err(Why::Refused(Refusal::Twin { .. }))) {
            wrong.push(format!("{reader}: a twin of the page was planned"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    store.close();
}

/// REG-NAME-MEMO-SKIPS-HELD-FILE-READ: a path read of a file a host holds is
/// no name observation ((G)): its owner's bytes name it, not the file. A page
/// held and not yet published by its owner (withheld), and a journal whose
/// owner published its deletion while the file is still on disk, must not
/// join the claimant memo from the file; a walk at the same generation lists
/// neither.
#[test]
fn a_path_read_of_a_held_file_does_not_join_the_claimant_memo() {
    let (_temp, store) = graph(&[
        ("pages/a.md", b"title:: Disk\n- a\n"),
        ("journals/2031_06_01.md", b"- the day\n"),
    ]);
    let (page, journal) = ("pages/a.md", "journals/2031_06_01.md");
    hold(&store, page);
    hold(&store, journal);
    publish(&store, journal, None);
    let mut wrong = Vec::new();
    for key in [page, journal] {
        let path = store.graph.root.join(key);
        // Both kinds walked completely at this generation.
        store.graph.forget_name_memos();
        store.graph.list_pages_shared();
        store.graph.find_claimants("Disk", PageKind::Page);
        store
            .graph
            .find_claimants("Jun 1st, 2031", PageKind::Journal);
        // An open's read of the file, named from its bytes.
        let entry = store.graph.entry_for_path(&path).unwrap();
        let paths = |entries: Vec<crate::model::PageEntry>| -> Vec<_> {
            entries.into_iter().map(|entry| entry.path).collect()
        };
        let memo = paths(store.graph.find_claimants(&entry.name, entry.kind));
        store.graph.forget_name_memos();
        let walked = paths(store.graph.find_claimants(&entry.name, entry.kind));
        if memo != walked {
            wrong.push(format!("{key}: memo {memo:?}, walk {walked:?}"));
        }
        assert!(walked.is_empty(), "{key}: the walk lists the held file");
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    store.close();
}
