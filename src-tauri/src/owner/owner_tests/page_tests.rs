use super::*;

/// GH #908: all CLI read verbs share the app's graph-text scope, including
/// the indexes and memoized answers restored from a headless checkpoint.
#[test]
fn cli_reads_ignore_logseq_backups_cold_and_from_checkpoint() {
    use tine_store::{CheckpointWrite, OpenOptions, WatchMode};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    let app_data = dir.path().join("app");
    let checkpoint = app_data.join("launch.bin");
    std::fs::create_dir_all(&app_data).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    let write = |relative: &str, text: &str| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    };
    write("logseq/config.edn", "{:file/name-format :triple-lowbar}\n");
    write("pages/anchor.md", &format!("- anchor\n  id:: {UUID}\n"));
    let body = format!("- #travel foo [[anchor]] (({UUID}))\n");
    write("journals/2026_01_15.md", &body);
    let excluded = [
        "logseq/bak/journals/2024_07_07/2026-11-07T18_06_21.481Z.android.md",
        "logseq/version-files/journals/versioned.md",
        "logseq/.recycle/recycled.md",
        "logseq/.tine-trash/pages/trashed.md",
        ".recycle/recycled-at-root.md",
        "pages/.tine-trash/nested-trash.md",
    ];
    let mut backup_ids = Vec::new();
    for (index, path) in excluded.iter().enumerate() {
        let id = format!("aaaaaaaa-0000-4000-8000-{index:012}");
        write(
            path,
            &format!("title:: backup-only/{index}\n\n{body}  id:: {id}\n"),
        );
        backup_ids.push(id);
    }

    for warm in [false, true] {
        let store = Arc::new(
            Store::open(
                &root,
                OpenOptions {
                    watch: WatchMode::Poll,
                    launch_checkpoint: Some(checkpoint.clone()),
                    ..Default::default()
                },
            )
            .unwrap()
            .0,
        );
        store.whole_graph_reconciled().unwrap();
        if warm {
            assert_eq!(
                store.diagnostics()["checkpoint"]["load"]["outcome"],
                "loaded",
                "the second pass must actually restore the checkpoint"
            );
        }
        let slot = GraphSlot::new(store.clone(), root.canonicalize().unwrap());
        *slot.host.write().unwrap() =
            PageHostSlot::Running(PageHost::start_for_tests(&store, &app_data).unwrap());
        let journal = verb(&slot, "journal", json!({"date":"2026-01-15"})).unwrap();
        assert_eq!(journal["id"], "journals/2026_01_15.md");
        let journal_name = journal["page"]["name"].as_str().unwrap();
        let live = verb(&slot, "page.show", json!({"name":journal_name})).unwrap();
        assert_eq!(live["id"], journal["id"]);
        assert!(verb(&slot, "block.show", json!({"uuid":UUID})).is_ok());
        for (index, id) in backup_ids.iter().enumerate() {
            assert_eq!(
                error_code(verb(&slot, "block.show", json!({"uuid":id}))).0,
                "not-found"
            );
            assert_eq!(
                error_code(verb(
                    &slot,
                    "page.show",
                    json!({"name":format!("backup-only/{index}")})
                ))
                .0,
                "not-found"
            );
        }
        let pages = verb(&slot, "pages", Value::Null).unwrap();
        assert_eq!(pages["pages"].as_array().unwrap().len(), 2, "{pages}");
        let namespace = verb(&slot, "pages", json!({"namespace":"backup-only"})).unwrap();
        assert!(namespace["pages"].as_array().unwrap().is_empty());
        for (name, args) in [
            ("query", json!({"text":"[[travel]]"})),
            ("refs", json!({"target":"anchor"})),
            ("refs", json!({"target":UUID,"block":true})),
        ] {
            let answer = verb(&slot, name, args).unwrap();
            let groups = answer["groups"].as_array().unwrap();
            assert_eq!(groups.len(), 1, "{name}: {answer}");
            assert_eq!(groups[0]["page"], journal_name);
            let mut text = String::new();
            super::super::terminal::render(&answer, &mut text);
            assert!(text.contains("#travel foo"), "{name}: {text}");
            assert!(!text.contains("backup-only"), "{name}: {text}");
        }
        let search = verb(&slot, "search", json!({"text":"foo"})).unwrap();
        assert_eq!(search["hits"].as_array().unwrap().len(), 1, "{search}");
        let absent = verb(&slot, "search", json!({"text":"backup-only"})).unwrap();
        assert!(absent["hits"].as_array().unwrap().is_empty(), "{absent}");
        assert!(matches!(
            store.write_checkpoint_now(),
            Some(CheckpointWrite::Written { .. })
        ));
        drop(slot);
        store.close();
    }
}

