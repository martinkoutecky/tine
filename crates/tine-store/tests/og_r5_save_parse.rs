//! I-15: a host save plans with one old-source parse and one splice check, then
//! publishes by reparsing the written bytes.
use std::fs;
use tine_store::{cost_counters, EditKind, PageId, SaveBase, Store};

#[test]
fn ordinary_host_save_plans_two_parses_and_publishes_the_written_tree() {
    let _integration_guard = crate::exclusive_integration_lock();
    for blocks in [1, 60] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(
            dir.path().join("pages/Page.md"),
            "- before\n".repeat(blocks),
        )
        .unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let store = std::sync::Arc::new(store);
        let host_writer = crate::test_host_writer::HostWriter::start(&store);
        let held = store.whole_graph().unwrap();
        let id = PageId::from("pages/Page.md");
        let read = store.page(&id).unwrap();
        let mut page = read.doc;
        page.blocks[0].raw = "after".into();
        cost_counters::reset();
        let kind = EditKind::ReplacePage;
        let mut plan = store.page_plan(Some(kind));
        plan.save_page(&[kind], &id, SaveBase::Existing(read.rev), &page);
        let op = plan.plan().unwrap();
        let planned = cost_counters::snapshot();
        host_writer.apply(op).unwrap();
        let cost = cost_counters::snapshot();
        eprintln!("R5 blocks={blocks}: {cost:?}");
        assert_eq!(
            planned.parses, 2,
            "I-15: one old-source parse and one splice check"
        );
        assert_eq!(
            cost.parses, 3,
            "I-15: the host adds one reparse of the bytes it publishes"
        );
        assert_eq!(store.page(&id).unwrap().doc.blocks[0].raw, "after");
        assert_eq!(
            held.corpus(None, false).pages[0].document.roots[0].raw(),
            "before"
        );
        store.close();
    }
}

#[test]
fn ambiguous_editor_submit_keeps_the_save_primitive_counts() {
    let _integration_guard = crate::exclusive_integration_lock();
    for ballast in [0, 53, 5993] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let tail: String = (0..ballast).map(|i| format!("- ballast {i}\n")).collect();
        let first = "- dup\n\t\t- x\n\t\t- a\n";
        let second = "- dup\n\t\t\t- x\n\t\t\t- b\n";
        let path = dir.path().join("pages/Page.md");
        fs::write(&path, format!("{first}{second}- target\n{tail}")).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
        let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        store.whole_graph_reconciled().unwrap();
        // Exact counters exclude the watcher's independently tested save echoes.
        store.stop_watcher_for_tests();
        let (sink, mail) = std::sync::mpsc::channel();
        let host = tine_store::PageHost::start(
            &store,
            &dir.path().join("app"),
            "cost",
            |_, _| {},
            move |m| {
                sink.send(serde_json::to_value(m).unwrap()).unwrap();
            },
        )
        .unwrap();
        let answer = |id: u64| loop {
            let value: serde_json::Value = mail
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            if value["answer"]["id"] == id {
                assert_eq!(value["answer"]["outcome"]["kind"], "applied", "{value}");
                break value;
            }
        };
        let reload = serde_json::to_value(host.window_reloaded()).unwrap();
        let session = reload["session"].as_u64().unwrap();
        let mut id = reload["nextId"].as_u64().unwrap();
        let page = PageId::from("pages/Page.md");
        host.open(session, id, &page, "Page").unwrap();
        let opened = answer(id);
        let key = opened["key"].as_str().unwrap();
        let mut dto: tine_core::model::PageDto =
            serde_json::from_value(opened["page"]["text"]["dto"].clone()).unwrap();
        dto.blocks.swap(0, 1);
        id += 1;
        host.submit(
            session,
            id,
            key,
            &dto,
            opened["page"]["version"].as_u64().unwrap(),
            None,
            &[EditKind::MoveBlocks],
        )
        .unwrap();
        answer(id);
        host.flush_input(&[page.clone()]).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("{second}{first}- target\n{tail}")
        );
        id += 1;
        host.open(session, id, &page, "Page").unwrap();
        let reopened = answer(id);
        let mut dto: tine_core::model::PageDto =
            serde_json::from_value(reopened["page"]["text"]["dto"].clone()).unwrap();
        dto.blocks[0].raw.push('!');
        let mut leaf = dto.blocks[2].clone();
        leaf.id = "new-leaf".into();
        leaf.raw = "inserted".into();
        dto.blocks.insert(0, leaf);
        cost_counters::reset();
        let start = std::time::Instant::now();
        id += 1;
        let (submitted, peak_bytes) = crate::query_unit_cost::peak_heap_of(|| {
            host.submit(
                session,
                id,
                key,
                &dto,
                reopened["page"]["version"].as_u64().unwrap(),
                None,
                &[EditKind::SaveBlock],
            )
        });
        submitted.unwrap();
        let submitted = start.elapsed();
        answer(id);
        host.flush_input(&[page]).unwrap();
        let elapsed = start.elapsed();
        let cost = cost_counters::snapshot();
        eprintln!(
            "ambiguous editor blocks={} submit={submitted:?} flush={elapsed:?} peak_bytes={peak_bytes} cost={cost:?}",
            ballast + 7
        );
        if ballast == 5993 {
            // Base 734810bed: debug submit 267 ms, through-flush 597 ms.
            assert!(
                submitted < std::time::Duration::from_secs(2),
                "{submitted:?}"
            );
            assert!(elapsed < std::time::Duration::from_secs(5), "{elapsed:?}");
        }
        assert_eq!(cost.parses, 3, "old source, splice check, publication");
        assert_eq!(cost.old_source_parses, 1);
        assert_eq!(cost.full_reads, 2, "save source and base guard");
        assert_eq!(cost.files_written, 1, "one guarded page save");
        assert_eq!(cost.fsyncs, 2, "temporary file and directory");
        assert_eq!(cost.readdir, 0);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!(
                "- inserted\n{}{first}- target\n{tail}",
                second.replacen("dup", "dup!", 1)
            )
        );
        drop(host);
        store.close();
    }
}
