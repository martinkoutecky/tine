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
