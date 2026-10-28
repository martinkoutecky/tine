use super::*;

#[test]
fn bounded_ranges_preserve_asset_access_and_detect_changes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("pages")).unwrap();
    std::fs::create_dir_all(root.path().join("assets/sub")).unwrap();
    let path = root.path().join("assets/sub/book.pdf");
    std::fs::write(&path, b"0123456789").unwrap();
    std::fs::write(root.path().join("outside.pdf"), b"outside").unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let (_, snapshot) = open(&store, "sub/book.pdf", 10).unwrap();
    assert_eq!(snapshot.length, 10);
    assert_eq!(
        read_range(&store, "sub/book.pdf", 10, 2, 6, &snapshot.version).unwrap(),
        b"2345"
    );
    assert_eq!(
        open(&store, "sub/book.pdf", 9).err().unwrap(),
        "asset-too-large"
    );
    for (begin, end) in [
        (0, 0),
        (5, 3),
        (0, 11),
        (0, RANGE_MAX_BYTES + 1),
        (u64::MAX, 0),
    ] {
        assert!(read_range(&store, "sub/book.pdf", 10, begin, end, &snapshot.version).is_err());
    }
    for bad in [
        "../outside.pdf",
        "/outside.pdf",
        "C:/outside.pdf",
        "sub/../book.pdf",
        "sub\\book.pdf",
        "sub",
    ] {
        assert!(open(&store, bad, 100).is_err(), "{bad}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&path, root.path().join("assets/inside.pdf")).unwrap();
        assert_eq!(
            read_range(&store, "inside.pdf", 10, 0, 2, &snapshot.version).unwrap(),
            b"01"
        );
        std::os::unix::fs::symlink(root.path(), root.path().join("assets/escape")).unwrap();
        assert!(open(&store, "escape/outside.pdf", 100).is_err());
    }
    std::fs::write(&path, b"changed").unwrap();
    assert!(
        read_range(&store, "sub/book.pdf", 10, 0, 2, &snapshot.version)
            .unwrap_err()
            .contains("changed")
    );
    store.close();
    assert!(open(&store, "sub/book.pdf", 10).is_err());
}
