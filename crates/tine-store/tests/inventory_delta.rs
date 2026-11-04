//! Cost-model audit 2026-11-02 rows 1 and 2 (lane og-perf-names): a link edit
//! changes the window's name answers only by what moved graph-wide, carried
//! as a delta; `[[` completion answers from the resident name table.
use crate::{EditKind, InventoryEntry, InventoryScope, PageId, SaveBase, Store};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn graph(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for dir_name in ["pages", "journals", "logseq"] {
        fs::create_dir_all(dir.path().join(dir_name)).unwrap();
    }
    for (path, body) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    dir
}

/// Open and make the window's first inventory read, as at every graph bind.
fn open(root: &Path, options: crate::OpenOptions) -> Arc<Store> {
    let store = Arc::new(Store::open(root, options).unwrap().0);
    store.whole_graph().unwrap().inventory(InventoryScope::All);
    store
}

fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Arc<Store>) {
    let dir = graph(files);
    let store = open(dir.path(), Default::default());
    (dir, store)
}

type Rows = Vec<(String, String, Vec<usize>)>;

/// The live generation's table: inherited by patch (`None` when this
/// generation has none), and the rows of a fresh build from its pages.
fn table_rows(store: &Store) -> (Option<Rows>, Rows) {
    let view = store.whole_graph().unwrap();
    let rows = crate::model::ReadSnapshot::referenced_name_rows;
    (
        view.graph.referenced_name_table_built().map(rows),
        rows(&view.graph.referenced_name_table_fresh()),
    )
}

type Image = BTreeMap<(bool, String, String), String>;

/// What every listed name resolves to, by key. A reference-only or alias
/// name is listed in the spelling of the least page slot referring to it;
/// slots follow the order pages entered this session, and a restart
/// re-slots them, so that spelling (and the absent page id derived from it)
/// is left out here and checked against the old walk instead.
fn resolution(image: &Image) -> Vec<(bool, String, String, String)> {
    let mut rows: Vec<_> = image
        .iter()
        .map(|((journal, key, name), target)| {
            if target.starts_with("Existing") {
                (*journal, key.clone(), name.clone(), target.clone())
            } else if target.starts_with("Absent") {
                (*journal, key.clone(), String::new(), "Absent".to_owned())
            } else {
                (*journal, key.clone(), String::new(), target.clone())
            }
        })
        .collect();
    rows.sort();
    rows
}

/// After `what`: the live table was patched (not rebuilt) and equals a fresh
/// build of the same generation; the reference-only names are the pre-table
/// page walk's (`referenced_page_names_for_editor_walk`, the old code), in
/// its spelling; and, with `restart`, every name resolves as in a store
/// opened afresh on the same files (continuing with that store).
fn assert_fresh(root: &Path, store: Arc<Store>, what: &str, restart: bool) -> Arc<Store> {
    let (patched, fresh) = table_rows(&store);
    let patched = patched.unwrap_or_else(|| panic!("{what}: the table was dropped, not patched"));
    assert_eq!(patched, fresh, "patched table after {what}");
    let live = inventory(&store);
    let walk: BTreeMap<String, String> = store
        .whole_graph()
        .unwrap()
        .graph
        .referenced_page_names_for_editor_walk(None)
        .into_iter()
        .map(|name| (tine_core::refs::page_key(&name), name))
        .collect();
    for ((_, key, name), target) in &live {
        if !target.starts_with("Existing") {
            if let Some(spelling) = walk.get(key) {
                assert_eq!(name, spelling, "spelling of {key:?} after {what}");
            }
        }
    }
    let listed: Vec<&String> = live
        .iter()
        .filter(|(_, target)| !target.starts_with("Absent"))
        .map(|((_, key, _), _)| key)
        .collect();
    // Namespace parents are absent too, but come from no reference.
    let absent: Vec<&String> = live
        .iter()
        .filter(|(_, target)| target.starts_with("Absent") && !target.ends_with(" parent"))
        .map(|((_, key, _), _)| key)
        .collect();
    let expected: Vec<&String> = walk.keys().filter(|key| !listed.contains(key)).collect();
    assert_eq!(absent, expected, "reference-only names after {what}");
    if !restart {
        return store;
    }
    store.close();
    drop(store);
    let reopened = open(root, Default::default());
    assert_eq!(
        resolution(&live),
        resolution(&inventory(&reopened)),
        "live inventory after {what}"
    );
    reopened
}

/// Re-scan until `done` holds (an outside edit may publish in parts).
fn settle(store: &Store, what: &str, done: impl Fn(&Store) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        store.refresh(crate::Depth::Stamps).unwrap();
        if done(store) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: never converged");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn names(store: &Store) -> Vec<String> {
    inventory(store)
        .into_keys()
        .map(|(_, _, name)| name)
        .collect()
}

fn unreadable(store: &Store, rel: &str) -> bool {
    store
        .whole_graph()
        .unwrap()
        .unreadable_files()
        .iter()
        .any(|(id, _)| id.as_str() == rel)
}

