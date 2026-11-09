#[test]
fn guide_explains_direct_capitalization_rename_and_twin_refusal() {
    let _integration_guard = crate::integration_lock();
    let tips = include_str!("../../tine-guide/src/templates/tips.md");
    assert!(tips.contains("`my note` → `My Note`"));
    assert!(tips.contains("filename, title and references follow together"));
    assert!(tips.contains("two distinct files claiming that name still block"));
}

#[test]
fn guide_explains_alias_rename_and_reload() {
    let guide = include_str!("../../tine-guide/src/templates/pages-links-references-search.md");
    assert!(guide.contains("aliases still work after a reload"));
    assert!(guide.contains("every declaring `alias:: ML` to `alias:: AI`"));
    assert!(guide.contains("no `AI` file is created"));
    assert!(guide.contains("no **Make it its own page** choice"));
}
