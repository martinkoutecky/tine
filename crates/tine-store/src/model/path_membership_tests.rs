use super::*;
use crate::{Cancel, IrAnswer, IrRequest, Store};
use tine_core::query::{ir::ExecutionContext, QueryDialect};

fn write(root: &std::path::Path, file: &str, raw: &str) {
    let path = root.join(file);
    std::fs::write(&path, raw).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000))
        .unwrap();
}

fn available(view: &crate::WholeGraph, name: &str) -> bool {
    let IrAnswer::Available(answer) = view
        .query_ir(IrRequest::TagTableAvailable { name }, &Cancel::default())
        .unwrap()
        .unwrap()
    else {
        panic!("wrong availability answer")
    };
    answer
}

fn table_nonempty(view: &crate::WholeGraph, name: &str) -> bool {
    let source = format!("(tag {})", serde_json::to_string(name).unwrap());
    let (query, settings) = tine_core::query::parse_query_text(
        &source,
        QueryDialect::Og,
        tine_core::date::JournalDate::today(),
    );
    let IrAnswer::Result(answer) = view
        .query_ir(
            IrRequest::Run {
                query: &query,
                view: &settings,
                context: &ExecutionContext::none(),
            },
            &Cancel::default(),
        )
        .unwrap()
        .unwrap()
    else {
        panic!("wrong table answer")
    };
    answer.total > 0
}

fn check(view: &crate::WholeGraph, names: &[(&str, bool)]) {
    for &(name, expected) in names {
        assert_eq!(available(view, name), expected, "availability {name}");
        assert_eq!(table_nonempty(view, name), expected, "shown table {name}");
    }
}

#[test]
fn tag_availability_reads_no_candidates_or_dtos_at_200_and_2000_pages() {
    for pages in [200, 2000] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        for n in 0..pages {
            write(root.path(), &format!("pages/P{n}.md"), "- ordinary\n");
        }
        write(root.path(), "pages/Source.md", "- #Target\n  - inherited\n");
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let view = store.whole_graph().unwrap();
        crate::query::exec::take_candidate_pages();
        let before = crate::query::result_dto_constructions();
        let start = std::time::Instant::now();
        assert!(available(&view, "Target"));
        assert!(!available(&view, "Absent"));
        let elapsed = start.elapsed();
        assert_eq!(crate::query::exec::take_candidate_pages(), 0);
        assert_eq!(crate::query::result_dto_constructions(), before);
        assert!(elapsed < std::time::Duration::from_millis(10));
        assert!(table_nonempty(&view, "Target"));
        let old_candidates = crate::query::exec::take_candidate_pages();
        let old_dtos = crate::query::result_dto_constructions() - before;
        assert_eq!(old_candidates, pages + 1);
        assert!(old_dtos > 0);
        eprintln!("TAG_AVAIL pages={pages} old_candidates={old_candidates} old_dtos={old_dtos} new_candidates=0 new_dtos=0 pair_ns={}", elapsed.as_nanos());
        store.close();
    }
}

#[test]
fn tag_availability_and_table_agree_on_closures_aliases_dates_and_removal() {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals"] {
        std::fs::create_dir(root.path().join(dir)).unwrap();
    }
    write(root.path(), "pages/Source.md", "tags:: Header\n\nPreamble [[Prose]]\n\n- [[Target/Child]]\n  - inherited\n- [[2026-10-02]]\n- [[2026-10-03]]\n");
    write(root.path(), "pages/Target.md", "alias:: Alternate\n");
    write(root.path(), "pages/Family%2FChild.md", "- body\n");
    write(root.path(), "pages/Empty.md", "");
    write(
        root.path(),
        "pages/Org.org",
        "#+TAGS: OrgHeader\n\n* [[OrgTarget/Child]]\n** inherited\n",
    );
    write(
        root.path(),
        "journals/2026_10_02.md",
        "alias:: DateAlias\n- journal\n",
    );
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let held = store.whole_graph().unwrap();
    check(
        &held,
        &[
            ("Target", true),
            ("Alternate", true),
            ("Source", true),
            ("Header", true),
            ("Prose", true),
            ("Target/Child", true),
            ("Family/Child", true),
            ("Family", false),
            ("Empty", false),
            ("Absent", false),
            ("OrgTarget", true),
            ("OrgHeader", true),
            ("2026-10-02", true),
            ("October 2nd, 2026", true),
            ("October 3rd, 2026", true),
            ("DateAlias", true),
        ],
    );
    write(root.path(), "pages/2026-10-03.md", "");
    store.refresh(crate::Depth::Stamps).unwrap();
    check(
        &store.whole_graph().unwrap(),
        &[("October 3rd, 2026", false), ("2026-10-03", true)],
    );
    std::fs::remove_file(root.path().join("pages/2026-10-03.md")).unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    check(
        &store.whole_graph().unwrap(),
        &[("October 3rd, 2026", true)],
    );
    write(root.path(), "pages/Source.md", "- ordinary\n");
    store.refresh(crate::Depth::Stamps).unwrap();
    let current = store.whole_graph().unwrap();
    check(
        &current,
        &[
            ("Target/Child", false),
            ("Header", false),
            ("Prose", false),
            ("Source", true),
            ("October 3rd, 2026", false),
        ],
    );
    // Target itself retains its header row; a last membership removal needs a
    // target without its own rows, including no alias/property header.
    assert!(available(&held, "Target/Child"));
    write(root.path(), "pages/Empty.md", "- [[New]]\n");
    store.refresh(crate::Depth::Stamps).unwrap();
    check(
        &store.whole_graph().unwrap(),
        &[("New", true), ("Empty", true)],
    );
    write(root.path(), "pages/Empty.md", "");
    store.refresh(crate::Depth::Stamps).unwrap();
    check(
        &store.whole_graph().unwrap(),
        &[("New", false), ("Empty", false)],
    );
    store.close();
}