/// The inventory as the window keys it: (journal?, key, name) -> entry text.
fn image(entries: &[InventoryEntry]) -> BTreeMap<(bool, String, String), String> {
    entries
        .iter()
        .map(|entry| {
            (
                (
                    entry.is_journal,
                    tine_core::refs::page_key(&entry.name),
                    entry.name.clone(),
                ),
                // `link` is part of the answer the window holds: a delta that
                // left it stale would route a link to the wrong page; so is
                // why a fileless entry exists (a namespace parent), and
                // whether an empty file yields its name (EMPTY-ALIAS).
                format!(
                    "{:?} {:?} {:?}{}{}",
                    entry.target,
                    entry.day,
                    entry.link,
                    if entry.namespace_parent {
                        " parent"
                    } else {
                        ""
                    },
                    if entry.empty_alias {
                        " empty-alias"
                    } else {
                        ""
                    }
                ),
            )
        })
        .collect()
}

fn inventory(store: &Store) -> Image {
    image(
        &store
            .whole_graph()
            .unwrap()
            .inventory(InventoryScope::All)
            .0,
    )
}

/// One publication's delta as the window receives it.
#[derive(Clone)]
struct Delta {
    keys: Vec<String>,
    entries: Vec<InventoryEntry>,
}

/// What one save published: (inventory changed in any publication, every
/// publication's delta, or `None` once one changed without a delta).
fn save(
    store: &Arc<Store>,
    id: &str,
    edit: impl FnOnce(&mut tine_core::model::PageDto),
) -> (bool, Option<Vec<Delta>>) {
    let id = PageId::from(id);
    let sub = store.subscribe();
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    edit(&mut doc);
    let mut tx = store.page_plan(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::Existing(read.rev),
        &doc,
    );
    crate::test_host_writer::HostWriter::for_store(store)
        .apply(tx.plan().expect("planned"))
        .expect("applied");
    let (mut changed, mut deltas) = (false, Some(Vec::new()));
    while let Some(change) = sub.try_recv().unwrap() {
        let wire = serde_json::to_value(&change).unwrap();
        if wire["inventoryChanged"] != true {
            assert!(change.inventory_delta().is_none());
            continue;
        }
        changed = true;
        match (change.inventory_delta(), deltas.as_mut()) {
            (Some((keys, entries)), Some(all)) => all.push(Delta {
                keys: keys.to_vec(),
                entries: entries.0.clone(),
            }),
            _ => deltas = None,
        }
    }
    (changed, deltas)
}

/// The window's patch: drop the listed keys' alias and reference-only
/// entries, add the delta's (`src/pageIndex.ts` `applyInventoryDelta`).
fn apply(held: &mut BTreeMap<(bool, String, String), String>, delta: &Delta) {
    held.retain(|(_, key, _), target| {
        !(delta.keys.contains(key) && !target.starts_with("Existing"))
    });
    held.extend(image(&delta.entries));
}

fn set_block(raw: &str) -> impl FnOnce(&mut tine_core::model::PageDto) + '_ {
    move |doc| doc.blocks[0].raw = raw.to_owned()
}

/// Every save keeps the window's patched inventory equal to a fresh read, and
/// signals exactly when the inventory changed (the old code signalled every
/// change of the saved page's own name set, and always as a refetch).
fn assert_patch(store: &Arc<Store>, id: &str, raw: &str) -> usize {
    let before = inventory(store);
    let mut held = before.clone();
    let (changed, deltas) = save(store, id, set_block(raw));
    let deltas = deltas.unwrap_or_else(|| panic!("{raw:?}: a refetch instead of a delta"));
    let mut entries = 0;
    for delta in &deltas {
        entries += delta.keys.len() + delta.entries.len();
        apply(&mut held, delta);
    }
    // The new generation inherited the table by patch (no rebuild), and the
    // patch equals a fresh build.
    let view = store.whole_graph().unwrap();
    let patched = view
        .graph
        .referenced_name_table_built()
        .expect("inherited, not rebuilt");
    assert_eq!(
        crate::model::ReadSnapshot::referenced_name_rows(patched),
        crate::model::ReadSnapshot::referenced_name_rows(&view.graph.referenced_name_table_fresh()),
        "patched table after {raw:?}"
    );
    let after = inventory(store);
    assert_eq!(held, after, "patched inventory after {raw:?}");
    assert_eq!(
        changed,
        before != after,
        "signal iff the inventory moved, {raw:?}"
    );
    entries
}

