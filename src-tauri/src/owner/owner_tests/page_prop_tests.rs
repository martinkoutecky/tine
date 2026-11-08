use super::*;

#[test]
fn gh839_page_props_journals_and_duplicate_claimants() {
    let graph = Graph::new();
    std::fs::create_dir_all(graph.root.join("journals")).unwrap();
    std::fs::write(graph.root.join("journals/2026_11_01.md"), "- day\n").unwrap();
    graph.store.refresh(tine_store::Depth::Stamps).unwrap();
    let slot = graph.slot();
    verb(
        &slot,
        "page.prop",
        json!({"journal":"2026-11-01","key":"tags","value":"technical"}),
    )
    .unwrap();
    assert!(
        std::fs::read_to_string(graph.root.join("journals/2026_11_01.md"))
            .unwrap()
            .starts_with("tags:: technical")
    );
    assert_eq!(
        error_code(verb(
            &slot,
            "page.prop",
            json!({"journal":"2026-11-02","key":"tags","value":"x"})
        ))
        .0,
        "not-found"
    );
    std::fs::write(graph.root.join("pages/twin.md"), "title:: a\n\n- twin\n").unwrap();
    graph.store.refresh(tine_store::Depth::Stamps).unwrap();
    let before = std::fs::read(graph.root.join("pages/a.md")).unwrap();
    assert_eq!(
        error_code(verb(
            &slot,
            "page.prop",
            json!({"page":"a","key":"tags","value":"x"})
        ))
        .0,
        "ambiguous"
    );
    assert_eq!(
        std::fs::read(graph.root.join("pages/a.md")).unwrap(),
        before
    );
}

#[test]
fn gh839_org_creation_imports_directives_and_drawers_as_page_properties() {
    let graph = Graph::new();
    std::fs::write(
        graph.root.join("logseq/config.edn"),
        "{:preferred-format :org}\n",
    )
    .unwrap();
    graph.store.refresh(tine_store::Depth::Stamps).unwrap();
    let slot = graph.slot();
    for (i, content) in [
        "#+tags: technical\n* body\n",
        ":PROPERTIES:\n:alias: Shortcut\n:END:\n* body\n",
    ]
    .iter()
    .enumerate()
    {
        let result = verb(
            &slot,
            "block.add",
            json!({"page":format!("OrgNew{i}"),"content":content}),
        )
        .unwrap();
        assert!(result["page"].as_str().unwrap().ends_with(".org"));
        let read = verb(&slot, "page.show", json!({"name":result["name"]})).unwrap();
        assert!(read["page"]["pre_block"]
            .as_str()
            .unwrap()
            .contains(if i == 0 { "technical" } else { "Shortcut" }));
        assert!(!read["page"]["pre_block"]
            .as_str()
            .unwrap()
            .contains("tine.author"));
    }
}

#[test]
fn gh839_new_page_import_accepts_page_headers() {
    for (i, content) in [
        "tags:: technical\n- block\n",
        "title:: Nice Title\n- block\n",
        "- tags:: technical\n",
        "- status:: done\n- block\n",
    ]
    .iter()
    .enumerate()
    {
        let graph = Graph::new();
        let slot = graph.slot();
        let result = verb(
            &slot,
            "block.add",
            json!({"page":format!("New{i}"),"content":content}),
        )
        .unwrap();
        let read = verb(&slot, "page.show", json!({"name":result["name"]})).unwrap();
        assert!(read["page"]["pre_block"].as_str().unwrap().contains(":: "));
        assert!(!read["page"]["pre_block"]
            .as_str()
            .unwrap()
            .contains("tine.author"));
        for block in read["page"]["blocks"].as_array().unwrap() {
            assert!(block["raw"]
                .as_str()
                .unwrap()
                .contains("tine.author:: agent"));
        }
    }
}

#[test]
fn gh839_page_properties_are_guarded_and_indexed_in_both_formats() {
    for (ext, text) in [("md", "- body\n"), ("org", "* body\n")] {
        let graph = Graph::new();
        std::fs::write(graph.root.join(format!("pages/Props.{ext}")), text).unwrap();
        graph.store.refresh(tine_store::Depth::Stamps).unwrap();
        let slot = graph.slot();
        for (key, value) in [
            ("tags", "technical"),
            ("alias", "Shortcut"),
            ("title", "Nice Title"),
        ] {
            let result = verb(
                &slot,
                "page.prop",
                json!({"page":"Props","key":key,"value":value}),
            )
            .unwrap();
            assert_eq!(result["blocks"], json!([]));
        }
        let read = verb(&slot, "page.show", json!({"name":"Shortcut"})).unwrap();
        assert_eq!(read["page"]["name"], "Nice Title");
        assert!(!read["text"].as_str().unwrap().contains("tine.author"));
        let tagged = verb(&slot, "query", json!({"text":"(page-tags technical)"})).unwrap();
        assert!(tagged.to_string().contains("Nice Title"), "{tagged}");
        assert_eq!(
            error_code(verb(
                &slot,
                "page.prop",
                json!({"page":"Shortcut","key":"tags","value":"wrong","rev":"old"})
            ))
            .0,
            "stale"
        );
        verb(
            &slot,
            "page.prop",
            json!({"page":"Shortcut","key":"tags","value":null,"rev":read["rev"]}),
        )
        .unwrap();
        let removed = verb(&slot, "page.show", json!({"name":"Shortcut"})).unwrap();
        assert!(!removed["text"].as_str().unwrap().contains("technical"));
        assert_eq!(
            error_code(verb(
                &slot,
                "page.prop",
                json!({"page":"Absent","key":"tags","value":"x"})
            ))
            .0,
            "not-found"
        );
    }
}

#[test]
fn gh839_existing_page_header_shape_refusal_gives_recovery() {
    let graph = Graph::new();
    std::fs::write(graph.root.join("pages/Empty.md"), "").unwrap();
    graph.store.refresh(tine_store::Depth::Stamps).unwrap();
    let error = verb(
        &graph.slot(),
        "block.add",
        json!({"page":"Empty","content":"- tags:: technical"}),
    )
    .unwrap_err();
    assert!(error.message.contains("page propert"), "{}", error.message);
    assert!(error.message.contains("tine prop set --page"));
    assert_eq!(
        std::fs::read_to_string(graph.root.join("pages/Empty.md")).unwrap(),
        ""
    );
}
