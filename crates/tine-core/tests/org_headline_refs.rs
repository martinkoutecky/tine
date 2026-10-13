use tine_core::doc::DocBlock;

#[test]
fn org_headline_tags_are_page_refs() {
    let mut block = DocBlock::new("TODO [[Plugins]] :Plugins:Équipe:Plugins:\r\nbody");
    block.set_org(true);
    assert_eq!(block.tags(), vec!["Plugins", "Équipe"]);
    assert!(block.projection().refs_contains("équipe"));
    assert_eq!(block.projection().refs_page(), &["Plugins", "Équipe"]);
    let evidence = block.projection().reference_source();
    let tags: Vec<_> = evidence
        .explicit
        .iter()
        .filter(|r| r.rule == "explicit_tag")
        .collect();
    assert_eq!(tags.len(), 3);
    for tag in tags {
        assert_eq!(&block.raw()[tag.range.clone()], tag.name);
    }
    block.set_raw("changed :Fresh:");
    assert!(!block.projection().refs_contains("équipe"));
    assert!(block.projection().refs_contains("fresh"));
    block.set_org(false);
    assert!(!block.projection().refs_contains("fresh"));
}

#[test]
fn headline_tag_spans_follow_parser_headers_and_whitespace() {
    for raw in ["  headline :Only:   ", "headline :Only:\nbody"] {
        let mut block = DocBlock::new(raw);
        block.set_org(true);
        assert!(block.projection().refs_contains("only"), "{raw}");
        let tag = block
            .projection()
            .reference_source()
            .explicit
            .iter()
            .find(|r| r.rule == "explicit_tag")
            .unwrap();
        assert_eq!(&raw[tag.range.clone()], "Only", "{raw}");
    }
}

#[test]
fn headline_tag_lookalikes_in_literals_are_not_page_refs() {
    for raw in [
        "~:Ghost:~",
        ":Ghost:",
        ":Ghost:Tags:",
        "body\n#+BEGIN_SRC\n:Ghost:\n#+END_SRC",
        "body\n:Ghost:",
    ] {
        let mut block = DocBlock::new(raw);
        block.set_org(true);
        assert!(!block.projection().refs_contains("Ghost"), "{raw}");
        assert!(
            block.projection().reference_source().explicit.is_empty(),
            "{raw}"
        );
    }
}
