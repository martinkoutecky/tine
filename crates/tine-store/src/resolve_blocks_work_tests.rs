//! I-13 (D-20261022-GX, related to GH #746): resolving `((uuid))` references
//! touches only the pages that hold the requested blocks, never every page,
//! and answers exactly what a scan of every page in page order would.
use crate::model::ReadSnapshot;
use crate::store::{Depth, OpenOptions, PageId, SaveBase, SaveOutcome, Store};
use tine_core::doc::DocBlock;

/// Enough pages that any per-call whole-graph pass dwarfs the allowed work.
const BALLAST: usize = 1_500;
const TARGET: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const DUPLICATE: &str = "aaaaaaaa-0000-4000-8000-000000000002";
const EXTERNAL: &str = "aaaaaaaa-0000-4000-8000-000000000003";
const DANGLING: &str = "aaaaaaaa-0000-4000-8000-0000000000ff";

fn ballast_id(i: usize) -> String {
    format!("bbbbbbbb-0000-4000-8000-{i:012}")
}

fn graph(tag: &str) -> tempfile::TempDir {
    let root = tempfile::Builder::new()
        .prefix(&format!("tine-resolve-work-{tag}-"))
        .tempdir()
        .unwrap();
    let pages = root.path().join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::create_dir_all(root.path().join("journals")).unwrap();
    for i in 0..BALLAST {
        std::fs::write(
            pages.join(format!("P{i}.md")),
            format!("- body {i}\n  id:: {}\n- plain {i}\n", ballast_id(i)),
        )
        .unwrap();
    }
    let write = |name: &str, text: String| std::fs::write(pages.join(name), text).unwrap();
    write("Target.md", format!("- target\n  id:: {TARGET}\n- other\n"));
    write("DupA.md", format!("- dup on A\n  id:: {DUPLICATE}\n"));
    write("DupB.md", format!("- dup on B\n  id:: {DUPLICATE}\n"));
    write("Other.md", "- unrelated\n".into());
    write(
        "Refs.md",
        format!("- see (({TARGET})) (({DUPLICATE})) (({DANGLING}))\n"),
    );
    root
}

fn open(root: &std::path::Path) -> Store {
    Store::open(root, OpenOptions::default()).unwrap().0
}

fn first_line(raw: &str) -> String {
    raw.lines().next().unwrap_or("").to_owned()
}

/// Every runtime uuid and persisted `id::` in `blocks`.
fn held_ids(blocks: &[DocBlock], out: &mut std::collections::BTreeSet<String>) {
    for block in blocks {
        out.insert(block.uuid.clone());
        if let Some(id) = block.property("id") {
            out.insert(id);
        }
        held_ids(&block.children, out);
    }
}

/// The reference answer: the first block, in page order then walk order,
/// whose runtime uuid or persisted `id::` is `id`.
fn scan(graph: &ReadSnapshot, id: &str) -> Option<(String, String)> {
    fn find<'a>(blocks: &'a [DocBlock], id: &str) -> Option<&'a DocBlock> {
        blocks.iter().find_map(|block| {
            (block.uuid == id || block.property("id").as_deref() == Some(id))
                .then_some(block)
                .or_else(|| find(&block.children, id))
        })
    }
    graph.with_pages(|pages| {
        pages.iter().find_map(|(entry, doc)| {
            find(&doc.roots, id).map(|block| (entry.name.clone(), first_line(block.raw())))
        })
    })
}

/// Resolve `ids` on the current generation: assert the answer equals the
/// whole-graph scan and return it with the number of pages the resolve walked.
fn resolve(store: &Store, ids: &[&str]) -> (Vec<Option<(String, String)>>, usize) {
    let view = store.whole_graph().unwrap();
    let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    // Publication already built or carried the exact index; this call only
    // measures resolving the requested ids.
    let before = crate::query::resolve_pages_touched();
    let got: Vec<_> = view
        .blocks(&ids)
        .unwrap()
        .into_iter()
        .map(|group| group.map(|group| (group.page, first_line(&group.blocks[0].raw))))
        .collect();
    let touched = crate::query::resolve_pages_touched() - before;
    let expected: Vec<_> = ids.iter().map(|id| scan(&view.graph, id)).collect();
    assert_eq!(got, expected, "resolve_blocks must answer as a full scan");
    for id in &ids {
        let preview = view.preview_block(id, 10).unwrap();
        assert_eq!(
            preview.map(|preview| preview.group.page),
            scan(&view.graph, id).map(|(page, _)| page),
            "preview_block must answer as a full scan"
        );
    }
    (got, touched)
}

