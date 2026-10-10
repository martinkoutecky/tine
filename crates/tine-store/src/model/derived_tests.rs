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

/// Hold `key` with no host running and nothing published yet.
fn hold(store: &Store, key: &str) {
    let _writer = store.writer.lock().unwrap();
    store.graph.hold_unhosted(key);
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
    let corpus = store.whole_graph().unwrap().corpus();
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
                    .map(|block| (block.raw().to_owned(), block.uuid.clone()))
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
        row.unwrap().1.roots[0].children[0].uuid.clone()
    });
    assert_eq!(bodies()[0].1, "saved-root", "R8: the saved root's identity");
    assert_eq!(
        child.as_deref(),
        Some("saved-child"),
        "R8: the child's identity"
    );
    store.close();
}
