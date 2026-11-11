use super::*;

#[test]
fn add_under_ordinary_block_does_not_treat_a_later_org_drawer_as_a_comment() {
    let text = "* Passage\n** Ordinary\nProse before the drawer\n:PROPERTIES:\n:tine.quote: Passage\n:END:\n";
    let graph = Graph::seeded(Default::default(), &[("pages/Ordinary.org", text)]);
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({"name": "Ordinary"})).unwrap();
    let handle = read["page"]["blocks"][0]["children"][0]["handle"]
        .as_str()
        .unwrap();
    verb(
        &slot,
        "block.add",
        json!({"under": format!("Ordinary/{handle}"), "content": "* Added\n** Body\n"}),
    )
    .unwrap();
    let bytes = std::fs::read_to_string(graph.root.join("pages/Ordinary.org")).unwrap();
    assert!(!bytes.contains(":tine.date:"), "{bytes}");
}

#[test]
fn add_under_comment_dates_only_message_roots_in_markdown_and_org() {
    for (extension, text, fragment) in [
        ("md", "- Passage\n  - First\n    tine.quote:: Passage\n    tine.date:: 2026-11-07T13:37Z\n    - Existing reply\n", "- New message\n  - Evidence\n    - Nested evidence\n"),
        ("org", "* Passage\n** First\n:PROPERTIES:\n:tine.quote: Passage\n:tine.date: 2026-11-07T13:37Z\n:END:\n*** Existing reply\n", "* New message\n** Evidence\n*** Nested evidence\n"),
    ] {
        let file = format!("pages/Messages.{extension}");
        let graph = Graph::seeded(Default::default(), &[(&file, text)]);
        let slot = graph.slot();
        assert_eq!(std::fs::read_to_string(graph.root.join(&file)).unwrap(), text, "open never rewrites old files");
        let read = verb(&slot, "page.show", json!({"name": "Messages"})).unwrap();
        let handle = read["page"]["blocks"][0]["children"][0]["children"][0]["handle"].as_str().unwrap();
        verb(&slot, "block.add", json!({"under": format!("Messages/{handle}"), "content": fragment, "author": "Ada"})).unwrap();
        let doc = graph.store.page(&PageId::from(file.as_str())).unwrap().doc;
        let added = &doc.blocks[0].children[0].children[0].children[0];
        let org = extension == "org";
        let regions = tine_core::block_regions::parse(&added.raw, org);
        let date = &regions.property("tine.date").unwrap().value;
        assert!(chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%MZ").is_ok(), "{date}");
        for body in [&added.children[0], &added.children[0].children[0]] {
            let props = tine_core::block_regions::parse(&body.raw, org);
            assert!(props.property("tine.date").is_none());
            assert_eq!(props.property("tine.author").unwrap().value, "Ada");
        }
        let bytes = std::fs::read_to_string(graph.root.join(&file)).unwrap();
        assert_eq!(bytes.matches(if org { ":tine.date:" } else { "tine.date::" }).count(), 2);
    }
}