#[test]
fn typing_a_link_to_an_existing_page_signals_only_names_that_moved() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(&[
        ("pages/A.md", "- start\n"),
        ("pages/Existing Page.md", "- here\n"),
        ("pages/B.md", "- [[Ex]] and [[shared ref]]\n"),
        ("pages/C.md", "- c\n"),
    ]);
    // Autopaired typing: each intermediate `[[prefix]]` is a reference, so
    // a save may add the new prefix and drop the previous one; the window
    // patches those few entries and never refetches the inventory.
    let name = "Existing Page";
    let mut payload = 0;
    for k in 1..=name.len() {
        let entries = assert_patch(&store, "pages/A.md", &format!("start [[{}]]", &name[..k]));
        assert!(entries <= 4, "{k}: {entries} delta entries");
        payload += entries;
    }
    assert!(payload <= 4 * name.len());
    // Text around the finished link, or the same name linked again in
    // another spelling, changes nothing graph-wide: no signal at all.
    let entries = assert_patch(
        &store,
        "pages/A.md",
        "edited [[Existing Page]] [[existing page]]",
    );
    assert_eq!(entries, 0, "linking an existing page again signals nothing");
    assert_patch(&store, "pages/C.md", "[[Ex]]");
    store.close();
}

#[test]
fn deltas_track_aliases_spellings_journals_and_namespaces() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(&[
        ("pages/A.md", "- start\n"),
        (
            "pages/B.md",
            "alias:: Nick\n- [[Mixed Case]] [[Team/Child]]\n",
        ),
        ("pages/C.md", "- [[mixed case]]\n"),
        ("journals/2026_06_26.md", "- day\n"),
        ("pages/Owner.md", "alias:: Jun 26th, 2026\n- owner\n"),
    ]);
    let id = "pages/A.md";
    // A new reference-only name, then its removal.
    assert_patch(&store, id, "[[Brand New]]");
    assert_patch(&store, id, "plain");
    // Another spelling of a name B already spells: the least slot keeps its
    // spelling, whichever that is.
    assert_patch(&store, id, "[[MIXED CASE]]");
    assert_patch(&store, id, "plain");
    // A reference to an alias, and to a namespaced name.
    assert_patch(&store, id, "[[nick]] [[Team/Child]] [[Team/Other]]");
    // A journal-claimed name referenced and aliased.
    assert_patch(&store, id, "[[Jun 26th, 2026]]");
    // Own alias edits: a new alias, a second owner of an existing alias,
    // then removal.
    let alias = |pre: Option<&'static str>| {
        move |doc: &mut tine_core::model::PageDto| doc.pre_block = pre.map(str::to_owned)
    };
    for pre in [Some("alias:: Fresh Alias"), Some("alias:: Nick"), None] {
        let mut held = inventory(&store);
        let (changed, deltas) = save(&store, id, alias(pre));
        assert!(changed, "{pre:?}");
        for delta in deltas.expect("an alias edit is a delta") {
            apply(&mut held, &delta);
        }
        assert_eq!(held, inventory(&store), "after {pre:?}");
    }
    // Removing B's references leaves C's spelling.
    assert_patch(&store, "pages/B.md", "nothing");
    store.close();
}

/// The namespace parents `inventory(All)` lists (model rule 1, SPEC §3.2,
/// CM3-3; conformance S8): (name, flag) of every fileless, non-alias entry.
fn fileless(store: &Store) -> Vec<(String, bool)> {
    store
        .whole_graph()
        .unwrap()
        .inventory(InventoryScope::All)
        .0
        .iter()
        .filter(|entry| matches!(entry.target, crate::Resolved::Absent { .. }))
        .map(|entry| (entry.name.clone(), entry.namespace_parent))
        .collect()
}

/// CM step 2 batch 10 (S8): `n/y` makes `n` a page, from a page file's name
/// (title or file name, journals too) and from a reference, at every level;
/// a blank file supplies its own page only (OG `extract` skips blank
/// content); a parent some file claims, some page declares as an alias or
/// some reference names is listed as that, not as a parent.
#[test]
fn namespace_parents_are_listed_pages() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(&[
        ("pages/x.md", "title:: Work/Plan\n- body\n"),
        ("pages/Notes.md", "- [[Area/Sub/Leaf]] [[topic]]\n"),
        ("pages/Empty%2FChild.md", ""),
        ("pages/Owned%2FChild.md", "- c\n"),
        ("pages/Owned.md", "- the parent has its own file\n"),
        (
            "pages/Holder.md",
            "alias:: Nick\n- [[nick/sub]] [[Topic/Sub]]\n",
        ),
    ]);
    let mut listed = fileless(&store);
    listed.sort();
    assert_eq!(
        listed,
        [
            ("Area".to_owned(), true),
            ("Area/Sub".to_owned(), true),
            ("Area/Sub/Leaf".to_owned(), false),
            ("Topic/Sub".to_owned(), false),
            ("Work".to_owned(), true),
            ("nick/sub".to_owned(), false),
            ("topic".to_owned(), false),
        ]
    );
    // The answer page queries and All Pages' counts read lists them too.
    let view = store.whole_graph().unwrap();
    let mut names = view.graph.reference_only_page_names();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "Area",
            "Area/Sub",
            "Area/Sub/Leaf",
            "Topic/Sub",
            "Work",
            "nick/sub",
            "topic"
        ]
    );
    // `[[Work]]` opens the parent page itself.
    assert_eq!(
        view.link_target("work"),
        crate::LinkTarget {
            name: "work".to_owned(),
            is_journal: false
        }
    );
    store.close();
}

