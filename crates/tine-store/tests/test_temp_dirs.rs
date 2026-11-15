#[test]
fn stale_legacy_directory_does_not_get_reused() {
    let _integration_guard = crate::integration_lock();
    // The guard owns the planted stale directory even if this test panics.
    let stale = tempfile::tempdir().unwrap();
    let name = format!("tine-query-publish-{}-0", std::process::id());
    let legacy_path = stale.path().join(&name);
    std::fs::create_dir(&legacy_path).unwrap();
    std::fs::write(legacy_path.join("output"), "old export").unwrap();
    assert_eq!(
        std::fs::create_dir(&legacy_path).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );

    // An absolute prefix places tempfile's randomized candidate alongside the
    // old name, exercising the same allocator as all legacy fixtures.
    for _ in 0..2 {
        let fresh = crate::test_support::fresh_dir(legacy_path.to_str().unwrap());
        assert_ne!(fresh, legacy_path);
        assert_eq!(std::fs::read_dir(&fresh).unwrap().count(), 0);
        std::fs::write(fresh.join("output"), "new export").unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(legacy_path.join("output")).unwrap(),
        "old export"
    );
}
