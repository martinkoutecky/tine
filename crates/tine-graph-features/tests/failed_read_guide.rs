#[test]
fn bundled_recovery_guide_explains_failed_reads_and_safe_retries() {
    let guide = tine_core::guide::bundled_guide_pages()
        .into_iter()
        .find(|page| page.title == "Reference/Troubleshooting and recovery")
        .unwrap();
    assert!(guide.page.guide && guide.page.read_only);
    for outcome in [
        "graph opens read-only",
        "repair the file",
        "reopen the graph",
        "applies no custom CSS",
        "keeps the last successful conflict list",
        "has not observed can still race deletion",
        "report is incomplete",
    ] {
        assert!(
            guide.markdown.contains(outcome),
            "recovery Guide must explain {outcome}"
        );
    }
}