/// A live edit that names the first child of a namespace lists its parents,
/// one that removes the last child drops them, and a parent's spelling
/// follows its children: all as a delta, equal to a fresh read.
#[test]
fn deltas_track_namespace_parents() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(&[
        ("pages/A.md", "- start\n"),
        ("pages/B.md", "- [[team/child]]\n"),
        ("pages/Docs%2FGuide.md", "- guide\n"),
    ]);
    let parents = |store: &Store| -> Vec<String> {
        let mut names: Vec<String> = fileless(store)
            .into_iter()
            .filter(|(_, parent)| *parent)
            .map(|(name, _)| name)
            .collect();
        names.sort();
        names
    };
    assert_eq!(parents(&store), ["Docs", "team"]);
    let id = "pages/A.md";
    assert_patch(&store, id, "[[Proj/Sub/Leaf]]");
    assert_eq!(parents(&store), ["Docs", "Proj", "Proj/Sub", "team"]);
    // A second spelling: the least one names the parent.
    assert_patch(&store, id, "[[Proj/Sub/Leaf]] [[TEAM/other]]");
    assert_eq!(parents(&store), ["Docs", "Proj", "Proj/Sub", "TEAM"]);
    // Naming a parent directly makes it an ordinary reference-only page.
    assert_patch(&store, id, "[[Proj/Sub/Leaf]] [[Proj]]");
    assert_eq!(parents(&store), ["Docs", "Proj/Sub", "team"]);
    // A child under a file-supplied parent, then the last reference child
    // goes: the file still supplies its parent.
    assert_patch(&store, id, "[[docs/faq]]");
    assert_patch(&store, id, "plain");
    assert_eq!(parents(&store), ["Docs", "team"]);
    assert_patch(&store, "pages/B.md", "plain");
    assert_eq!(parents(&store), ["Docs"]);
    store.close();
}

/// The `link` answer (what `[[name]]` opens, `WholeGraph::link_target`)
/// travels in the delta and equals a fresh read after every alias edit: a
/// journal's alias opens the journal (D-F2), and an alias spelled as a day with
/// no journal file opens its owner page (S6).
#[test]
fn a_delta_keeps_the_link_answer_fresh() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(&[
        ("pages/A.md", "- [[Launch]] [[Jun 28th, 2026]]\n"),
        ("pages/Owner.md", "- owner\n"),
        ("journals/2026_06_27.md", "- day\n"),
    ]);
    // What the window routes by: the entry's `link`, else the entry itself.
    let link = |store: &Store, name: &str| {
        let key = tine_core::refs::page_key(name);
        let inventory = store.whole_graph().unwrap().inventory(InventoryScope::All);
        let entry = inventory
            .0
            .iter()
            .find(|entry| tine_core::refs::page_key(&entry.name) == key)
            .unwrap_or_else(|| panic!("{name} listed"))
            .clone();
        entry.link.unwrap_or(crate::LinkTarget {
            name: entry.name,
            is_journal: entry.is_journal,
        })
    };
    let pre = |text: Option<&'static str>| {
        move |doc: &mut tine_core::model::PageDto| doc.pre_block = text.map(str::to_owned)
    };
    let steps: [(&str, Option<&'static str>); 4] = [
        ("journals/2026_06_27.md", Some("alias:: Launch")),
        ("pages/Owner.md", Some("alias:: [[Jun 28th, 2026]]")),
        ("journals/2026_06_27.md", None),
        ("pages/Owner.md", None),
    ];
    for (step, (id, text)) in steps.into_iter().enumerate() {
        let mut held = inventory(&store);
        let (changed, deltas) = save(&store, id, pre(text));
        assert!(changed, "step {step}: an alias edit moves the inventory");
        for delta in deltas.expect("an alias edit is a delta") {
            apply(&mut held, &delta);
        }
        assert_eq!(held, inventory(&store), "step {step}: patched equals fresh");
        let (launch, day) = (link(&store, "Launch"), link(&store, "Jun 28th, 2026"));
        match step {
            0 => assert!(
                launch.is_journal && launch.name == "Jun 27th, 2026",
                "{launch:?}"
            ),
            1 => assert!(!day.is_journal && day.name == "Owner", "{day:?}"),
            2 => assert!(!launch.is_journal && launch.name == "Launch", "{launch:?}"),
            _ => assert!(day.is_journal && day.name == "Jun 28th, 2026", "{day:?}"),
        }
    }
    store.close();
}

#[test]
fn a_page_created_or_renamed_still_refetches() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(&[("pages/A.md", "- start\n")]);
    let sub = store.subscribe();
    let new = PageId::from("pages/New.md");
    let mut dto = store.page(&PageId::from("pages/A.md")).unwrap().doc;
    dto.blocks[0].raw = "[[Elsewhere]]".into();
    let mut tx = store.page_plan(Some(EditKind::CreatePage));
    tx.save_page(&[EditKind::CreatePage], &new, SaveBase::CreateNew, &dto);
    crate::test_host_writer::HostWriter::for_store(&store)
        .apply(tx.plan().unwrap())
        .unwrap();
    let mut refetch = false;
    while let Some(change) = sub.try_recv().unwrap() {
        let wire = serde_json::to_value(&change).unwrap();
        refetch |= wire["inventoryChanged"] == true && change.inventory_delta().is_none();
    }
    assert!(refetch, "a claim change is a refetch, never a delta");
    store.close();
}

