//! Cost audit 2026-11-02 row 9b (D-10): a CLI block handle (`tine edit <uuid>`,
//! `tine ref`) resolves its page through the block-id index, walking only the
//! pages that can hold the id, never every page; and it answers exactly what
//! the former whole-graph walk answered: persisted `id::` only (a runtime id is
//! not a handle), ASCII case and surrounding whitespace ignored, page order,
//! stopping at the second page (a duplicate id is ambiguous).
use crate::model::ReadSnapshot;
use crate::store::{Depth, OpenOptions, PageId, SaveBase, Store};

/// Enough pages that a per-call whole-graph walk dwarfs the allowed work.
const BALLAST: usize = 1_500;
const TARGET: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const DUPLICATE: &str = "aaaaaaaa-0000-4000-8000-000000000002";
const MIXED: &str = "aaaaaaaa-0000-4000-8000-00000000abcd";
const DANGLING: &str = "aaaaaaaa-0000-4000-8000-0000000000ff";

fn ballast_id(i: usize) -> String {
    format!("bbbbbbbb-0000-4000-8000-{i:012}")
}

fn graph(tag: &str) -> tempfile::TempDir {
    let root = tempfile::Builder::new()
        .prefix(&format!("tine-block-pages-{tag}-"))
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
    // Hand-typed spellings of one id: uppercase on one page, mixed case on
    // another. Case-insensitively they are the same handle, so it is ambiguous.
    write(
        "Upper.md",
        format!("- upper\n  id:: {}\n", MIXED.to_ascii_uppercase()),
    );
    write(
        "Mixed.md",
        "- mixed\n  ID:: AaaaaaaA-0000-4000-8000-00000000AbCd\n".into(),
    );
    write("Other.md", "- unrelated\n".into());
    root
}

fn open(root: &std::path::Path) -> Store {
    Store::open(root, OpenOptions::default()).unwrap().0
}

/// The former answer: walk every page in page order, persisted `id::` only.
fn scan(graph: &ReadSnapshot, uuid: &str) -> Vec<String> {
    graph.with_pages(|pages| {
        let mut found = Vec::new();
        for (entry, doc) in pages.iter() {
            let mut here = false;
            crate::query::walk(&doc.roots, &mut |block| {
                here |= block
                    .property("id")
                    .is_some_and(|id| id.trim().eq_ignore_ascii_case(uuid));
            });
            if here {
                found.push(entry.name.clone());
                if found.len() == 2 {
                    break;
                }
            }
        }
        found
    })
}

/// `block_pages` on the current generation, checked against the full walk,
/// with the number of pages it walked.
fn pages_of(store: &Store, uuid: &str) -> (Vec<String>, usize) {
    let view = store.whole_graph().unwrap();
    let before = crate::query::block_pages_walked();
    let got = crate::query::block_pages(&view.graph, uuid);
    let walked = crate::query::block_pages_walked() - before;
    assert_eq!(
        got,
        scan(&view.graph, uuid),
        "block_pages must answer as the whole-graph walk did ({uuid})"
    );
    (got, walked)
}

#[test]
fn a_block_handle_walks_only_the_pages_that_hold_its_id() {
    let root = graph("handle");
    let store = open(root.path());
    let (got, walked) = pages_of(&store, TARGET);
    assert_eq!(got, vec!["Target".to_string()]);
    assert_eq!(
        walked, 1,
        "one handle, one owning page walked (was every page)"
    );

    let (got, walked) = pages_of(&store, DANGLING);
    assert!(got.is_empty());
    assert_eq!(walked, 0, "an unknown id is answered without walking pages");

    let (got, walked) = pages_of(&store, &ballast_id(700));
    assert_eq!(got, vec!["P700".to_string()]);
    assert_eq!(walked, 1);
    store.close();
}

#[test]
fn duplicate_and_differently_cased_ids_stay_ambiguous() {
    let root = graph("duplicate");
    let store = open(root.path());
    let (got, walked) = pages_of(&store, DUPLICATE);
    assert_eq!(got.len(), 2, "a duplicate id names two pages: {got:?}");
    assert_eq!(walked, 2);

    let (got, walked) = pages_of(&store, MIXED);
    let mut sorted = got.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["Mixed".to_string(), "Upper".to_string()]);
    assert_eq!(walked, 2);
    store.close();
}

#[test]
fn a_runtime_id_is_not_a_handle() {
    let root = graph("runtime");
    let store = open(root.path());
    let view = store.whole_graph().unwrap();
    let runtime = view.graph.with_pages(|pages| {
        pages
            .iter()
            .find(|(entry, _)| entry.name == "Other")
            .map(|(_, doc)| doc.roots[0].uuid.to_string())
            .unwrap()
    });
    assert!(!runtime.is_empty());
    let (got, walked) = pages_of(&store, &runtime);
    assert!(got.is_empty(), "a runtime id never names a handle's page");
    assert!(
        walked <= 1,
        "only the page holding the runtime id is checked"
    );
    store.close();
}

