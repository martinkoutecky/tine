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
