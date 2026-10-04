use tine_core::guide::bundled_guide_pages;

#[test]
fn guide_explains_existing_code_block_deletion_paths() {
    let tips = bundled_guide_pages()
        .into_iter()
        .find(|page| page.title == "Features/Tips & shortcuts")
        .expect("tips guide is bundled");
    assert!(tips.markdown.contains("choose **Delete block**"));
    assert!(tips
        .markdown
        .contains("show line numbers and a display-only language label"));
    assert!(tips
        .markdown
        .contains("Settings → Appearance → Wrap code lines"));
    assert!(tips
        .markdown
        .contains("off by default and remembered on this device"));
    assert!(tips.markdown.contains("with no text selected"));
    assert!(tips.markdown.contains("Backspace in an empty code block"));
    assert!(tips
        .markdown
        .contains("last block, it becomes a normal empty bullet"));
}