#[test]
fn tag_availability_patches_only_changed_pages_and_rebuilds_on_checkpoint_load() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("pages")).unwrap();
    write(root.path(), "pages/A.md", "- [[Target]]\n");
    write(root.path(), "pages/B.md", "- ordinary\n");
    let checkpoint = root.path().join("launch.bin");
    let open = || {
        Store::open(
            root.path(),
            crate::OpenOptions {
                launch_checkpoint: Some(checkpoint.clone()),
                ..Default::default()
            },
        )
        .unwrap()
        .0
    };
    let store = open();
    let held = store.whole_graph().unwrap();
    let mut pages = (*held.graph.pages).clone();
    let slot = *pages.positions.get("pages/A.md").unwrap();
    let entry = pages.get(slot).unwrap().0.clone();
    pages.set(
        slot,
        (
            entry,
            Arc::new(tine_core::doc::parse("- changed [[Next]]\n")),
        ),
    );
    // Exercise the actual snapshot capture, not just this index's patch helper.
    PAGES.with(|count| count.set(0));
    let patched = ReadSnapshot::capture_rows(
        Arc::new(pages),
        2,
        Arc::clone(&held.graph.observed_mtimes),
        Arc::clone(&held.graph.completion_name_keys),
        held.graph.config.clone(),
        Arc::clone(&held.graph.list),
        Some(&held.graph),
        &["pages/A.md".into()],
    );
    assert_eq!(PAGES.with(|count| count.get()), 2);
    assert_eq!(
        patched
            .referenced_name_full_builds
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert!(!patched.tag_table_available("Target"));
    assert!(patched.tag_table_available("Next"));
    assert!(held.graph.tag_table_available("Target"));
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(crate::store::checkpoint::CheckpointWrite::Written { .. })
    ));
    store.close();
    let store = open();
    let loaded = store.whole_graph().unwrap();
    assert!(available(&loaded, "Target"));
    assert!(!available(&loaded, "Next"));
    assert_eq!(
        store.diagnostics()["checkpoint"]["load"]["outcome"],
        "loaded"
    );
    store.close();
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir(other.path().join("pages")).unwrap();
    write(other.path(), "pages/B.md", "- ordinary\n");
    let store = Store::open(other.path(), Default::default()).unwrap().0;
    assert!(!available(&store.whole_graph().unwrap(), "Target"));
    store.close();
}

#[test]
fn tag_availability_promotes_active_twins_and_publishes_only_membership_changes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("pages")).unwrap();
    write(root.path(), "pages/A.md", "title:: Shared\n\n- [[Old]]\n");
    write(root.path(), "pages/Z.md", "title:: Shared\n\n- [[New]]\n");
    write(root.path(), "pages/U.md", "- ordinary\n");
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let held = store.whole_graph().unwrap();
    check(&held, &[("Old", true), ("New", false)]);
    let sub = store.subscribe();
    write(root.path(), "pages/U.md", "- ordinary body changed\n");
    store.refresh(crate::Depth::Stamps).unwrap();
    let mut changes = 0;
    while let Some(change) = sub.try_recv().unwrap() {
        let wire = serde_json::to_value(change).unwrap();
        if wire["summary"]["full"] == true {
            continue;
        }
        assert_eq!(
            wire["summary"]["tagTableMembershipTo"],
            serde_json::json!([])
        );
        changes += 1;
    }
    assert!(changes > 0, "body edit must produce precise evidence");
    std::fs::remove_file(root.path().join("pages/A.md")).unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    check(
        &store.whole_graph().unwrap(),
        &[("Old", false), ("New", true)],
    );
    check(&held, &[("Old", true), ("New", false)]);
    store.close();
}
