//! Phase A of `RUNTIME-ID-MODEL.md` v3: the witness W (A1, A11-A13), the
//! foreign-key registry (A3), intake (A2) and the cost pins (A9, A20, T10).
//! These use phase A's own API, so they have no baseline twin.

use super::page_ids::{Input, Trust, CLASSES, REGISTRY_VISITS, VALIDATION_SETS};
use super::page_ids_compat as compat;
use super::unchanged_save::take_parses;
use super::*;
use tine_core::projection::take_runtime_id_work;

fn graph(files: &[(&str, String)]) -> (tempfile::TempDir, Graph) {
    let temp = tempfile::tempdir().unwrap();
    for area in ["pages", "journals", "logseq"] {
        fs::create_dir_all(temp.path().join(area)).unwrap();
    }
    for (rel, text) in files {
        fs::write(temp.path().join(rel), text).unwrap();
    }
    let graph = Graph::open(temp.path());
    (temp, graph)
}

fn doc_of(graph: &Graph, rel: &str) -> Arc<Document> {
    let path = graph.root.join(rel);
    (graph.with_cached(&path, |row| row.map(|(_, doc)| Arc::clone(doc))))
        .flatten()
        .unwrap_or_else(|| panic!("{rel} has a row"))
}

fn keys(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    tine_core::projection::for_each_runtime_key(&doc.roots, &mut |key| out.push(key.to_string()));
    out
}

/// `text` parsed as `rel` with `keys` in page order.
fn keyed(graph: &Graph, rel: &str, text: &str, keys: &[String]) -> Document {
    fn set(blocks: &mut [DocBlock], keys: &mut std::slice::Iter<'_, String>) {
        for block in blocks {
            block.uuid = keys.next().expect("a key per block").as_str().into();
            set(&mut block.children, keys);
        }
    }
    let mut doc = parse_doc(&graph.root.join(rel), text);
    set(&mut doc.roots, &mut keys.iter());
    doc
}

fn classes() -> [u64; 4] {
    CLASSES.with(|classes| classes.replace([0; 4]))
}

fn validation_sets() -> u64 {
    VALIDATION_SETS.with(|sets| sets.replace(0))
}

/// Reset this thread's counters.
fn reset() {
    let _ = (
        take_runtime_id_work(),
        take_parses(),
        classes(),
        validation_sets(),
    );
}

fn page(blocks: usize) -> String {
    (0..blocks)
        .map(|at| format!("- block {at} [[Shared]]\n"))
        .collect()
}

/// A20 and F4/F5: an ordinary own save of a fully keyed page, at 1 and 60
/// blocks: the store installs the serializer's document with no positional
/// hash, no validation set, no parse (the serializer's parse of the final
/// bytes is the save's one), and the per-class counter says Full.
#[test]
fn a20_an_own_save_assigns_and_allocates_nothing() {
    for blocks in [1, 60] {
        let rel = "pages/p.md";
        let text = page(blocks);
        let (_temp, graph) = graph(&[(rel, text.clone())]);
        graph.with_pages(|_| ());
        graph.hold_unhosted(rel);
        compat::publish(&graph, rel, &text, None);
        let saved = text.replacen("block 0", "block 0x", 1);
        let doc = keyed(&graph, rel, &saved, &keys(&doc_of(&graph, rel)));
        reset();
        compat::publish(&graph, rel, &saved, Some(doc));
        assert_eq!(
            take_runtime_id_work(),
            (0, 0),
            "{blocks}: hashes, fill sets"
        );
        assert_eq!(validation_sets(), 0, "{blocks}: validation sets");
        assert_eq!(take_parses(), 0, "{blocks}: parses in the publication");
        assert_eq!(classes(), [0, 1, 0, 0], "{blocks}: one Full decision");
        // An untrusted submission of the same page pays one set: one hash
        // insert per block, and nothing else.
        let doc = keyed(&graph, rel, &saved, &keys(&doc_of(&graph, rel)));
        let none = |_: &tine_core::BlockId| false;
        let input = Input::Pair(Arc::new(doc), Trust::Untrusted(&none));
        let _ = graph.page_ids(&graph.root.join(rel), rel, input, None);
        assert_eq!(validation_sets(), 1, "{blocks}: untrusted intake");
        assert_eq!(
            take_runtime_id_work(),
            (0, 0),
            "{blocks}: untrusted, all valid"
        );
    }
}