fn apply_plan(store: &Arc<Store>, tx: crate::transaction::PagePlan<'_>) {
    crate::test_host_writer::HostWriter::for_store(store)
        .apply(tx.plan().expect("planned"))
        .expect("applied");
}

/// Review of og-perf-names (required neighbour): the table is index-driven,
/// so a page that leaves the graph must leave it too, or a ghost reference
/// name stays listed. After every way a page leaves, moves or comes back,
/// the patched table equals a fresh build and the inventory equals a fresh
/// store's.
#[test]
fn the_table_stays_fresh_when_pages_leave_move_or_turn_unreadable() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let dir = graph(&[
        ("pages/A.md", "- [[Shared]] [[Only A]]\n"),
        (
            "pages/B.md",
            "alias:: Bee\n\n- [[shared]] [[Only B]] [[Bee]]\n",
        ),
        ("pages/C.md", "- [[SHARED]] [[Only C]]\n"),
        ("pages/D.md", "- [[Only D]] [[Shared]]\n"),
        ("journals/2026_06_26.md", "- [[Only J]] [[Shared]]\n"),
    ]);
    let root = dir.path();
    let store = open(root, Default::default());
    // Own delete of the page whose slot spells "Shared".
    let a = PageId::from("pages/A.md");
    let mut tx = store.page_plan(Some(EditKind::DeletePage));
    tx.trash(&a.file(), store.page(&a).unwrap().rev, crate::TrashIf::Any);
    apply_plan(&store, tx);
    assert!(!names(&store).contains(&"Only A".to_owned()));
    let store = assert_fresh(root, store, "own delete", true);
    // Own rename: the page's names move to another slot.
    let b = PageId::from("pages/B.md");
    let dest = store.file_id(crate::Area::Pages, "Renamed B.md").unwrap();
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    tx.move_file(&b.file(), store.page(&b).unwrap().rev, &dest, None);
    apply_plan(&store, tx);
    let store = assert_fresh(root, store, "own rename", true);
    // External rename.
    fs::rename(root.join("pages/C.md"), root.join("pages/C moved.md")).unwrap();
    settle(&store, "external rename", |s| {
        names(s).contains(&"C moved".to_owned()) && !names(s).contains(&"C".to_owned())
    });
    let store = assert_fresh(root, store, "external rename", true);
    // External modify: a reference dropped, another added, a spelling moved.
    fs::write(
        root.join("pages/D.md"),
        "- [[Only D2]] [[sHaReD]] [[Bee]]\n",
    )
    .unwrap();
    settle(&store, "external modify", |s| {
        names(s).contains(&"Only D2".to_owned())
    });
    let store = assert_fresh(root, store, "external modify", true);
    // The page turns unreadable (undecodable preamble): its names leave.
    fs::write(
        root.join("pages/D.md"),
        b"tags:: caf\xe9\n\n- [[Only D3]] [[Shared]]\n".as_slice(),
    )
    .unwrap();
    settle(&store, "turned unreadable", |s| unreadable(s, "pages/D.md"));
    // A running store keeps the page's last readable parse (so its names
    // stay listed, as the old page walk listed them); a store launched on
    // these files never read it, so there is no restart comparison here.
    let store = assert_fresh(root, store, "turned unreadable", false);
    // ... and readable again.
    fs::write(root.join("pages/D.md"), "- [[Only D4]] [[Shared]]\n").unwrap();
    settle(&store, "readable again", |s| {
        !unreadable(s, "pages/D.md") && names(s).contains(&"Only D4".to_owned())
    });
    let store = assert_fresh(root, store, "readable again", true);
    // The journal's reference is the last one standing for its key.
    let j = "journals/2026_06_26.md";
    fs::write(root.join(j), "- day\n").unwrap();
    settle(&store, "journal modify", |s| {
        !names(s).contains(&"Only J".to_owned())
    });
    assert_fresh(root, store, "journal modify", true).close();
}

