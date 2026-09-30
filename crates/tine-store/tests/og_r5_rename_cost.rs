//! I-13/I-25: rename checks known page identities without rereading headers.
use std::fs;
use tine_store::{cost_counters, Store};

#[test]
fn rename_does_not_read_unrelated_page_headers() {
    for pages in [32, 256] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        for i in 0..pages {
            fs::write(
                dir.path().join(format!("pages/P{i}.md")),
                format!("title:: Name {i}\n\n- body\n"),
            )
            .unwrap();
        }
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        cost_counters::reset();
        tine_graph_features::pages::rename_page_expected(&store, "Name 0", "Renamed", None)
            .unwrap();
        let cost = cost_counters::snapshot();
        eprintln!("R5 rename pages={pages}: {cost:?}");
        assert!(cost.preamble_reads <= 12, "I-13/I-25: rename must answer same-name claims from the published identity index; exemplar store/snapshot.rs: {cost:?}");
        assert!(fs::read_to_string(dir.path().join("pages/Renamed.md"))
            .unwrap()
            .contains("title:: Renamed"));
        store.close();
    }
}