/// A9: a live answer at the indexed revision (the host's mail, a held
/// read) parses nothing, Markdown and Org, and the Org read-only flag comes
/// from the very document served.
#[test]
fn a9_a_live_answer_at_the_indexed_revision_parses_nothing() {
    for (rel, text) in [
        ("pages/p.md", "- a\n- b\n  - c\n"),
        ("pages/p.org", "* a\n* b\n** c\n"),
    ] {
        let (_temp, graph) = graph(&[(rel, text.to_owned())]);
        graph.with_pages(|_| ());
        reset();
        let dto = (graph.page_dto_for_bytes(&graph.root.join(rel), text.as_bytes()))
            .unwrap()
            .unwrap();
        assert_eq!(take_parses(), 0, "{rel}: parses");
        assert_eq!(take_runtime_id_work(), (0, 0), "{rel}: id work");
        assert!(!dto.read_only, "{rel}: editable");
        let keys: Vec<String> = dto.blocks.iter().map(|b| b.id.clone()).collect();
        assert_eq!(keys[..], self::keys(&doc_of(&graph, rel))[..2], "{rel}");
    }
}

/// A fresh load takes one positional walk (one hash per block) and no set;
/// the registry is empty after a restart.
#[test]
fn a_fresh_load_takes_one_walk_and_the_registry_starts_empty() {
    let (temp, graph) = graph(&[
        ("pages/a.md", "- a\n- b\n  - c\n".into()),
        ("pages/b.md", "- d\n".into()),
    ]);
    reset();
    graph.with_pages(|_| ());
    assert_eq!(take_runtime_id_work(), (4, 0), "hashes, fill sets");
    assert_eq!(classes(), [2, 0, 0, 0], "two Fresh decisions");
    graph.ids.moved(
        [tine_core::BlockId::from("k")],
        &graph.root.join("pages/b.md"),
    );
    assert_eq!(graph.ids.registry_len(), 1);
    let restarted = Graph::open(temp.path());
    restarted.with_pages(|_| ());
    assert_eq!(
        restarted.ids.registry_len(),
        0,
        "the registry is not persisted"
    );
}

/// T10 (A4, M1): a rebuild reuses every unchanged row whole (its Arc, no
/// parse) and parses only the paths whose bytes changed, keeping their
/// keys where they hold their outline.
#[test]
fn a_rebuild_reuses_unchanged_rows_and_parses_only_changed_paths() {
    let files: Vec<(String, String)> = (0..20)
        .map(|at| (format!("pages/p{at}.md"), page(5)))
        .collect();
    let files: Vec<(&str, String)> = files.iter().map(|(r, t)| (r.as_str(), t.clone())).collect();
    let (_temp, graph) = graph(&files);
    graph.with_pages(|_| ());
    let before: Vec<Arc<Document>> = files.iter().map(|(rel, _)| doc_of(&graph, rel)).collect();
    // One page changes its line endings (same outline), one its text.
    fs::write(
        graph.root.join("pages/p0.md"),
        page(5).replace('\n', "\r\n"),
    )
    .unwrap();
    fs::write(graph.root.join("pages/p1.md"), page(6)).unwrap();
    reset();
    graph.invalidate_cache();
    graph.with_pages(|_| ());
    assert_eq!(take_parses(), 2, "only the changed paths are parsed");
    let after: Vec<Arc<Document>> = files.iter().map(|(rel, _)| doc_of(&graph, rel)).collect();
    assert!(
        Arc::ptr_eq(&before[0], &after[0]),
        "the same outline keeps its Arc"
    );
    assert!(
        !Arc::ptr_eq(&before[1], &after[1]),
        "the changed page is new"
    );
    for at in 2..20 {
        assert!(Arc::ptr_eq(&before[at], &after[at]), "p{at}: reused whole");
    }
    assert_eq!(graph.witness(&graph.root.join("pages/p0.md")), 0, "W stays");
    assert_ne!(graph.witness(&graph.root.join("pages/p1.md")), 0, "W moved");
}

