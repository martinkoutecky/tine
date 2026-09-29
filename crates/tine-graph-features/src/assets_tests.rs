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

#[test]
fn asset_open_accepts_files_directories_and_the_assets_root() {
    // GH #367: the OS opener takes a regular file, a nested directory, and the
    // empty name (the assets root); traversal, missing names and escapes still
    // fail, and the edit handoff keeps its regular-file gate.
    let root = std::env::temp_dir().join(format!(
        "tine-asset-open-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let nested = root.join("assets/some dir/报表");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(&nested).unwrap();
    let file = nested.join("API ref.docx");
    std::fs::write(&file, b"doc").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let canonical = |p: std::path::PathBuf| p.canonicalize().unwrap();
    assert_eq!(
        path_for_os_open(&store, "").unwrap(),
        canonical(root.join("assets"))
    );
    assert_eq!(
        path_for_os_open(&store, "some dir").unwrap(),
        canonical(root.join("assets/some dir"))
    );
    assert_eq!(
        path_for_os_open(&store, "some dir/报表").unwrap(),
        canonical(nested.clone())
    );
    assert_eq!(
        path_for_os_open(&store, "some dir/报表/API ref.docx").unwrap(),
        canonical(file)
    );
    for bad in [
        "../outside",
        "/outside",
        "back\\slash.png",
        "missing.png",
        "sub/../x",
    ] {
        assert!(
            path_for_os_open(&store, bad).is_err(),
            "must reject {bad:?}"
        );
    }
    assert!(path_for_os_handoff(&store, "some dir").is_err());
    #[cfg(unix)]
    {
        std::fs::create_dir_all(root.join("outside-dir")).unwrap();
        std::os::unix::fs::symlink(root.join("outside-dir"), root.join("assets/escape")).unwrap();
        assert!(path_for_os_open(&store, "escape").is_err());
    }
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
