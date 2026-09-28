use super::*;

#[test]
fn nested_asset_reads_and_openers_stay_inside_assets() {
    let root = std::env::temp_dir().join(format!(
        "tine-nested-assets-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets/sub")).unwrap();
    std::fs::write(root.join("assets/sub/x.png"), b"png").unwrap();
    std::fs::write(root.join("outside.png"), b"outside").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(read_asset(&store, "sub/x.png", None).unwrap(), b"png");
    validate_stream_asset(&store, "sub/x.png").unwrap();
    assert_eq!(
        path_for_os_handoff(&store, "sub/x.png").unwrap(),
        // The handoff path is canonical (a verbatim `\\?\` path on Windows), as on master.
        root.join("assets/sub/x.png").canonicalize().unwrap()
    );
    for bad in [
        "../outside.png",
        "/outside.png",
        "C:/outside.png",
        "C:outside.png",
        "sub/../x.png",
        "sub\\x.png",
    ] {
        assert!(read_asset(&store, bad, None).is_err(), "{bad}");
        assert!(validate_stream_asset(&store, bad).is_err(), "{bad}");
        assert!(path_for_os_handoff(&store, bad).is_err(), "{bad}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&root, root.join("assets/escape")).unwrap();
        assert!(read_asset(&store, "escape/outside.png", None).is_err());
        assert!(validate_stream_asset(&store, "escape/outside.png").is_err());
        assert!(path_for_os_handoff(&store, "escape/outside.png").is_err());
    }
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