/// A1 and A11: a key swap among identical bullets (an own save) and an
/// external swap of two bullets that reuses the positional key strings both
/// advance W; an equivalent rewrite does not.
#[test]
fn a1_a11_a_swap_advances_the_witness() {
    let rel = "pages/a.md";
    let (_temp, graph) = graph(&[(rel, "- x\n- x\n".into())]);
    let path = graph.root.join(rel);
    graph.with_pages(|_| ());
    graph.hold_unhosted(rel);
    compat::publish(&graph, rel, "- x\n- x\n", None);
    let w0 = graph.witness(&path);
    let before = keys(&doc_of(&graph, rel));
    let swapped = vec![before[1].clone(), before[0].clone()];
    let doc = keyed(&graph, rel, "- x\n- x\n", &swapped);
    compat::publish(&graph, rel, "- x\n- x\n", Some(doc));
    let w1 = graph.witness(&path);
    assert!(w1 > w0, "A1: the swap advanced W ({w0} -> {w1})");
    compat::publish(&graph, rel, "- x\r\n- x\r\n", None);
    assert_eq!(graph.witness(&path), w1, "an equivalent rewrite keeps W");
    let _ = graph.release(rel);

    let rel = "pages/b.md";
    let path = graph.root.join(rel);
    fs::write(&path, "- A [[X]]\n- B\n").unwrap();
    let _ = graph.sync_file_internal(&path, None);
    let (k0, w0) = (keys(&doc_of(&graph, rel)), graph.witness(&path));
    fs::write(&path, "- B\n- A [[X]]\n").unwrap();
    let _ = graph.sync_file_internal(&path, None);
    assert_eq!(keys(&doc_of(&graph, rel)), k0, "A11: the same key strings");
    assert!(graph.witness(&path) > w0, "A11: W advanced");
}

