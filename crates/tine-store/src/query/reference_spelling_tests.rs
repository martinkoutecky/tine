//! R13 B2 F3: authored spellings reuse resident traversals, without a tree
//! lookup per digit-bearing name. Full reference queries and counts are timed
//! together so a faster iterator alone cannot stand in for path evidence.
use super::*;
use crate::model::referenced_names::name_lookups;
use crate::Store;
use std::fs;

fn fixture(names: usize) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::create_dir(dir.path().join("logseq")).unwrap();
    fs::write(
        dir.path().join("logseq/config.edn"),
        "{:journal/page-title-format \"X-yyyy_d_MM\"}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("pages/Owner.md"),
        "alias:: X-2026_3_10\n\n- owner\n",
    )
    .unwrap();
    let mut notes = "- [[X-2026_03_10]]\n- [[2026-10-03]]\n".to_owned();
    for i in 0..names {
        notes.push_str(&format!("- [[Name{i:05}]]\n"));
    }
    fs::write(dir.path().join("pages/Notes.md"), notes).unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    (dir, store)
}

#[test]
fn authored_spelling_sweep_does_no_per_name_tree_lookup() {
    for size in [8, 32] {
        let (_dir, store) = fixture(size);
        let graph = store.whole_graph().unwrap().test_read_snapshot();
        graph.referenced_name_table();
        let before = name_lookups();
        let names: Vec<_> = explicit_reference_spellings(&graph).collect();
        assert!(names.contains(&"X-2026_03_10"));
        assert!(names.contains(&"2026-10-03"));
        assert_eq!(name_lookups() - before, 0, "size {size}");
        let before = name_lookups();
        let query = run_query(&graph, "[[Owner]]");
        assert_eq!(
            query
                .iter()
                .find(|group| group.page == "Notes")
                .unwrap()
                .blocks
                .len(),
            2
        );
        let groups = backlinks_bounded(&graph, "Owner", 100, 100_000);
        assert_eq!(groups.total, 2);
        let counts = page_counts::referring_block_counts(&graph);
        assert_eq!(counts.get(&(false, "owner".into())), Some(&2));
        assert_eq!(
            name_lookups() - before,
            0,
            "complete query/panel/count path, size {size}"
        );
        store.close();
    }
}

#[test]
#[ignore = "measurement: full query/count path with 10,000 referenced names"]
fn date_reference_query_and_count_timing() {
    let (_dir, store) = fixture(10_000);
    let view = store.whole_graph().unwrap();
    let graph = view.test_read_snapshot();
    graph.referenced_name_table();
    let query = "[[Owner]]";
    let lookups_before = name_lookups();
    let explicit_names = graph.explicit_reference_names().count();
    let authored_names = graph.referenced_name_table().iter().count();
    let started = std::time::Instant::now();
    for _ in 0..10 {
        let groups = crate::query::run_query(&graph, query);
        assert_eq!(
            groups.iter().map(|group| group.blocks.len()).sum::<usize>(),
            4
        );
        assert_eq!(
            groups
                .iter()
                .find(|group| group.page == "Notes")
                .unwrap()
                .blocks
                .len(),
            2
        );
        let counts = page_counts::referring_block_counts(&graph);
        assert_eq!(counts.get(&(false, "owner".into())), Some(&2));
    }
    eprintln!(
        "10 complete reference-query/count pairs, {explicit_names} explicit keys, {authored_names} authored rows: {:?}; name-table lookups: {}",
        started.elapsed(),
        name_lookups() - lookups_before,
    );
    store.close();
}