/// A launch from the checkpoint holds no table; the first inventory read
/// builds it from the checkpointed per-page names (no parse), equal to the
/// table before the restart, and the next publication patches it.
#[test]
fn the_table_after_a_checkpoint_launch_equals_the_one_before() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let dir = graph(&[
        ("pages/A.md", "- [[Shared]] [[Only A]]\n"),
        ("pages/B.md", "alias:: Bee\n\n- [[shared]] [[Only B]]\n"),
        ("journals/2026_06_26.md", "- [[Only J]]\n"),
    ]);
    // Outside the checkpoint's racy-stamp window, so the launch reuses it.
    let past = std::time::SystemTime::now() - Duration::from_secs(3600);
    for rel in ["pages/A.md", "pages/B.md", "journals/2026_06_26.md"] {
        fs::File::options()
            .write(true)
            .open(dir.path().join(rel))
            .unwrap()
            .set_modified(past)
            .unwrap();
    }
    let cp = dir.path().join("launch.bin");
    let options = || crate::OpenOptions {
        watch: crate::WatchMode::Poll,
        launch_checkpoint: Some(cp.clone()),
        ..Default::default()
    };
    let store = open(dir.path(), options());
    store.whole_graph_reconciled().unwrap();
    let (before, _) = table_rows(&store);
    let before = before.expect("built by the inventory read");
    let names_before = inventory(&store);
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(crate::CheckpointWrite::Written { .. })
    ));
    store.close();
    drop(store);
    let store = Arc::new(Store::open(dir.path(), options()).unwrap().0);
    let launched = store.whole_graph_reconciled().unwrap();
    assert_eq!(
        store.diagnostics()["checkpoint"]["load"]["outcome"],
        "loaded"
    );
    assert!(
        launched.graph.referenced_name_table_built().is_none(),
        "a launch holds no table"
    );
    assert!(
        launched.graph.alias_list_built(),
        "the launch generation holds the alias relation"
    );
    drop(launched);
    assert_eq!(inventory(&store), names_before);
    let (built, fresh) = table_rows(&store);
    assert_eq!(built.as_ref(), Some(&before), "table after the launch");
    assert_eq!(fresh, before);
    save(&store, "pages/A.md", set_block("[[Only A2]]"));
    let (patched, fresh) = table_rows(&store);
    assert_eq!(patched.expect("patched"), fresh, "after the first save");
    store.close();
}

/// Review of og-perf-names (perf follow-up): the delta asks the OLD
/// generation's alias relation for a key's owners. `[[` completion builds the
/// name table without the relation, so a generation holding the table but
/// not the relation would make the next publication walk every page. Such a
/// generation is never published: each publication builds its generation's
/// relation (`change_summary.rs`, at the first publication and after a
/// rebuild; inherited per page otherwise) before a later publication reads
/// it. (`answer_changes` would refetch rather than build it.)
#[test]
fn every_published_generation_already_holds_the_alias_relation() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let dir = graph(&[
        ("pages/A.md", "- start\n"),
        ("pages/B.md", "alias:: Bee\n\n- [[Shared]]\n"),
    ]);
    // No inventory read: the editor's `[[` completion is the first reader.
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let held = |what: &str| {
        let view = store.whole_graph().unwrap();
        assert!(view.graph.alias_list_built(), "{what}: no alias relation");
        view
    };
    let view = held("open");
    view.complete_page_names("Sh", 20, None, false, None);
    assert!(view.graph.referenced_name_table_built().is_some());
    drop(view);
    // The table rides on, and every later generation has the relation
    // before a delta reads it: the saves are deltas, not refetches.
    for raw in ["[[Brand New]]", "[[Bee]] [[Second]]", "plain"] {
        let (changed, deltas) = save(&store, "pages/A.md", set_block(raw));
        assert!(changed, "{raw}");
        assert!(deltas.is_some(), "{raw}: a delta, not a refetch");
        held(raw);
    }
    fs::write(dir.path().join("pages/B.md"), "alias:: Bee, Wasp\n\n- x\n").unwrap();
    settle(&store, "external alias edit", |s| {
        names(s).contains(&"Wasp".to_owned())
    });
    held("external alias edit");
    store.close();
}

/// `[[` completion visits only the editing page per keystroke (the old union
/// re-read every page's names on every call), with the same answers as the
/// page-by-page union (`referenced_page_names_for_editor_walk`).
#[test]
fn bracket_completion_reads_the_resident_table_not_every_page() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let mut files = vec![
        (
            "pages/Editing.md".to_owned(),
            "- [[Typed Na]] [[Kept]]\n- [[Other Block]]\n".to_owned(),
        ),
        (
            "pages/Twin.md".to_owned(),
            "- [[kept]] [[Only Twin]]\n".to_owned(),
        ),
    ];
    for i in 0..200 {
        files.push((
            format!("pages/P{i}.md"),
            format!("- [[Hub]] [[Name {i}]]\n"),
        ));
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_dir, store) = fixture(&files);
    let view = store.whole_graph().unwrap();
    let id = view
        .graph
        .pages
        .iter()
        .find(|(entry, _)| entry.name == "Editing")
        .unwrap()
        .1
        .roots[0]
        .uuid
        .clone();
    crate::model::referenced_names::take_page_probes();
    for (k, typed) in ["Ty", "Typ", "Type"].into_iter().enumerate() {
        let hits = view.complete_page_names(
            typed,
            20,
            Some(("Editing", &id, "[[Kept]] [[Baseline Only]]")),
            false,
            None,
        );
        assert!(
            hits.iter().all(|(entry, _)| entry.name != "Typed Na"),
            "baseline hides the live typed name"
        );
        if k == 0 {
            assert!(view
                .complete_page_names(
                    "Baseline",
                    20,
                    Some(("Editing", &id, "[[Kept]] [[Baseline Only]]")),
                    false,
                    None
                )
                .iter()
                .any(|(entry, _)| entry.name == "Baseline Only"));
        }
    }
    let probes = crate::model::referenced_names::take_page_probes();
    assert!(
        probes <= 4,
        "pages visited by 4 completions: {probes} (old code: every page per call)"
    );
    for baseline in [
        None,
        Some(("Editing", id.as_str(), "[[Kept]] [[Baseline Only]]")),
        Some(("Editing", id.as_str(), "")),
        Some(("Editing", id.as_str(), "[[KEPT]] [[hub]]")),
        Some(("Twin", "no-such-block", "")),
        Some(("Missing Page", id.as_str(), "[[x]]")),
    ] {
        let mut table: Vec<String> = view
            .graph
            .referenced_names_for_completion(baseline)
            .into_iter()
            .map(|name| name.into_owned())
            .collect();
        let mut walk = view.graph.referenced_page_names_for_editor_walk(baseline);
        table.sort();
        walk.sort();
        assert_eq!(table, walk, "{baseline:?}");
    }
    store.close();
}