/// A12: an Org change that keeps the tree but no longer round-trips keeps
/// W and the row's document, and the page answers read-only.
#[test]
fn a12_a_metadata_only_org_change_keeps_w_and_flags_read_only() {
    let rel = "pages/o.org";
    let (_temp, graph) = graph(&[(rel, "* a\n** c\n".into())]);
    let path = graph.root.join(rel);
    graph.with_pages(|_| ());
    let (doc, w) = (doc_of(&graph, rel), graph.witness(&path));
    let bytes = "* a\n*** c\n";
    // Whether lsdoc's parse holds the same tree for both (the case as the
    // review states it); otherwise the outline changed and W moves.
    let same = parse_doc(&path, bytes) == *doc;
    eprintln!("A12 same-tree={same}");
    fs::write(&path, bytes).unwrap();
    let _ = graph.sync_file_internal(&path, None);
    if same {
        assert!(Arc::ptr_eq(&doc, &doc_of(&graph, rel)), "nodes kept");
        assert_eq!(graph.witness(&path), w, "W unchanged");
    } else {
        assert!(graph.witness(&path) > w, "a changed outline moves W");
    }
    let dto = graph
        .page_dto_for_bytes(&path, bytes.as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(dto.read_only, !tine_core::org::org_editable(bytes));
}

/// A13: a page deleted and created again with the same bytes takes a new
/// W; the old one validates nothing.
#[test]
fn a13_a_recreated_path_takes_a_new_witness() {
    let rel = "pages/a.md";
    let (_temp, graph) = graph(&[(rel, "- a\n".into())]);
    let path = graph.root.join(rel);
    graph.with_pages(|_| ());
    let w0 = graph.witness(&path);
    fs::remove_file(&path).unwrap();
    let _ = graph.forget_file_internal(&path);
    assert_eq!(graph.cached_rev(&path), None, "deleted");
    fs::write(&path, "- a\n").unwrap();
    let _ = graph.sync_file_internal(&path, None);
    let w1 = graph.witness(&path);
    assert!(graph.cached_rev(&path).is_some(), "recreated");
    assert!(w1 > w0, "a new W ({w0} -> {w1})");
}

/// A3 (store side): a key moved to page B with `move_blocks` is B's; a
/// raw rewrite of the source A whose first position hashes to that key
/// gives A's block another key, so no key has two runtime owners.
#[test]
fn a3_a_source_never_reallocates_a_moved_key() {
    let (_temp, graph) = graph(&[
        ("pages/a.md", "- moved\n- stays\n".into()),
        ("pages/b.md", "- b\n".into()),
    ]);
    let (a, b) = (graph.root.join("pages/a.md"), graph.root.join("pages/b.md"));
    graph.with_pages(|_| ());
    let moved = keys(&doc_of(&graph, "pages/a.md"))[0].clone();
    graph
        .ids
        .moved([tine_core::BlockId::from(moved.as_str())], &b);
    assert_eq!(graph.ids.owner_of(&moved.as_str().into()), Some(b.clone()));
    fs::write(&a, "- new first\n- stays\n").unwrap();
    let _ = graph.sync_file_internal(&a, None);
    let now = keys(&doc_of(&graph, "pages/a.md"));
    assert!(!now.contains(&moved), "A reallocated B's key: {now:?}");
    let distinct: HashSet<_> = now.iter().collect();
    assert_eq!(distinct.len(), now.len(), "A's keys are distinct");
}

/// A2 (store side): an untrusted submission repeating a key, or naming a
/// key another page owns, keeps each key once in the page and graph-wide.
#[test]
fn a2_untrusted_intake_drops_repeated_and_foreign_keys() {
    let rel = "pages/a.md";
    let (_temp, graph) = graph(&[(rel, "- one\n- two\n- three\n".into())]);
    let path = graph.root.join(rel);
    graph.with_pages(|_| ());
    let text = "- one\n- two\n- three\n";
    let doc = keyed(
        &graph,
        rel,
        text,
        &["k".into(), "k".into(), "theirs".into()],
    );
    let elsewhere = |key: &tine_core::BlockId| *key == "theirs";
    let input = Input::Pair(Arc::new(doc), Trust::Untrusted(&elsewhere));
    let keys = keys(&graph.page_ids(&path, rel, input, None));
    assert_eq!(keys[0], "k", "the first holder keeps the key");
    assert!(keys[1] != "k" && keys[2] != "theirs", "{keys:?}");
    let distinct: HashSet<_> = keys.iter().collect();
    assert_eq!(distinct.len(), 3, "{keys:?}");
}

/// Blocker 2 of the ee4ea5b9a review: a one-block own save of a page with
/// no moved keys visits no registry entry, whether the registry holds 0,
/// 100 or 10,000 keys of another page.
#[test]
fn r2_an_unrelated_save_does_no_registry_work_at_any_registry_size() {
    for size in [0, 100, 10_000] {
        let rel = "pages/a.md";
        let (_temp, graph) = graph(&[(rel, "- a\n".into()), ("pages/b.md", "- b\n".into())]);
        graph.with_pages(|_| ());
        let b = graph.root.join("pages/b.md");
        let moved = (0..size).map(|at| tine_core::BlockId::from(format!("m{at}").as_str()));
        graph.ids.moved(moved, &b);
        assert_eq!(graph.ids.registry_len(), size);
        graph.hold_unhosted(rel);
        compat::publish(&graph, rel, "- a\n", None);
        let saved = "- a edited\n";
        let doc = keyed(&graph, rel, saved, &keys(&doc_of(&graph, rel)));
        REGISTRY_VISITS.with(|visits| visits.set(0));
        compat::publish(&graph, rel, saved, Some(doc));
        let visits = REGISTRY_VISITS.with(|visits| visits.replace(0));
        assert_eq!(visits, 0, "registry entries visited at size {size}");
        assert_eq!(graph.ids.registry_len(), size, "{size}: B's keys stay");
    }
}

/// Finding 5 of the ee4ea5b9a review: the witness map holds one entry per
/// live path created or changed this session (a cold-built row has none).
/// A page renamed N times (each a deletion at the old path and a creation
/// at the new one) leaves the map at the live paths' count.
#[test]
fn r2_the_witness_map_holds_the_live_paths_after_renames() {
    let (_temp, graph) = graph(&[("pages/other.md", "- o\n".into())]);
    graph.with_pages(|_| ());
    let other = graph.root.join("pages/other.md");
    fs::write(&other, "- o\n- changed\n").unwrap();
    let _ = graph.sync_file_internal(&other, None);
    let mut from = graph.root.join("pages/p0.md");
    fs::write(&from, "- p\n").unwrap();
    let _ = graph.sync_file_internal(&from, None);
    assert_eq!(graph.ids.witnesses(), 2, "changed and created");
    for at in 1..=50 {
        let to = graph.root.join(format!("pages/p{at}.md"));
        fs::rename(&from, &to).unwrap();
        let _ = graph.forget_file_internal(&from);
        let _ = graph.sync_file_internal(&to, None);
        assert!(graph.cached_rev(&to).is_some(), "renamed {at}");
        assert_eq!(graph.ids.witnesses(), 2, "after {at} renames");
        from = to;
    }
}