fn save(store: &Store, rel: &str, edit: impl FnOnce(&mut tine_core::PageDto)) {
    let id = PageId::from(rel.to_owned());
    let mut read = store.page(&id).unwrap();
    edit(&mut read.doc);
    assert!(matches!(
        store.save(
            crate::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &read.doc
        ),
        SaveOutcome::Saved(_)
    ));
}

#[test]
fn dangling_and_hinted_refs_walk_only_the_owning_pages() {
    let root = graph("dangling");
    let store = open(root.path());
    let (got, touched) = resolve(&store, &[DANGLING]);
    assert_eq!(got, vec![None]);
    assert_eq!(
        touched, 0,
        "a dangling ref is answered without walking pages"
    );

    let (got, touched) = resolve(&store, &[TARGET, TARGET, DANGLING]);
    assert_eq!(got[0].as_ref().unwrap().0, "Target");
    assert_eq!(touched, 1, "hinted refs walk only their page");

    let many: Vec<String> = (0..40).map(ballast_id).collect();
    let mut ids: Vec<&str> = many.iter().map(String::as_str).collect();
    ids.push(DANGLING);
    let (_, touched) = resolve(&store, &ids);
    assert_eq!(touched, 40, "one walk per owning page");
    store.close();
}

#[test]
fn duplicate_ids_answer_the_first_page_in_page_order() {
    let root = graph("duplicate");
    let store = open(root.path());
    let (got, touched) = resolve(&store, &[DUPLICATE]);
    assert!(got[0].is_some());
    assert_eq!(touched, 1, "the first owner holds the answer");
    store.close();
}

#[test]
fn a_moved_block_resolves_on_its_new_page() {
    let root = graph("moved");
    let store = open(root.path());
    resolve(&store, &[TARGET]);
    // Tine's own move: the copy lands first, then the original goes.
    save(&store, "pages/Other.md", |page| {
        let mut moved = page.blocks[0].clone();
        moved.id = TARGET.into();
        moved.raw = format!("moved here\nid:: {TARGET}");
        page.blocks.push(moved);
    });
    let (got, _) = resolve(&store, &[TARGET]);
    assert_eq!(got[0].as_ref().unwrap().0.as_str(), {
        // Both pages hold it now; the answer is the first in page order.
        let view = store.whole_graph().unwrap();
        scan(&view.graph, TARGET).unwrap().0
    });
    save(&store, "pages/Target.md", |page| {
        page.blocks.remove(0);
    });
    let (got, touched) = resolve(&store, &[TARGET]);
    assert_eq!(
        got,
        vec![Some(("Other".into(), "moved here".into()))],
        "the block resolves on its new page"
    );
    assert_eq!(touched, 1, "a moved block walks only its new page");

    // An external editor moves it back.
    std::fs::write(
        root.path().join("pages/Other.md"),
        "- unrelated\n".to_string(),
    )
    .unwrap();
    std::fs::write(
        root.path().join("pages/Target.md"),
        format!("- back\n  id:: {TARGET}\n"),
    )
    .unwrap();
    store.refresh(Depth::Stamps).unwrap();
    let (got, touched) = resolve(&store, &[TARGET]);
    assert_eq!(got, vec![Some(("Target".into(), "back".into()))]);
    assert_eq!(touched, 1);
    store.close();
}

#[test]
fn a_deleted_block_is_dangling_without_a_scan() {
    let root = graph("deleted");
    let store = open(root.path());
    resolve(&store, &[TARGET]);
    save(&store, "pages/Target.md", |page| {
        page.blocks.remove(0);
    });
    let (got, touched) = resolve(&store, &[TARGET]);
    assert_eq!(got, vec![None]);
    assert!(
        touched <= 1,
        "a removed block is not searched for: {touched}"
    );
    store.close();
}

#[test]
fn an_external_edit_and_a_rename_keep_answers() {
    let root = graph("external");
    let store = open(root.path());
    resolve(&store, &[TARGET]);
    std::fs::write(
        root.path().join("pages/Ext.md"),
        format!("- added outside\n  id:: {EXTERNAL}\n"),
    )
    .unwrap();
    store.refresh(Depth::Stamps).unwrap();
    let (got, touched) = resolve(&store, &[EXTERNAL]);
    assert_eq!(got, vec![Some(("Ext".into(), "added outside".into()))]);
    assert_eq!(touched, 1);

    std::fs::rename(
        root.path().join("pages/Target.md"),
        root.path().join("pages/Renamed.md"),
    )
    .unwrap();
    store.refresh(Depth::Stamps).unwrap();
    let (got, touched) = resolve(&store, &[TARGET]);
    assert_eq!(got, vec![Some(("Renamed".into(), "target".into()))]);
    assert_eq!(touched, 1);

    // A title:: rename keeps the file and changes the page name.
    std::fs::write(
        root.path().join("pages/Renamed.md"),
        format!("title:: Titled\n\n- target\n  id:: {TARGET}\n"),
    )
    .unwrap();
    store.refresh(Depth::Stamps).unwrap();
    let (got, touched) = resolve(&store, &[TARGET]);
    assert_eq!(got, vec![Some(("Titled".into(), "target".into()))]);
    assert_eq!(touched, 1);
    store.close();
}

