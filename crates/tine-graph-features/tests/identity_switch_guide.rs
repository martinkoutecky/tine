#[test]
fn switching_instructions_keep_existing_graphs_and_explain_rollback() {
    let guide = include_str!("../../tine-core/src/templates/troubleshooting-recovery.md");
    for outcome in [
        "Close Tine before switching",
        "choose your existing graph folder",
        "do not delete application data",
        "Save your edits before switching",
        "Stable gets only stable updates",
        "Beta gets only Beta updates",
        "Copy version",
        "Android's separate Beta app",
    ] {
        assert!(
            guide.contains(outcome),
            "missing recovery instruction: {outcome}"
        );
    }
}

#[test]
fn update_guide_explains_device_opt_out_and_manual_checks() {
    let guide = tine_core::guide::bundled_guide_pages()
        .into_iter()
        .find(|page| page.title == "Reference/Platforms and mobile")
        .expect("platform guide is bundled");
    for outcome in [
        "Automatic checks are ON by default",
        "Settings → **About**",
        "**Check for updates automatically**",
        "stop startup checks and update notifications on this device",
        "still use **Check for updates** manually",
        "package-manager or manual-download installs",
        "Undo uses an arrow bending left",
        "Redo uses an arrow bending right",
    ] {
        assert!(
            guide.markdown.contains(outcome),
            "missing update instruction: {outcome}"
        );
    }
}
