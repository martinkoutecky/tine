#[test]
fn guide_explains_direct_capitalization_rename_and_twin_refusal() {
    let tips = include_str!("../../tine-core/src/templates/tips.md");
    assert!(tips.contains("`my note` → `My Note`"));
    assert!(tips.contains("filename, title and references follow together"));
    assert!(tips.contains("two distinct files claiming that name still block"));
}
