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

#[test]
fn onboarding_find_replace_names_the_skipped_line_the_find_bar_shows() {
    let pages = tine_guide::bundled_guide_pages();
    let guide = pages
        .iter()
        .find(|p| p.title == "Reference/Pages, links, references, and search")
        .unwrap();
    let line = guide
        .markdown
        .lines()
        .find(|l| l.contains("**Find and replace.**"))
        .unwrap();
    // The find bar's own words (catalog key `inpage-replace-skipped-here`).
    let catalog = include_str!("../../../src/i18n/locales/en/interface.ftl");
    assert!(catalog.contains("*[other] { $count } matches can't be replaced here"));
    assert!(line.contains("**N matches can't be replaced here**"));
    for skipped in [
        "an image's alt text",
        "under a collapsed parent",
        "the block you are editing",
    ] {
        assert!(line.contains(skipped), "{skipped}");
    }
}
