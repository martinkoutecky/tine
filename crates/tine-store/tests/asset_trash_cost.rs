//! Trashing one asset (an inline image removed from a block, or one trash
//! button in Settings > unused assets) is asset-sized work under the Store
//! writer, not a walk of every page (cost-model audit 2026-11-02, row 9a;
//! D-10). The orphan answer keeps every reference the full scan counted: an
//! edit that adds a reference refuses the next trash, one that drops the last
//! reference allows it, and a set-aside duplicate-day file still protects.
use crate::{
    cost_counters, Area, EditKind, PageId, Refusal, SaveBase, Store, TrashIf, TxOutcome, Why,
};
use std::fs;

const OLD: std::time::Duration = std::time::Duration::from_secs(3600);

/// `pages` pages; every fifth page shows an image `imgN.png`; `spare` assets
/// nobody references.
fn graph(pages: usize, spare: usize) -> (std::path::PathBuf, std::sync::Arc<Store>) {
    let root = crate::test_support::fresh_dir("tine-asset-trash-cost-");
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let backdate = |path: &std::path::Path| {
        // Outside the racy window (§5.4), as query_unit_cost does.
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - OLD)
            .unwrap();
    };
    for index in 0..pages {
        let body = if index % 5 == 0 {
            format!("- block {index}\n- ![shot](../assets/img{index}.png)\n")
        } else {
            format!("- block {index}\n")
        };
        let path = root.join("pages").join(format!("Page{index:05}.md"));
        fs::write(&path, body).unwrap();
        backdate(&path);
        if index % 5 == 0 {
            let asset = root.join("assets").join(format!("img{index}.png"));
            fs::write(&asset, b"png").unwrap();
            backdate(&asset);
        }
    }
    for index in 0..spare {
        let asset = root.join("assets").join(format!("spare{index}.png"));
        fs::write(&asset, b"png").unwrap();
        backdate(&asset);
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph_reconciled().unwrap();
    (root, store)
}

/// Trash `name` if unreferenced: `Ok(true)` trashed, `Ok(false)` referenced.
fn trash(store: &Store, name: &str) -> Result<bool, String> {
    let id = store.file_id(Area::Assets, name).unwrap();
    let (_, rev) = store.read(&id, None).unwrap();
    let mut tx = store.transaction(None);
    tx.trash(&id, rev, TrashIf::UnreferencedAsset);
    match tx.commit() {
        TxOutcome::Committed { .. } => Ok(true),
        TxOutcome::NotCommitted {
            why: Why::Refused(Refusal::AssetReferenced),
            ..
        } => Ok(false),
        other => Err(format!("{other:?}")),
    }
}

fn append_block(
    store: &Store,
    host_writer: &crate::test_host_writer::HostWriter,
    rel: &str,
    text: &str,
) {
    let id = PageId::from(rel.to_string());
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks.push(tine_core::model::BlockDto {
        raw: text.into(),
        ..Default::default()
    });
    host_writer
        .save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &doc,
        )
        .unwrap();
}

/// Counts of one trash decision.
fn measured(label: &str, pages: usize, decide: impl FnOnce() -> bool, expect: bool) -> (u64, u64) {
    cost_counters::reset();
    let started = std::time::Instant::now();
    assert_eq!(decide(), expect, "{label} on {pages} pages");
    let counts = cost_counters::snapshot();
    eprintln!(
        "asset trash cost: {pages} pages, {label}: {:?}, {} asset-reference documents, \
         {} under the writer ({} us under the writer)",
        started.elapsed(),
        counts.asset_ref_documents,
        counts.orphan_check_documents,
        counts.orphan_check_nanos / 1000,
    );
    (counts.asset_ref_documents, counts.orphan_check_documents)
}

fn run(pages: usize) -> Vec<(&'static str, (u64, u64))> {
    let (root, store) = graph(pages, 4);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
    // The session's first trash may build what later trashes reuse.
    assert_eq!(trash(&store, "spare0.png"), Ok(true));
    let mut out = Vec::new();
    out.push((
        "trash an unreferenced asset",
        measured(
            "unreferenced",
            pages,
            || trash(&store, "spare1.png").unwrap(),
            true,
        ),
    ));
    out.push((
        "refuse a referenced asset",
        measured(
            "referenced",
            pages,
            || trash(&store, "img5.png").unwrap(),
            false,
        ),
    ));
    // An edit that adds the only reference to a spare protects it at once.
    append_block(
        &store,
        &host_writer,
        "pages/Page00001.md",
        "![late](../assets/spare2.png)",
    );
    out.push((
        "refuse after an edit adds a reference",
        measured(
            "added reference",
            pages,
            || trash(&store, "spare2.png").unwrap(),
            false,
        ),
    ));
    // An edit elsewhere that mentions the asset in plain text (the
    // conservative fallback) protects it too.
    append_block(
        &store,
        &host_writer,
        "pages/Page00002.md",
        "see assets/spare3.png",
    );
    out.push((
        "refuse a plaintext mention",
        measured(
            "plaintext mention",
            pages,
            || trash(&store, "spare3.png").unwrap(),
            false,
        ),
    ));
    // Dropping the only reference (the page that held it is rewritten
    // without it) frees the asset.
    let id = PageId::from("pages/Page00005.md".to_string());
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks.truncate(1);
    host_writer
        .save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &doc,
        )
        .unwrap();
    out.push((
        "trash after the last reference goes",
        measured(
            "dropped reference",
            pages,
            || trash(&store, "img5.png").unwrap(),
            true,
        ),
    ));
    drop(host_writer);
    store.close();
    let _ = fs::remove_dir_all(&root);
    out
}

/// D-10: the documents scanned for one trash, and those scanned while the
/// writer is held, do not grow with the graph.
#[test]
fn one_asset_trash_scans_no_graph_under_the_writer() {
    let _integration_guard = crate::test_transactions::exclusive_integration_lock();
    if crate::test_transactions::isolated_cost_case(
        "test_transactions::asset_trash_cost::one_asset_trash_scans_no_graph_under_the_writer",
    ) {
        return;
    }
    let small = run(40);
    let large = run(400);
    for ((label, few), (_, many)) in small.iter().zip(&large) {
        assert_eq!(
            few.1, many.1,
            "{label}: documents scanned under the writer grew with the graph \
             (40 pages: {few:?}, 400 pages: {many:?})"
        );
        assert_eq!(
            few.0, many.0,
            "{label}: documents scanned for one trash grew with the graph \
             (40 pages: {few:?}, 400 pages: {many:?})"
        );
    }
}
