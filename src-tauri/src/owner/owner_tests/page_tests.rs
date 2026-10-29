use super::*;

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
        .contains("author::"));
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
            "* target\n:PROPERTIES:\n:author: Martin\n:END:\nbody\n",
        )],
    );
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({"name":"Org"})).unwrap();
    let handle = format!("Org/{}", block_handle(&read, 0));
    let first = verb(&slot, "embed", json!({"handle":handle})).unwrap();
    let second = verb(&slot, "ref", json!({"handle":first["uuid"]})).unwrap();
    assert_eq!(first["uuid"], second["uuid"]);
    let text = std::fs::read_to_string(graph.root.join("pages/Org.org")).unwrap();
    assert!(text.contains(":author: Martin"));
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