#[test]
fn page_create_rename_delete_and_lazy_reference_helpers() {
    let graph = Graph::new();
    let slot = graph.slot();
    let created = verb(&slot, "page.create", json!({"name":"New/Page"})).unwrap();
    assert_eq!(created["name"], "New/Page");
    assert!(graph.root.join(created["page"].as_str().unwrap()).exists());
    assert_eq!(
        error_code(verb(&slot, "page.create", json!({"name":"New/Page"}))).0,
        "refused"
    );
    let page = verb(&slot, "page.show", json!({"name":"a"})).unwrap();
    let handle = format!(
        "a/{}",
        page["page"]["blocks"][1]["handle"].as_str().unwrap()
    );
    let reference = verb(&slot, "ref", json!({"handle":handle})).unwrap();
    let uuid = reference["uuid"].as_str().unwrap();
    assert_eq!(reference["text"], format!("(({uuid}))"));
    assert!(std::fs::read_to_string(graph.root.join("pages/a.md"))
        .unwrap()
        .contains(uuid));
    let embed = verb(&slot, "embed", json!({"handle":uuid})).unwrap();
    assert_eq!(embed["text"], format!("{{{{embed (({uuid}))}}}}"));
    assert!(!std::fs::read_to_string(graph.root.join("pages/a.md"))
        .unwrap()
        .contains("tine.author::"));
    let renamed = verb(&slot, "page.rename", json!({"old":"a", "new":"A"})).unwrap();
    assert_eq!(renamed["report"]["outcome"], "renamed");
    assert!(graph.root.join("pages/A.md").exists());
    assert!(std::fs::read_to_string(graph.root.join("pages/b.md"))
        .unwrap()
        .contains("[[A]]"));
    assert_eq!(
        error_code(verb(&slot, "page.delete", json!({"name":"A"}))).0,
        "refused"
    );
    let deleted = verb(&slot, "page.delete", json!({"name":"A", "yes":true})).unwrap();
    assert_eq!(deleted["broken_refs"], 2);
    assert!(!graph.root.join("pages/A.md").exists());
}

#[test]
fn rename_refuses_existing_destination_and_preserves_both_pages() {
    let graph = Graph::new();
    let slot = graph.slot();
    let before = std::fs::read(graph.root.join("pages/a.md")).unwrap();
    assert_eq!(
        error_code(verb(&slot, "page.rename", json!({"old":"a", "new":"b"}))).0,
        "refused"
    );
    assert_eq!(
        std::fs::read(graph.root.join("pages/a.md")).unwrap(),
        before
    );
}

fn rename_then_delete(old: &str, new: &str, namespace: bool) {
    let graph = Graph::new();
    if namespace {
        seed(&graph, &[("pages/ns.md", "- [[ns/child]]\n")]);
    }
    let slot = graph.slot();
    verb(&slot, "page.show", json!({"name":old})).unwrap();
    let renamed = verb(&slot, "page.rename", json!({"old":old,"new":new})).unwrap();
    assert_eq!(renamed["report"]["outcome"], "renamed");
    let mut names = vec![new.to_owned()];
    if namespace {
        names.push(format!("{new}/child"));
    }
    for name in names {
        assert!(
            matches!(
                graph.store.whole_graph().unwrap().resolve(&name, false),
                tine_store::Resolved::Existing { .. }
            ),
            "completed rename lost {name}"
        );
        let deleted = verb(&slot, "page.delete", json!({"name":name,"yes":true})).unwrap();
        assert_eq!(deleted["name"], name);
    }
}

#[test]
fn non_case_rename_then_delete() {
    rename_then_delete("a", "Renamed", false);
}