/// Property-name completion is one keys-only walk per session; values come
/// for the chosen key only, equal to the full facets' values for it.
#[test]
fn property_completion_splits_keys_from_one_keys_values() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(&[
        (
            "pages/A.md",
            "type:: [[Book]], [[Paper]]\n- status:: done\n  rating:: 5\n",
        ),
        (
            "pages/B.md",
            "- status:: todo\n- id:: 00000000-0000-4000-8000-000000000001\n",
        ),
    ]);
    let view = store.whole_graph().unwrap();
    let full = view.property_facets(crate::FacetPolicy::Truncated).unwrap();
    let walks = || crate::query::PROPERTY_FACET_PAGE_WALKS.with(|walks| walks.replace(0));
    walks();
    let facets = |policy| view.property_facets(policy).unwrap();
    let keys = facets(crate::FacetPolicy::CompletionKeys);
    assert_eq!(
        keys,
        full.iter()
            .map(|(key, _)| (key.clone(), Vec::new()))
            .collect::<Vec<_>>()
    );
    assert_eq!(walks(), 2, "one walk over the two pages");
    for row in &full {
        assert_eq!(
            facets(crate::FacetPolicy::CompletionValues(row.0.clone())),
            [row.clone()]
        );
    }
    assert!(facets(crate::FacetPolicy::CompletionValues("no-such-key".into())).is_empty());
    store.close();
}

/// Resident cost of the name table on a real-sized graph (og-perf-names
/// receipt), not a gate: `TINE_NAMES_PROBE=<copy of a generated graph> cargo
/// test -p tine-store --lib names_table_resident_probe -- --ignored --nocapture`.
/// Per key: one tree node (`Arc<Node>`, 56 B + 16 B counts), one entry
/// (`Arc<(String, KeyReferrers)>`, 72 B + 16 B), the key and spelling bytes,
/// and 8 B per referring page; each heap block rounded to 16 B.
#[test]
#[ignore]
fn names_table_resident_probe() {
    let Ok(dir) = std::env::var("TINE_NAMES_PROBE") else {
        return;
    };
    let store = Store::open(std::path::Path::new(&dir), Default::default())
        .unwrap()
        .0;
    let view = store.whole_graph().unwrap();
    let table = view.graph.referenced_name_table();
    let round = |bytes: usize| bytes.div_ceil(16) * 16;
    let (mut keys, mut slots, mut bytes) = (0usize, 0usize, 0usize);
    for (key, referrers) in table.iter() {
        keys += 1;
        slots += referrers.slots().len();
        bytes += round(16 + 56)
            + round(16 + 72)
            + round(key.len())
            + round(referrers.spelling.len())
            + round(8 * referrers.slots().len());
    }
    let pages = view.graph.pages.len();
    println!(
        "names table: pages={pages} keys={keys} referrer_slots={slots} bytes={bytes} bytes_per_page={:.1}",
        bytes as f64 / pages as f64
    );
    store.close();
}

/// What `[[name]]` opens, as the window reads it: the `All` entry's `link`,
/// else the entry itself; with the entry's `empty_alias` bit.
fn window_link(store: &Store, name: &str) -> (String, bool, bool) {
    let key = tine_core::refs::page_key(name);
    let view = store.whole_graph().unwrap();
    let inventory = view.inventory(InventoryScope::All);
    let entry = inventory
        .0
        .iter()
        .find(|entry| !entry.is_journal && tine_core::refs::page_key(&entry.name) == key)
        .unwrap_or_else(|| panic!("{name} listed"))
        .clone();
    let link = entry.link.clone().unwrap_or(crate::LinkTarget {
        name: entry.name.clone(),
        is_journal: entry.is_journal,
    });
    // The per-name oracle is the same answer.
    assert_eq!(
        view.link_target(name),
        link,
        "link_target({name}) vs inventory"
    );
    (link.name, link.is_journal, entry.empty_alias)
}

