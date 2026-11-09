use tine_core::DocBlock;

#[test]
fn onboarding_property_search_example_matches_the_search_projection() {
    let pages = tine_guide::bundled_guide_pages();
    for title in [
        "Workflows/Find and revisit",
        "Reference/Pages, links, references, and search",
    ] {
        let guide = pages.iter().find(|p| p.title == title).unwrap();
        assert!(guide.markdown.contains("reviewer:: Avery"));
        assert!(guide.markdown.contains("Hidden built-in properties"));
        assert!(guide.markdown.contains("Markdown and Org"));
    }
    let block = DocBlock::new("Body\nreviewer:: Avery\nid:: hiddenidentity");
    for query in ["reviewer", "avery"] {
        assert!(block.search_folded(true).contains(query));
    }
    assert!(!block.search_folded(true).contains("hiddenidentity"));
}
