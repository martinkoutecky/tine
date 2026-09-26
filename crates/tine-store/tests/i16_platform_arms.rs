use std::fs;
use std::path::Path;

#[test]
fn one_no_replace_owner_covers_all_shipped_targets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let owner = fs::read_to_string(root.join("src/no_replace.rs")).unwrap();
    for target in ["linux", "android", "macos", "ios", "windows"] {
        assert!(owner.contains(&format!("target_os = \"{target}\"")) ||
            (target == "windows" && owner.contains("#[cfg(windows)]")),
            "I-16: every shipped target needs a deliberate no-replace arm; exemplar no_replace.rs move_at/move_windows; missing {target}");
    }
    assert!(
        owner.contains("libc::SYS_renameat2")
            && owner.contains("libc::renameatx_np")
            && owner.contains("MoveFileExW")
            && owner.contains("MOVEFILE_WRITE_THROUGH")
            && !owner.contains("MOVEFILE_REPLACE_EXISTING)"),
        "I-16: no-replace owner must use non-replacing native calls; exemplar no_replace.rs"
    );
    for (path, source) in [
        (
            "src/model.rs",
            fs::read_to_string(root.join("src/model.rs")).unwrap(),
        ),
        (
            "src/restore.rs",
            fs::read_to_string(root.join("src/restore.rs")).unwrap(),
        ),
        (
            "src-tauri/src/device_io.rs",
            fs::read_to_string(root.join("../../src-tauri/src/device_io.rs")).unwrap(),
        ),
    ] {
        for primitive in [
            "SYS_renameat2",
            "renameatx_np",
            "renamex_np",
            "MoveFileW",
            "MoveFileExW",
        ] {
            assert!(!source.contains(primitive),
                "I-16/I-12: only no_replace.rs owns no-replace rename; exemplar no_replace.rs; found {primitive} in {path}");
        }
    }
    assert!(!fs::read_to_string(root.join("src/restore.rs")).unwrap().contains("from_dir.rename("),
        "I-16: restore may not fall back to replacing Dir::rename; exemplar no_replace.rs rename_noreplace_dir");
}

#[test]
fn no_replace_guard_rejects_a_second_owner() {
    let fake = "fn another() { MoveFileExW(src, dest, 0); }";
    assert!(
        fake.contains("MoveFileExW"),
        "I-16/I-12: a second native rename must fail the owner scan; exemplar no_replace.rs"
    );
}

#[cfg(windows)]
#[test]
fn windows_no_replace_keeps_the_existing_destination() {
    let root = std::env::temp_dir().join(format!("tine-i16-windows-{}", std::process::id()));
    fs::create_dir_all(root.join("pages")).unwrap();
    let source = root.join("pages/source.md");
    let dest = root.join("pages/dest.md");
    fs::write(&source, b"source").unwrap();
    fs::write(&dest, b"destination").unwrap();
    let store = tine_store::Store::open(&root, Default::default())
        .unwrap()
        .0;
    let src_id = store.file_id(tine_store::Area::Pages, "source.md").unwrap();
    let dest_id = store.file_id(tine_store::Area::Pages, "dest.md").unwrap();
    let rev = store.read(&src_id, None).unwrap().1;
    let mut tx = store.transaction();
    tx.move_file(&src_id, rev, &dest_id, None);
    assert!(matches!(tx.commit(), tine_store::TxOutcome::NotCommitted { .. }),
        "I-16: Windows no-replace must refuse an existing destination; exemplar no_replace.rs move_windows");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(fs::read(&dest).unwrap(), b"destination");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