#[test]
fn runtime_ids_resolve_after_edits() {
    let root = graph("runtime");
    let store = open(root.path());
    save(&store, "pages/Other.md", |page| {
        let mut added = page.blocks[0].clone();
        added.id = "cccccccc-0000-4000-8000-000000000001".into();
        added.raw = "typed in the editor".into();
        page.blocks.insert(0, added);
    });
    let view = store.whole_graph().unwrap();
    let ids: Vec<String> = view.graph.with_pages(|pages| {
        pages
            .iter()
            .filter(|(entry, _)| entry.name == "Other" || entry.name == "Target")
            .flat_map(|(_, doc)| doc.roots.iter().map(|block| block.uuid.clone()))
            .collect()
    });
    drop(view);
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (got, touched) = resolve(&store, &ids);
    assert!(got.iter().all(Option::is_some));
    assert_eq!(touched, 2);
    store.close();
}

/// Differential: after every publication (Tine saves and external edits that
/// copy, move, duplicate and delete `id::` blocks, rename and delete pages),
/// the carried block index lists exactly the slots a fresh full build lists,
/// for every id any generation held.
#[test]
fn the_carried_block_index_equals_a_fresh_build_under_random_edits() {
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }
    const NAMES: &[&str] = &["F0", "F1", "F2", "F3", "F4"];
    let shared = |n: usize| format!("dddddddd-0000-4000-8000-{n:012}");
    for seed in [1u64, 7, 0xC0FFEE] {
        let mut r = Rng(seed);
        let root = tempfile::tempdir().unwrap();
        let pages = root.path().join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        for name in NAMES {
            std::fs::write(pages.join(format!("{name}.md")), "- start\n").unwrap();
        }
        let store = open(root.path());
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        for step in 0..120 {
            let name = NAMES[r.below(NAMES.len())];
            let block = |r: &mut Rng| match r.below(3) {
                0 => format!("- shared\n  id:: {}\n", shared(r.below(4))),
                1 => "- plain\n".to_owned(),
                _ => format!("- nested\n  - child\n    id:: {}\n", shared(r.below(4))),
            };
            match r.below(5) {
                0 | 1 => {
                    let rel = format!("pages/{name}.md");
                    if store.page(&PageId::from(rel.clone())).is_ok() {
                        let raw = block(&mut r);
                        let id = (r.below(2) == 0).then(|| shared(r.below(4)));
                        let id_ = PageId::from(rel.clone());
                        let mut read = store.page(&id_).unwrap();
                        {
                            let page = &mut read.doc;
                            if !page.blocks.is_empty() && r.below(2) == 0 {
                                let at = r.below(page.blocks.len());
                                page.blocks.remove(at);
                            }
                            let mut added = tine_core::BlockDto {
                                raw: raw.trim_start_matches("- ").lines().next().unwrap().into(),
                                ..Default::default()
                            };
                            if let Some(id) = id {
                                added.raw = format!("{}\nid:: {id}", added.raw);
                            }
                            page.blocks.push(added);
                        }
                        let outcome = store.save(
                            crate::EditKind::ReplacePage,
                            &id_,
                            SaveBase::Existing(read.rev),
                            &read.doc,
                        );
                        assert!(
                            matches!(outcome, SaveOutcome::Saved(_) | SaveOutcome::Unchanged(_)),
                            "seed {seed} step {step}: {outcome:?}"
                        );
                    }
                }
                2 => {
                    let text: String = (0..1 + r.below(3)).map(|_| block(&mut r)).collect();
                    std::fs::write(pages.join(format!("{name}.md")), text).unwrap();
                }
                3 => {
                    let _ = std::fs::remove_file(pages.join(format!("{name}.md")));
                }
                _ => {
                    let to = NAMES[r.below(NAMES.len())];
                    let _ = std::fs::rename(
                        pages.join(format!("{name}.md")),
                        pages.join(format!("{to}.md")),
                    );
                }
            }
            store.refresh(Depth::Stamps).unwrap();
            let view = store.whole_graph().unwrap();
            view.graph.with_pages(|pages| {
                for (_, doc) in pages.iter() {
                    held_ids(&doc.roots, &mut seen);
                }
            });
            let fresh = ReadSnapshot::capture(
                &store.graph,
                (*view.config.config).clone(),
                std::sync::Arc::clone(&view.list),
                None,
                &[],
            );
            for id in &seen {
                assert_eq!(
                    view.graph.block_owner_slots(id),
                    fresh.block_owner_slots(id),
                    "seed {seed} step {step}: block index diverged for {id}"
                );
            }
            drop(view);
            let ids: Vec<&str> = seen.iter().map(String::as_str).collect();
            resolve(&store, &ids);
        }
        store.close();
    }
}

