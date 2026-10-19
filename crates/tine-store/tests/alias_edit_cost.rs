//! I-25 / D-16 unit cost of an alias edit (D-20261018-M2b/M2c): the alias
//! relation (edge index, owner paths, alias list) is patched by the edited
//! page, never rebuilt by walking the graph. Counts the pages the alias
//! relation examines to serve the first alias-reading query after one edit,
//! on a small and on a 10k-page graph; the bound is the same for both.
use std::fs;
use std::time::Instant;
use tine_store::{
    cost_counters, Area, EditKind, PageId, SaveBase, SaveOutcome, Store, TrashIf, TxOutcome,
};

/// Every 20th page declares an alias, so the relation is neither empty nor
/// a rounding error of the graph.
fn graph(pages: usize) -> (std::path::PathBuf, Store) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-alias-edit-cost-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    for index in 0..pages {
        let body = if index % 20 == 0 {
            format!("alias:: Al{index:05}\n\n- block {index}\n")
        } else {
            format!("- block {index}\n")
        };
        let path = root.join("pages").join(format!("Page{index:05}.md"));
        fs::write(&path, body).unwrap();
        // Outside the racy window (§5.4), as query_unit_cost does.
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

/// What an alias reader needs: the edge index (the backlinks of an alias walk
/// its class) and the owner paths of an alias spelling (resolve).
fn read_aliases(store: &Store, alias: &str) -> usize {
    let view = store.whole_graph().unwrap();
    view.backlinks(alias).unwrap();
    match view.resolve(alias, false) {
        tine_store::Resolved::Alias { owners } => owners.len(),
        _ => 0,
    }
}

fn set_alias(store: &Store, rel: &str, pre_block: Option<&str>) {
    let id = PageId::from(rel.to_string());
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.pre_block = pre_block.map(str::to_owned);
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &doc
        ),
        SaveOutcome::Saved(_)
    ));
}

/// Probes for one step: the edit and the first alias reads after it.
fn step(label: &str, edit: impl FnOnce(), alias_read: impl FnOnce() -> usize) -> (u64, usize) {
    cost_counters::reset();
    let started = Instant::now();
    edit();
    let edited = started.elapsed();
    let found = alias_read();
    let probes = cost_counters::snapshot().alias_page_probes;
    eprintln!(
        "alias edit cost: {label}: alias_page_probes={probes} owners_found={found} \
         edit {edited:?} + first alias reads {:?}",
        started.elapsed() - edited
    );
    (probes, found)
}

fn run(pages: usize) -> Vec<(&'static str, u64)> {
    let (root, store) = graph(pages);
    // The session's warm state: the relation is built once, as a first
    // backlinks or resolve builds it.
    assert_eq!(read_aliases(&store, "al00020"), 1);
    let mut out = Vec::new();
    let mut record = |label: &'static str, (probes, found): (u64, usize), expect: usize| {
        assert_eq!(found, expect, "{label}: the edited alias reads back");
        out.push((label, probes));
    };
    record(
        "add an alias",
        step(
            "add an alias",
            || {
                set_alias(
                    &store,
                    "pages/Page00041.md",
                    Some("alias:: Fresh41, Other41"),
                )
            },
            || read_aliases(&store, "fresh41"),
        ),
        1,
    );
    record(
        "change an alias by case only",
        step(
            "case-only change",
            || {
                set_alias(
                    &store,
                    "pages/Page00041.md",
                    Some("alias:: FRESH41, Other41"),
                )
            },
            || read_aliases(&store, "fresh41"),
        ),
        1,
    );
    record(
        "remove the alias",
        step(
            "remove the alias",
            || set_alias(&store, "pages/Page00041.md", None),
            || read_aliases(&store, "fresh41"),
        ),
        0,
    );
    record(
        "rename an aliased page",
        step(
            "rename an aliased page",
            || {
                let src = store.file_id(Area::Pages, "Page00060.md").unwrap();
                let dst = store.file_id(Area::Pages, "Renamed60.md").unwrap();
                let rev = store.read(&src, None).unwrap().1;
                let mut tx = store.transaction(Some(EditKind::RenamePage));
                tx.move_file(&src, rev, &dst, None);
                assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
            },
            || read_aliases(&store, "al00060"),
        ),
        1,
    );
    record(
        "delete an aliased page",
        step(
            "delete an aliased page",
            || {
                let id = store.file_id(Area::Pages, "Page00080.md").unwrap();
                let rev = store.read(&id, None).unwrap().1;
                let mut tx = store.transaction(Some(EditKind::ReplacePage));
                tx.trash(&id, rev, TrashIf::Any);
                assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
            },
            || read_aliases(&store, "al00080"),
        ),
        0,
    );
    store.close();
    let _ = fs::remove_dir_all(root);
    out
}

#[test]
fn an_alias_edit_costs_the_edit_not_the_graph() {
    let _integration_guard = crate::integration_lock();
    let small = run(200);
    let large = run(10_000);
    for ((label, small), (_, large)) in small.iter().zip(&large) {
        eprintln!("alias edit cost: {label}: 200 pages={small} 10k pages={large}");
        assert!(
            *large <= 8,
            "I-25 / D-16 (M2c): `{label}` examined {large} pages of a 10k-page graph \
             (200 pages: {small}) to serve the next alias read; the alias relation is \
             patched by the edited page. exemplar model/persistent.rs, query/page_names.rs"
        );
    }
}
