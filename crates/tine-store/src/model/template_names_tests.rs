use super::*;
use tine_core::template::template_work;

#[test]
fn picker_cost_is_independent_of_non_template_pages() {
    for count in [200, 2000] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        for i in 0..count {
            let text = if i < 3 {
                format!("- root\n  template:: Template {i}\n  - child\n    - grandchild\n")
            } else {
                "- ordinary\n  - child\n    - grandchild\n".into()
            };
            let file = root.path().join(format!("pages/P{i:04}.md"));
            std::fs::write(&file, text).unwrap();
            std::fs::File::open(file)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000),
                ))
                .unwrap();
        }
        let (store, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
        let view = store.whole_graph().unwrap();
        template_work::take();
        let start = std::time::Instant::now();
        let old = view.templates();
        let elapsed = start.elapsed();
        let work = template_work::take();
        let bytes = serde_json::to_vec(&old).unwrap().len();
        eprintln!(
            "before pages={count} documents={} blocks={} copied={} bytes={bytes} ns={}",
            work.0,
            work.1,
            work.2,
            elapsed.as_nanos()
        );
        assert_eq!(work, (count, count * 3, 9));
        assert_eq!(
            old.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            ["Template 0", "Template 1", "Template 2"]
        );
        let start = std::time::Instant::now();
        let names = view.template_names();
        let elapsed = start.elapsed();
        assert_eq!(
            names,
            old.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
        );
        assert_eq!(template_work::take(), (0, 0, 0));
        assert!(elapsed < std::time::Duration::from_millis(10));
        eprintln!(
            "after pages={count} documents=0 blocks=0 copied=0 names={} bytes={} ns={}",
            names.len(),
            serde_json::to_vec(&names).unwrap().len(),
            elapsed.as_nanos()
        );
        let mut before_ns = Vec::new();
        let mut after_ns = Vec::new();
        for _ in 0..51 {
            let start = std::time::Instant::now();
            std::hint::black_box(view.templates());
            before_ns.push(start.elapsed().as_nanos());
            let start = std::time::Instant::now();
            std::hint::black_box(view.template_names());
            after_ns.push(start.elapsed().as_nanos());
        }
        before_ns.sort_unstable();
        after_ns.sort_unstable();
        let rows = &view.graph.template_names.0;
        let bytes = 8 + rows
            .iter()
            .map(|(_, names)| {
                112 + names.len() * std::mem::size_of::<String>()
                    + names.iter().map(String::capacity).sum::<usize>()
            })
            .sum::<usize>();
        eprintln!("timing pages={count} before_p50_ns={} before_p95_ns={} after_p50_ns={} after_p95_ns={} resident_payload_bytes={bytes}", before_ns[25], before_ns[48], after_ns[25], after_ns[48]);
        store.close();
    }
}

#[test]
fn names_follow_headers_nested_templates_edits_deletions_and_warm_load() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("pages")).unwrap();
    let a = root.path().join("pages/A.md");
    let b = root.path().join("pages/B.org");
    std::fs::write(&a, "template:: Header\n\n- root\n  Template:: Same\n  template-including-parent:: false\n  - inner\n    template:: Nested\n- ignored\n  template::\n- code\n  ```\n  template:: Literal\n  ```\n").unwrap();
    std::fs::write(
        &b,
        "#+TEMPLATE: Same\n\n* row\n:PROPERTIES:\n:template: Org block\n:END:\n",
    )
    .unwrap();
    for path in [&a, &b] {
        std::fs::File::open(path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000))
            .unwrap();
    }
    let cp = root.path().join("launch.bin");
    let open = || {
        crate::Store::open(
            root.path(),
            crate::OpenOptions {
                watch: crate::WatchMode::Poll,
                launch_checkpoint: Some(cp.clone()),
                ..Default::default()
            },
        )
        .unwrap()
        .0
    };
    let store = open();
    let held = store.whole_graph().unwrap();
    let expected = vec!["Header", "Same", "Nested", "Same", "Org block"];
    assert_eq!(held.template_names(), expected);
    let check = |store: &crate::Store| {
        let view = store.whole_graph().unwrap();
        assert_eq!(
            view.template_names(),
            view.templates()
                .into_iter()
                .map(|t| t.name)
                .collect::<Vec<_>>()
        );
        view.template_names()
    };
    // Derivation work belongs only to changed pages, even for a first picker read.
    let mut pages = (*held.graph.pages).clone();
    let slot = *pages.positions.get("pages/A.md").unwrap();
    let entry = pages.get(slot).unwrap().0.clone();
    pages.set(
        slot,
        (
            entry,
            Arc::new(tine_core::doc::parse("- renamed\n  template:: Renamed\n")),
        ),
    );
    template_work::take();
    let patched =
        TemplateNames::capture(Some(&held.graph), &Arc::new(pages), &["pages/A.md".into()]);
    assert_eq!(template_work::take(), (1, 1, 0));
    assert_eq!(
        patched
            .0
            .iter()
            .flat_map(|(_, names)| names.iter().map(String::as_str))
            .collect::<Vec<_>>(),
        ["Renamed", "Same", "Org block"]
    );
    std::fs::write(&a, "- changed\n  template:: Changed\n").unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_eq!(check(&store), ["Changed", "Same", "Org block"]);
    std::fs::remove_file(&b).unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_eq!(check(&store), ["Changed"]);
    std::fs::rename(&a, root.path().join("pages/C.md")).unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_eq!(check(&store), ["Changed"]);
    std::fs::write(&a, "- restored\n  template:: New\n").unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    assert_eq!(check(&store), ["Changed", "New"]);
    assert_eq!(held.template_names(), expected);
    for rel in ["pages/A.md", "pages/C.md"] {
        std::fs::File::open(root.path().join(rel))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000))
            .unwrap();
    }
    store.refresh(crate::Depth::Stamps).unwrap();
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(crate::store::checkpoint::CheckpointWrite::Written { .. })
    ));
    store.close();
    assert!(cp.exists());
    let store = open();
    assert_eq!(check(&store), ["Changed", "New"]);
    assert_eq!(
        store.diagnostics()["checkpoint"]["load"]["outcome"],
        "loaded"
    );
    store.close();
}

#[test]
fn names_follow_the_existing_active_page_selection() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("pages")).unwrap();
    let winner = root.path().join("pages/A.md");
    std::fs::write(&winner, "title:: Shared\n\n- row\n  template:: Winner\n").unwrap();
    std::fs::write(
        root.path().join("pages/Z.md"),
        "title:: Shared\n\n- row\n  template:: Loser\n",
    )
    .unwrap();
    let store = crate::Store::open(root.path(), Default::default())
        .unwrap()
        .0;
    assert_eq!(store.whole_graph().unwrap().template_names(), ["Winner"]);
    std::fs::write(&winner, "title:: Different\n\n- ordinary\n").unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    let view = store.whole_graph().unwrap();
    assert_eq!(view.template_names(), ["Loser"]);
    assert_eq!(
        view.template_names(),
        view.templates()
            .into_iter()
            .map(|t| t.name)
            .collect::<Vec<_>>()
    );
    store.close();
}