#[test]
fn namespace_rename_then_delete() {
    rename_then_delete("ns", "Moved", true);
}

#[test]
fn case_namespace_rename_then_delete() {
    rename_then_delete("ns", "NS", true);
}

fn seed(graph: &Graph, files: &[(&str, &str)]) {
    for (file, text) in files {
        std::fs::write(graph.root.join(file), text).unwrap();
    }
    graph.store.refresh(tine_store::Depth::Stamps).unwrap();
}

#[test]
#[cfg(unix)]
fn rename_home_config_failure_discloses_published_pages() {
    use std::os::unix::fs::PermissionsExt;
    let graph = Graph::new();
    seed(
        &graph,
        &[(
            "logseq/config.edn",
            "{:file/name-format :triple-lowbar :default-home {:page \"a\"}}\n",
        )],
    );
    // The host can publish pages and trash the source, but the subsequent
    // config transaction cannot create its temporary file in logseq/.
    std::fs::create_dir_all(graph.root.join("logseq/.tine-trash/pages")).unwrap();
    let slot = graph.slot();
    let dir = graph.root.join("logseq");
    let permissions = std::fs::metadata(&dir).unwrap().permissions();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = verb(&slot, "page.rename", json!({"old":"a","new":"New"}));
    std::fs::set_permissions(&dir, permissions).unwrap();
    assert!(graph.root.join("pages/New.md").exists());
    assert!(!graph.root.join("pages/a.md").exists());
    assert!(std::fs::read_to_string(graph.root.join("pages/b.md"))
        .unwrap()
        .contains("[[New]]"));
    assert!(
        std::fs::read_to_string(graph.root.join("logseq/config.edn"))
            .unwrap()
            .contains("\"a\"")
    );
    let error = result.unwrap_err();
    assert_eq!(error.code, "uncertain");
    let detail = error.detail.unwrap();
    assert_eq!(detail["admitted"], true);
    assert_eq!(detail["report"]["outcome"], "renamed");
    assert!(detail["report"]["touched"]
        .as_array()
        .unwrap()
        .iter()
        .any(|page| page["path"] == "pages/b.md"));
}

#[test]
fn rename_fileless_sources_rewrites_references_and_moves_namespace_children() {
    for child in [false, true] {
        let graph = Graph::new();
        seed(
            &graph,
            &[("pages/Links.md", "- [[Ghost]] and [[Ghost/child]]\n")],
        );
        if child {
            seed(&graph, &[("pages/Ghost___child.md", "- child\n")]);
        }
        let slot = graph.slot();
        let result = verb(
            &slot,
            "page.rename",
            json!({"old":"Ghost","new":"NewGhost"}),
        )
        .unwrap();
        assert_eq!(result["report"]["outcome"], "renamed");
        let links = std::fs::read_to_string(graph.root.join("pages/Links.md")).unwrap();
        assert!(links.contains("[[NewGhost]]"));
        if child {
            assert!(links.contains("[[NewGhost/child]]"));
            assert_eq!(
                std::fs::read_to_string(graph.root.join("pages/NewGhost___child.md")).unwrap(),
                "- child\n"
            );
            assert!(!graph.root.join("pages/Ghost___child.md").exists());
        }
        assert!(!graph.root.join("pages/Ghost.md").exists());
        assert!(!graph.root.join("pages/NewGhost.md").exists());
    }
}

#[test]
fn rename_uses_title_identity_aliases_and_namespace_descendants_in_both_formats() {
    for (ext, header, block) in [
        ("md", "title:: Project\nalias:: Shortcut\n\n", "- "),
        ("org", "#+TITLE: Project\n#+ALIAS: Shortcut\n", "* "),
    ] {
        let graph = Graph::new();
        seed(
            &graph,
            &[
                (
                    &format!("pages/different.{ext}"),
                    &format!("{header}{block}self [[Project]]\n"),
                ),
                ("pages/Project___child.md", "- child\n"),
                ("pages/Links.md", "- [[Project]] and [[Project/child]]\n"),
            ],
        );
        let slot = graph.slot();
        let result = verb(
            &slot,
            "page.rename",
            json!({"old":"Shortcut","new":"New/Project"}),
        )
        .unwrap();
        assert_eq!(result["old"], "Project");
        let page = verb(&slot, "page.show", json!({"name":"New/Project"})).unwrap();
        assert!(page["text"].as_str().unwrap().contains("[[New/Project]]"));
        assert!(verb(&slot, "page.show", json!({"name":"New/Project/child"})).is_ok());
        assert_eq!(
            std::fs::read_to_string(graph.root.join("pages/Links.md")).unwrap(),
            "- [[New/Project]] and [[New/Project/child]]\n"
        );
    }
}

