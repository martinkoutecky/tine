//! I-25: saving one page while readers retain old views cannot copy the graph.
use std::{fs, sync::Mutex};
static CASE: Mutex<()> = Mutex::new(());
use tine_store::{cost_counters, EditKind, OpenOptions, PageId, SaveBase, SaveOutcome, Store};

#[test]
fn retained_views_bound_content_and_structural_publication_work() {
    let _case = CASE.lock().unwrap();
    for pages in [32, 256] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        for i in 0..pages {
            fs::write(
                dir.path().join(format!("pages/P{i}.md")),
                format!("- ((00000000-0000-0000-0000-{i:012}))\n"),
            )
            .unwrap();
        }
        let store = Store::open(dir.path(), OpenOptions::default()).unwrap().0;
        let held = store.whole_graph().unwrap();
        let id = PageId::from("pages/P0.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "edited ((00000000-0000-0000-0000-999999999999))".into();
        cost_counters::reset();
        assert!(matches!(
            store.save(
                EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        let cost = cost_counters::snapshot();
        assert!(cost.cache_page_copies <= 1, "I-25: clone only the changed page, never graph slots; exemplar model/persistent.rs: {cost:?}");
        assert_eq!(
            held.block_ref_counts()
                .get("00000000-0000-0000-0000-000000000000"),
            Some(&1)
        );
        assert!(!held
            .block_ref_counts()
            .contains_key("00000000-0000-0000-0000-999999999999"));
        assert_eq!(
            store
                .whole_graph()
                .unwrap()
                .block_ref_counts()
                .get("00000000-0000-0000-0000-999999999999"),
            Some(&1)
        );
        let created = PageId::from("pages/Added.md");
        doc.name = "Added".into();
        cost_counters::reset();
        assert!(matches!(
            store.save(EditKind::ReplacePage, &created, SaveBase::CreateNew, &doc),
            SaveOutcome::Saved(_)
        ));
        let cost = cost_counters::snapshot();
        assert!(
            cost.shared_tree_node_copies <= 256,
            "I-25: bounded tree paths, not graph copies: {cost:?}"
        );
        assert_eq!(cost.snapshot_rebuilds, 0, "I-25: structural publication patches affected paths, never rebuilds signatures; exemplar model/persistent.rs: {cost:?}");
        assert_eq!(held.corpus().pages.len(), pages);
        assert_eq!(store.whole_graph().unwrap().corpus().pages.len(), pages + 1);
        store.close();
    }
}

#[test]
fn save_preparation_parses_old_document_once() {
    let source = include_str!("../src/model.rs");
    let preparation = source
        .split("fn prepare_page_content(")
        .nth(1)
        .unwrap()
        .split("/// Properties that describe the block they sit on")
        .next()
        .unwrap();
    assert!(!preparation.contains("doc::parse"), "I-25/I-12: parse old source once through parse_doc and pass the Document to every validator; exemplar model/layout_retention.rs");
    let layout = include_str!("../src/model/layout_retention.rs");
    assert!(
        !layout.contains("doc::parse(source)"),
        "I-25: layout retention must borrow the caller's parsed old Document"
    );
}

#[test]
fn actual_save_old_source_parse_count() {
    let _case = CASE.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/Page.md"), "- before\n").unwrap();
    let store = Store::open(dir.path(), OpenOptions::default()).unwrap().0;
    let held = store.whole_graph().unwrap();
    let id = PageId::from("pages/Page.md");
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks[0].raw = "after".into();
    cost_counters::reset();
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &doc
        ),
        SaveOutcome::Saved(_)
    ));
    let counts = cost_counters::snapshot();
    assert_eq!(counts.old_source_parses, 1, "I-25/I-12: all save validators borrow one old Document; exemplar model/layout_retention.rs: {counts:?}");
    assert!(
        counts.parses > counts.old_source_parses,
        "count result/publication and formatting-outline parses too"
    );
    drop(held);
    store.close();
}

#[test]
fn snapshot_family_uses_persistent_roots_and_one_name_winner_answerer() {
    let model = include_str!("../src/model.rs");
    assert!(
        !model.contains("Arc<Vec<(PageEntry"),
        "I-25: held snapshots cannot copy all page slots; exemplar model/persistent.rs"
    );
    let capture = model
        .split("pub(crate) fn capture(")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn carry_memos_from")
        .next()
        .unwrap();
    assert!(
        capture.contains("RealPageNames::capture"),
        "I-12: publication and query use query/page_names.rs RealPageNames::capture"
    );
    assert!(
        !capture.contains(".min_by("),
        "I-12/I-25: no whole-page winner scan; exemplar query/page_names.rs RealPageNames"
    );
    let signature = model
        .split("impl SnapshotReferenceCandidateIndex")
        .nth(1)
        .unwrap()
        .split("type BlockOwner")
        .next()
        .unwrap();
    assert!(
        signature.contains("Arc::clone(&pages.positions)"),
        "I-25: structural publication shares stable positions; exemplar model/persistent.rs Pages"
    );
    assert!(
        !signature.contains("old_by_path"),
        "I-25: never rebuild every old signature path on structural changes"
    );
}
