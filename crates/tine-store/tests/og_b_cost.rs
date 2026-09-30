//! I-25: saving one page while readers retain old views cannot copy the graph.
use std::fs;
use tine_store::{cost_counters, EditKind, OpenOptions, PageId, SaveBase, SaveOutcome, Store};

#[test]
fn retained_views_bound_content_and_structural_publication_work() {
    for pages in [32, 256] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        for i in 0..pages {
            fs::write(dir.path().join(format!("pages/P{i}.md")), format!("- ((00000000-0000-0000-0000-{i:012}))\n")).unwrap();
        }
        let store = Store::open(dir.path(), OpenOptions::default()).unwrap().0;
        let held = store.whole_graph().unwrap();
        let id = PageId::from("pages/P0.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "edited ((00000000-0000-0000-0000-999999999999))".into();
        cost_counters::reset();
        assert!(matches!(store.save(EditKind::ReplacePage, &id, SaveBase::Existing(read.rev), &doc), SaveOutcome::Saved(_)));
        let cost = cost_counters::snapshot();
        assert!(cost.cache_page_copies <= 1, "I-25: clone only the changed page, never graph slots; exemplar model/persistent.rs: {cost:?}");
        assert_eq!(held.block_ref_counts().get("00000000-0000-0000-0000-000000000000"), Some(&1));
        assert!(!held.block_ref_counts().contains_key("00000000-0000-0000-0000-999999999999"));
        assert_eq!(store.whole_graph().unwrap().block_ref_counts().get("00000000-0000-0000-0000-999999999999"), Some(&1));
        let created = PageId::from("pages/Added.md");
        doc.name = "Added".into();
        cost_counters::reset();
        assert!(matches!(store.save(EditKind::ReplacePage, &created, SaveBase::CreateNew, &doc), SaveOutcome::Saved(_)));
        let cost = cost_counters::snapshot();
        assert_eq!(cost.snapshot_rebuilds, 0, "I-25: structural publication patches affected paths, never rebuilds signatures; exemplar model/persistent.rs: {cost:?}");
        assert_eq!(held.corpus().pages.len(), pages);
        assert_eq!(store.whole_graph().unwrap().corpus().pages.len(), pages + 1);
        store.close();
    }
}

#[test]
fn save_preparation_parses_old_document_once() {
    let source = include_str!("../src/model.rs");
    let preparation = source.split("fn prepare_page_content(").nth(1).unwrap().split("/// Canonical Markdown page-header").next().unwrap();
    assert!(!preparation.contains("doc::parse"), "I-25/I-12: parse old source once through parse_doc and pass the Document to every validator; exemplar model/layout_retention.rs");
    let layout = include_str!("../src/model/layout_retention.rs");
    assert!(!layout.contains("doc::parse(source)"), "I-25: layout retention must borrow the caller's parsed old Document");
}