/// Acceptance on a real graph (run by hand on a COPY):
/// `TINE_RESOLVE_GRAPH=<copy> TINE_RESOLVE_OUT=<file> cargo test --release -p tine-store
/// --lib acceptance_resolve_every_reference -- --ignored --nocapture`. Resolves every
/// `((uuid))` the graph references and every block id it holds, checks each
/// answer against one full scan in page order, and writes the referenced
/// ids' `id<TAB>page<TAB>first line` rows.
#[test]
#[ignore]
fn acceptance_resolve_every_reference_in_a_graph() {
    let Ok(root) = std::env::var("TINE_RESOLVE_GRAPH") else {
        return;
    };
    let store = open(std::path::Path::new(&root));
    let view = store.whole_graph().unwrap();
    // The full-scan answer for every held id: first page in page order, then
    // walk order (one pass).
    let started = std::time::Instant::now();
    let (refs, oracle) = view.graph.with_pages(|pages| {
        fn visit(
            blocks: &[DocBlock],
            page: &str,
            oracle: &mut std::collections::HashMap<String, (String, String)>,
        ) {
            for block in blocks {
                let answer = || (page.to_owned(), first_line(block.raw()));
                oracle.entry(block.uuid.clone()).or_insert_with(answer);
                if let Some(id) = block.property("id") {
                    oracle.entry(id).or_insert_with(answer);
                }
                visit(&block.children, page, oracle);
            }
        }
        let mut refs = std::collections::BTreeSet::new();
        let mut oracle = std::collections::HashMap::new();
        for (entry, doc) in pages.iter() {
            refs.extend(tine_core::page_properties::document_block_ref_counts(doc).into_keys());
            visit(&doc.roots, &entry.name, &mut oracle);
        }
        (refs, oracle)
    });
    let scan_time = started.elapsed();
    let started = std::time::Instant::now();
    view.graph.block_owner_page("");
    let lookup_time = started.elapsed();
    let dangling: Vec<String> = refs
        .iter()
        .filter(|id| !oracle.contains_key(*id))
        .cloned()
        .collect();
    let time = |ids: &[String]| {
        let before = crate::query::resolve_pages_touched();
        let started = std::time::Instant::now();
        let out: Vec<_> = ids
            .chunks(crate::store::RESULT_BRIDGE_MAX_ROWS)
            .flat_map(|chunk| view.blocks(chunk).unwrap())
            .collect();
        (
            out,
            started.elapsed(),
            crate::query::resolve_pages_touched() - before,
        )
    };
    let (_, dangling_time, dangling_walks) = time(&dangling);
    let refs: Vec<String> = refs.into_iter().collect();
    let (ref_out, ref_time, ref_walks) = time(&refs);
    let held: Vec<String> = oracle.keys().cloned().collect();
    let (held_out, held_time, held_walks) = time(&held);
    let answer = |group: Option<tine_core::RefGroup>| {
        group.map(|group| (group.page, first_line(&group.blocks[0].raw)))
    };
    let ref_answers: Vec<_> = ref_out.into_iter().map(answer).collect();
    let mismatches = refs
        .iter()
        .zip(&ref_answers)
        .filter(|(id, got)| oracle.get(*id) != got.as_ref())
        .count()
        + held
            .iter()
            .zip(held_out.into_iter().map(answer))
            .filter(|(id, got)| oracle.get(*id) != got.as_ref())
            .count();
    let pages = view.graph.with_pages(|pages| pages.len());
    eprintln!(
        "pages {pages}; held ids {} (index lookup {lookup_time:?}; full scan {scan_time:?}); \
         refs {} in {ref_time:?} ({ref_walks} page walks); dangling {} in {dangling_time:?} \
         ({dangling_walks} walks); all held ids in {held_time:?} ({held_walks} walks); \
         mismatches {mismatches}",
        held.len(),
        refs.len(),
        dangling.len(),
    );
    if let Ok(path) = std::env::var("TINE_RESOLVE_OUT") {
        let rows: String = refs
            .iter()
            .zip(&ref_answers)
            .map(|(id, answer)| match answer {
                Some((page, line)) => format!("{id}\t{page}\t{line}\n"),
                None => format!("{id}\t-\t-\n"),
            })
            .collect();
        std::fs::write(path, rows).unwrap();
    }
    assert_eq!(mismatches, 0);
    store.close();
}