#[test]
fn edits_and_external_changes_move_the_answer() {
    let root = graph("edits");
    let store = std::sync::Arc::new(open(root.path()));
    pages_of(&store, TARGET);
    // Tine's own save copies the id onto a second page: now ambiguous.
    {
        let host_writer = crate::test_host_writer::HostWriter::start(&store);
        let id = PageId::from("pages/Other.md".to_owned());
        let mut read = store.page(&id).unwrap();
        let mut copy = read.doc.blocks[0].clone();
        copy.id = "runtime-copy".into();
        copy.raw = format!("copy\nid:: {TARGET}");
        read.doc.blocks.push(copy);
        host_writer
            .save(
                crate::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &read.doc,
            )
            .unwrap();
    }
    let (got, walked) = pages_of(&store, TARGET);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(walked, 2);

    // An external editor removes the original.
    std::fs::write(root.path().join("pages/Target.md"), "- gone\n").unwrap();
    store.refresh(Depth::Stamps).unwrap();
    let (got, walked) = pages_of(&store, TARGET);
    assert_eq!(got, vec!["Other".to_string()]);
    assert_eq!(walked, 1);
    store.close();
}

/// Cost audit 2026-11-02 row 8: each day of the Journals feed shows its
/// Linked References, which resolve the day's journal. That lookup is keyed
/// by the day's title, never a parse of every journal name (it was a
/// `page_list_arc` walk per day), and finds the same journal page.
#[test]
fn a_journal_days_references_resolve_the_day_without_listing_every_journal() {
    let root = tempfile::Builder::new()
        .prefix("tine-journal-day-lookup-")
        .tempdir()
        .unwrap();
    let journals = root.path().join("journals");
    std::fs::create_dir_all(&journals).unwrap();
    std::fs::create_dir_all(root.path().join("pages")).unwrap();
    for day in 1..=28 {
        for month in 1..=12 {
            std::fs::write(
                journals.join(format!("2001_{month:02}_{day:02}.md")),
                format!("- day {month}/{day}\n"),
            )
            .unwrap();
        }
    }
    std::fs::write(
        root.path().join("pages/Plan.md"),
        "- see [[Mar 5th, 2001]]\n- and [[2001-03-06]]\n",
    )
    .unwrap();
    let store = open(root.path());
    let view = store.whole_graph().unwrap();
    for (name, expected) in [
        ("Mar 5th, 2001", 1),
        ("2001_03_06", 1),
        ("Mar 7th, 2001", 0),
    ] {
        let lists = crate::model::page_list_reads();
        let lookups = crate::query::journal_day_lookups();
        let groups = view.backlinks(name).unwrap();
        assert_eq!(groups.len(), expected, "{name}: {groups:?}");
        assert!(
            crate::query::journal_day_lookups() > lookups,
            "{name} is date-shaped"
        );
        assert_eq!(
            crate::model::page_list_reads() - lists,
            0,
            "{name}: a day's references never list every journal"
        );
    }
    store.close();
}

/// Before/after numbers on a generated graph (`scripts/generate-realistic-graph.mjs`),
/// never a real one: `TINE_PERF_GRAPH=<root> cargo test -p tine-store --lib
/// realistic_graph_costs -- --ignored --nocapture`.
#[test]
#[ignore = "measurement on a generated graph named by TINE_PERF_GRAPH"]
fn realistic_graph_costs() {
    let Ok(root) = std::env::var("TINE_PERF_GRAPH") else {
        return;
    };
    let store = open(std::path::Path::new(&root));
    let view = store.whole_graph().unwrap();
    let pages = view.graph.with_pages(|pages| pages.iter().count());
    // Persisted ids, first and last in page order (the old walk stops early for neither:
    // an id must be checked for a second owner).
    let ids: Vec<String> = view.graph.with_pages(|pages| {
        let mut ids = Vec::new();
        for (_, doc) in pages.iter() {
            crate::query::walk(&doc.roots, &mut |block| {
                if let Some(id) = block.property("id") {
                    ids.push(id.trim().to_owned());
                }
            });
        }
        ids
    });
    let sample: Vec<&String> = ids.iter().step_by((ids.len() / 20).max(1)).collect();
    let t = std::time::Instant::now();
    for id in &sample {
        std::hint::black_box(scan(&view.graph, id));
    }
    let old = t.elapsed() / sample.len() as u32;
    let walked = crate::query::block_pages_walked();
    let t = std::time::Instant::now();
    for id in &sample {
        let got = crate::query::block_pages(&view.graph, id);
        std::hint::black_box(got);
    }
    let new = t.elapsed() / sample.len() as u32;
    let walked = crate::query::block_pages_walked() - walked;
    for id in &sample {
        assert_eq!(
            crate::query::block_pages(&view.graph, id),
            scan(&view.graph, id)
        );
    }
    println!(
        "9b block handle: {pages} pages, {} persisted ids, {} sampled; whole-graph walk {old:?}/handle, index {new:?}/handle, pages walked {walked}",
        ids.len(),
        sample.len()
    );
    // Row 6a: what one page open used to compute for its folded Unlinked
    // References section (cold answer, one fresh store per name).
    drop(view);
    store.close();
    for name in ["tag0", "Area 0/Topic 0"] {
        let store = open(std::path::Path::new(&root));
        let view = store.whole_graph().unwrap();
        let t = std::time::Instant::now();
        match view.unlinked_references(name) {
            Ok(groups) => println!(
                "6a unlinked references for {name:?}: {:?}, {} groups",
                t.elapsed(),
                groups.len()
            ),
            Err(error) => println!("6a unlinked references for {name:?}: {error:?}"),
        }
        drop(view);
        store.close();
    }
}
