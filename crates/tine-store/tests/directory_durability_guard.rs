use std::fs;
use std::path::Path;

fn rust_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn directory_sync_has_one_owner_and_no_discarded_result() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let owner = crate_root.join("src/directory_durability.rs");
    let helper = fs::read_to_string(&owner).unwrap();
    for target in ["linux", "android", "macos", "ios", "windows"] {
        assert!(
            helper.contains(&format!("target_os = \"{target}\"")),
            "directory sync must cover every shipped target; exemplar directory_durability::sync_directory_entry; missing {target}"
        );
    }

    let mut files = Vec::new();
    rust_files(&crate_root.join("src"), &mut files);
    rust_files(&crate_root.join("../../src-tauri/src"), &mut files);
    for path in files {
        if path == owner {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        for statement in source.split(';') {
            if statement.contains("let _ =")
                && (statement.contains("sync_all()") || statement.contains("sync_directory_entry("))
            {
                panic!(
                    "directory sync result must be checked; exemplar directory_durability::sync_directory_entry; discarded in {}",
                    path.display()
                );
            }
            if (statement.contains("File::open(") || statement.contains("into_std_file()"))
                && statement.contains("sync_all()")
            {
                panic!(
                    "directory sync must use its one helper; exemplar directory_durability::sync_directory_entry; direct sync in {}",
                    path.display()
                );
            }
        }
    }
}

/// Master 54dfcc1b6674 also swallows `EACCES`, `EBADF`, `EISDIR`,
/// `PermissionDenied` and `NotFound` from a directory sync. og deliberately
/// does not (see `sync_directory_entry`): a directory that vanished after the
/// rename took the renamed file with it, and an unopenable directory gives no
/// durability, so reporting either as synced would acknowledge a save that a
/// crash can lose. These stay errors the caller recovers from by re-reading.
#[cfg(unix)]
#[test]
fn directory_sync_reports_a_missing_or_unopenable_directory() {
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!("tine-dir-sync-errno-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let missing = base.join("gone");
    let error = tine_store::directory_durability::sync_directory_entry(&missing).unwrap_err();
    assert_eq!(
        error.kind(),
        ErrorKind::NotFound,
        "a vanished directory is not a synced one"
    );

    // Writable and searchable (a rename into it succeeds) but unreadable, so
    // the directory cannot be opened to sync it: EACCES.
    let unreadable = base.join("unreadable");
    fs::create_dir(&unreadable).unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o300)).unwrap();
    let openable = fs::File::open(&unreadable).is_ok();
    let result = tine_store::directory_durability::sync_directory_entry(&unreadable);
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o700)).unwrap();
    // A privileged user bypasses the permission bits; only assert where the
    // directory really could not be opened.
    if !openable {
        assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
    }
    let _ = fs::remove_dir_all(&base);
}