#[test]
fn org_helpers_stamp_a_drawer_once_and_preserve_existing_authorship() {
    let graph = Graph::new();
    seed(
        &graph,
        &[(
            "pages/Org.org",
            "* target\n:PROPERTIES:\n:tine.author: Martin\n:END:\nbody\n",
        )],
    );
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({"name":"Org"})).unwrap();
    let handle = format!("Org/{}", block_handle(&read, 0));
    let first = verb(&slot, "embed", json!({"handle":handle})).unwrap();
    let second = verb(&slot, "ref", json!({"handle":first["uuid"]})).unwrap();
    assert_eq!(first["uuid"], second["uuid"]);
    let text = std::fs::read_to_string(graph.root.join("pages/Org.org")).unwrap();
    assert!(text.contains(":tine.author: Martin"));
    assert_eq!(text.to_lowercase().matches(":id:").count(), 1);
    assert!(!text.contains("id::"));
}

#[test]
fn page_delete_stale_guard_and_duplicate_identity_refuse_without_changes() {
    let graph = Graph::new();
    let slot = graph.slot();
    assert_eq!(
        error_code(verb(
            &slot,
            "page.delete",
            json!({"name":"a","yes":true,"rev":"old"})
        ))
        .0,
        "stale"
    );
    assert!(graph.root.join("pages/a.md").exists());
    seed(&graph, &[("pages/twin.md", "title:: a\n\n- twin\n")]);
    for (name, args) in [
        ("page.rename", json!({"old":"a","new":"renamed"})),
        ("page.delete", json!({"name":"a","yes":true})),
    ] {
        assert_eq!(error_code(verb(&slot, name, args)).0, "ambiguous");
    }
    assert!(graph.root.join("pages/a.md").exists());
    assert!(graph.root.join("pages/twin.md").exists());
}

#[test]
fn rename_options_and_destination_alias_refuse_before_writing() {
    let graph = Graph::new();
    seed(
        &graph,
        &[("pages/owner.md", "alias:: Destination\n\n- owner\n")],
    );
    let slot = graph.slot();
    for args in [
        json!({"old":"a","new":"other","rev":"read-rev"}),
        json!({"old":"a","new":"other","run":"run-id"}),
        json!({"old":"a","new":"Destination"}),
    ] {
        assert_eq!(error_code(verb(&slot, "page.rename", args)).0, "refused");
    }
    assert!(graph.root.join("pages/a.md").exists());
}