/// EMPTY-ALIAS (D-20261103-CMA, OG `page-empty-or-dummy?`): with `Machine
/// Learning` declaring `alias:: ML`, `[[ML]]` opens Machine Learning while
/// `pages/ML.md` has no real content, and `ML.md` once it has any. The empty
/// page stays a real, listed page (its entry is `Existing`).
#[test]
fn empty_alias_page_yields_its_name_to_the_alias_owner() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let owner = ("pages/Machine Learning.md", "alias:: ML\n\n- ml\n");
    let notes = ("pages/Notes.md", "- [[ML]]\n");
    // (ML.md text, does `[[ML]]` open Machine Learning?)
    let cases: [(&str, &str, bool); 8] = [
        ("no blocks", "", true),
        ("a blank line", "\n", true),
        ("one empty block", "- \n", true),
        ("one block with a space", "-  \n", false),
        ("two empty blocks", "- \n- \n", false),
        ("page properties only", "tags:: x\n", false),
        ("a title only", "title:: ML\n", false),
        ("content", "- my own ML notes\n", false),
    ];
    for (what, text, yields) in cases {
        let (_dir, store) = fixture(&[owner, notes, ("pages/ML.md", text)]);
        let (name, journal, empty_alias) = window_link(&store, "ML");
        let want = if yields { "Machine Learning" } else { "ML" };
        assert_eq!(
            (name.as_str(), journal),
            (want, false),
            "{what}: [[ML]] opens"
        );
        assert_eq!(empty_alias, yields, "{what}: empty_alias");
        // Still a real page: listed with its own file.
        let view = store.whole_graph().unwrap();
        assert!(
            matches!(view.resolve("ML", false), crate::Resolved::Existing { ref id, .. } if id.as_str() == "pages/ML.md"),
            "{what}: ML.md is still the page named ML"
        );
        store.close();
    }
    // No page declares the name: the empty file opens itself.
    let (_dir, store) = fixture(&[notes, ("pages/ML.md", "")]);
    assert_eq!(window_link(&store, "ML"), ("ML".into(), false, false));
    store.close();
    // The declarer's kind does not matter: a journal declaring `alias:: n/y`
    // wins over an empty `n/y` page (model C-g).
    let (_dir, store) = fixture(&[
        ("journals/2026_10_27.md", "alias:: n/y\n\n- see [[A]]\n"),
        ("pages/N%2Fy.md", ""),
    ]);
    assert_eq!(
        window_link(&store, "N/y"),
        ("Oct 27th, 2026".into(), true, true)
    );
    store.close();
}

/// EMPTY-ALIAS follows live edits without a restart: typing the first
/// character into the empty `ML.md` makes `[[ML]]` open it, clearing it again
/// makes `[[ML]]` open Machine Learning, and dropping or adding the alias
/// moves it too. The window's held inventory (patched by deltas, or refetched
/// when a publication carries none) always equals a fresh read.
#[test]
fn empty_alias_follows_live_edits() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let (_dir, store) = fixture(&[
        ("pages/Machine Learning.md", "alias:: ML\n\n- ml\n"),
        ("pages/Notes.md", "- [[ML]]\n"),
        ("pages/ML.md", ""),
    ]);
    let step = |store: &Arc<Store>, id: &str, edit: &dyn Fn(&mut tine_core::model::PageDto)| {
        let mut held = inventory(store);
        let (changed, deltas) = save(store, id, |doc| edit(doc));
        match deltas {
            Some(deltas) => deltas.iter().for_each(|delta| apply(&mut held, delta)),
            None => held = inventory(store),
        }
        let fresh = inventory(store);
        assert_eq!(held, fresh, "the window's inventory after saving {id}");
        changed
    };
    assert_eq!(window_link(&store, "ML").0, "Machine Learning");
    let typed = |text: &'static str| {
        move |doc: &mut tine_core::model::PageDto| {
            doc.blocks = vec![tine_core::model::BlockDto {
                raw: text.to_owned(),
                ..Default::default()
            }];
        }
    };
    assert!(
        step(&store, "pages/ML.md", &typed("m")),
        "first character signals"
    );
    assert_eq!(window_link(&store, "ML"), ("ML".into(), false, false));
    assert!(step(&store, "pages/ML.md", &typed("")), "clearing signals");
    assert_eq!(
        window_link(&store, "ML"),
        ("Machine Learning".into(), false, true)
    );
    let pre = |text: Option<&'static str>| {
        move |doc: &mut tine_core::model::PageDto| doc.pre_block = text.map(str::to_owned)
    };
    assert!(step(&store, "pages/Machine Learning.md", &pre(None)));
    assert_eq!(window_link(&store, "ML"), ("ML".into(), false, false));
    assert!(step(
        &store,
        "pages/Machine Learning.md",
        &pre(Some("alias:: ML"))
    ));
    assert_eq!(
        window_link(&store, "ML"),
        ("Machine Learning".into(), false, true)
    );
    store.close();
}
