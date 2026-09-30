use std::fs;
use tine_store::{Day, Resolved, Store};

#[test]
fn absent_journal_proposals_share_the_path_rule() {
    for (preferred, ext) in [("Markdown", "md"), ("Org", "org")] {
        for (format, stem) in [("yyyy_MM_dd", "2026_09_30"), ("dd-MM-yyyy", "30-09-2026")] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("logseq")).unwrap();
            fs::write(dir.path().join("logseq/config.edn"), format!(
                "{{:journals-directory \"diary\" :preferred-format \"{preferred}\" :journal/file-name-format \"{format}\"}}"
            )).unwrap();
            let store = Store::open(dir.path(), Default::default()).unwrap().0;
            let from_day = store.journal_id(Day(20260930));
            let view = store.whole_graph().unwrap();
            let Resolved::Absent { id } = view.resolve(stem, true) else {
                panic!("fixture day is absent")
            };
            assert_eq!(id, from_day);
            assert_eq!(id.as_str(), format!("diary/{stem}.{ext}"));
            let Resolved::Absent { id } = view.resolve("not-a-day", true) else {
                panic!("absent invalid name")
            };
            assert_eq!(id.as_str(), format!("diary/not-a-day.{ext}"));
            store.close();
        }
    }
    let source = include_str!("../src/store.rs");
    assert_eq!(source.matches("proposed_journal_id(").count(), 3,
        "I-12: Store::journal_id and WholeGraph::resolve must both use proposed_journal_id; one definition and two clients, exemplar store.rs");
}