/// CLI-DUPNAME (D-20261103-CMA): a write to a page name that two files claim
/// (a sync conflict, a case twin, a `title::` collision) refuses `ambiguous`,
/// naming both files, and writes neither. Before, it went silently to the
/// file that owns the name: an agent edited one twin and never learned the
/// other existed. Reads keep answering the owning file.
#[test]
fn a_cli_write_to_a_name_two_files_claim_refuses_naming_both() {
    let graph = Graph::new();
    seed(&graph, &[("pages/twin.md", "title:: a\n\n- twin\n")]);
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({"name":"a"})).expect("a read still answers");
    assert_eq!(read["id"], "pages/a.md");
    let one = format!("a/{}", block_handle(&read, 0));
    let two = format!("a/{}", block_handle(&read, 1));
    let files = |graph: &Graph, names: &[&str]| -> Vec<String> {
        names
            .iter()
            .map(|n| std::fs::read_to_string(graph.root.join(n)).unwrap())
            .collect()
    };
    let before = files(&graph, &["pages/a.md", "pages/twin.md"]);
    let assert_refused = |what: &str, result: Result<Value, ErrorBody>, both: [&str; 2]| {
        let body = result.expect_err(what);
        assert_eq!(body.code, "ambiguous", "{what}: {}", body.message);
        for file in both {
            assert!(body.message.contains(file), "{what}: {}", body.message);
        }
        let detail = body.detail.expect("detail");
        assert_eq!(detail["admitted"], false, "{what}");
        let candidates: Vec<&str> = detail["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(candidates, both, "{what}");
    };
    let both = ["pages/a.md", "pages/twin.md"];
    for (name, args) in [
        ("block.add", json!({"page":"a","content":"- new"})),
        ("block.add", json!({"page":"A","content":"- new"})),
        ("block.add", json!({"under":one,"content":"- new"})),
        ("block.add", json!({"after":one,"content":"- new"})),
        ("block.edit", json!({"handle":one,"text":"changed"})),
        ("block.edit", json!({"handle":UUID,"text":"changed"})),
        (
            "block.prop",
            json!({"handle":one,"key":"status","value":"x"}),
        ),
        ("block.move", json!({"handle":two,"to":one,"at":"under"})),
        ("block.delete", json!({"handle":two})),
        ("ref", json!({"handle":two})),
        ("embed", json!({"handle":two})),
        ("page.delete", json!({"name":"a","yes":true})),
        ("page.rename", json!({"old":"a","new":"renamed"})),
    ] {
        assert_refused(&format!("{name} {args}"), verb(&slot, name, args), both);
    }
    assert_eq!(files(&graph, &["pages/a.md", "pages/twin.md"]), before);

    // A journal day two files claim (Markdown and Org).
    std::fs::create_dir_all(graph.root.join("journals")).unwrap();
    seed(
        &graph,
        &[
            ("journals/2026_01_05.md", "- md day\n"),
            ("journals/2026_01_05.org", "* org day\n"),
        ],
    );
    let days = ["journals/2026_01_05.md", "journals/2026_01_05.org"];
    let before = files(&graph, &days);
    assert_refused(
        "block.add --journal",
        verb(
            &slot,
            "block.add",
            json!({"journal":"2026-01-05","content":"- new"}),
        ),
        days,
    );
    assert_eq!(files(&graph, &days), before);

    // A case twin, where the file system keeps both spellings.
    seed(
        &graph,
        &[("pages/Foo.md", "- upper\n"), ("pages/foo.md", "- lower\n")],
    );
    let listed = std::fs::read_dir(graph.root.join("pages"))
        .unwrap()
        .filter(|e| {
            let name = e.as_ref().unwrap().file_name();
            name == "Foo.md" || name == "foo.md"
        })
        .count();
    if listed == 2 {
        let twins = ["pages/Foo.md", "pages/foo.md"];
        let before = files(&graph, &twins);
        assert_refused(
            "block.add --page Foo",
            verb(&slot, "block.add", json!({"page":"foo","content":"- new"})),
            twins,
        );
        assert_eq!(files(&graph, &twins), before);
    }
}

/// GH #908: run from the folder above a graph (`~` with the graph at
/// `~/logseq`), discovery must not take that folder for the graph: its
/// `logseq/` child is the graph itself, not graph metadata.
#[test]
fn cli_graph_discovery_needs_logseq_config_not_a_bare_logseq_folder() {
    use crate::owner::terminal::graph_around;
    let home = tempfile::tempdir().unwrap();
    let graph = home.path().join("logseq");
    std::fs::create_dir_all(graph.join("logseq/bak/journals/2024_07_07")).unwrap();
    std::fs::create_dir_all(graph.join("journals")).unwrap();
    std::fs::create_dir_all(graph.join("pages/deep")).unwrap();
    std::fs::write(graph.join("logseq/config.edn"), "{}\n").unwrap();
    assert_eq!(
        graph_around(home.path()),
        None,
        "the folder above the graph is not a graph"
    );
    assert_eq!(graph_around(&graph).as_deref(), Some(graph.as_path()));
    assert_eq!(
        graph_around(&graph.join("pages/deep")).as_deref(),
        Some(graph.as_path())
    );
    assert_eq!(
        graph_around(&graph.join("logseq/bak")).as_deref(),
        Some(graph.as_path())
    );
}
