use super::*;

fn fixture() -> (tempfile::TempDir, CloneFolder) {
    let dir = tempfile::Builder::new()
        .prefix("tine-clone-cleanup-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    let root = dir.path().join("cloned");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git/config"), b"fixture metadata").unwrap();
    let folder = CloneFolder {
        binding: CloneDirectory::new(&root).unwrap(),
        created: true,
        metadata: Some(CloneDirectory::new(&root.join(".git")).unwrap()),
    };
    (dir, folder)
}

#[test]
fn clone_cleanup_retains_unknown_contents_and_names_the_folder() {
    let (dir, folder) = fixture();
    let root = dir.path().join("cloned");
    fs::write(root.join("keep.md"), b"user arrival").unwrap();
    assert!(folder.cleanup().is_err());
    assert_eq!(fs::read(root.join("keep.md")).unwrap(), b"user arrival");
    assert!(root.join(".git/config").exists());
    assert!(
        matches!(retained(&root, Outcome::AuthRequired), Outcome::Refused(m)
        if m.contains(root.to_str().unwrap()) && m.contains("retained") && m.contains("Sign in"))
    );
}

#[test]
fn clone_cleanup_never_removes_unowned_git_metadata() {
    let (dir, mut folder) = fixture();
    folder.metadata = None;
    assert!(folder.cleanup().is_err());
    assert!(dir.path().join("cloned/.git/config").exists());
}

#[test]
fn clone_cleanup_retains_replaced_git_metadata() {
    let (dir, folder) = fixture();
    let root = dir.path().join("cloned");
    fs::rename(root.join(".git"), dir.path().join("original-git")).unwrap();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git/keep"), b"replacement metadata").unwrap();
    assert!(folder.cleanup().is_err());
    assert_eq!(
        fs::read(root.join(".git/keep")).unwrap(),
        b"replacement metadata"
    );
    assert!(dir.path().join("original-git/config").exists());
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
#[test]
fn clone_cleanup_retains_symlinks_without_following_them() {
    for location in [".git", ".git/objects", "outside-link"] {
        let (dir, folder) = fixture();
        let root = dir.path().join("cloned");
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"outside bytes").unwrap();
        if location == ".git" {
            fs::rename(root.join(".git"), dir.path().join("original-git")).unwrap();
        }
        std::os::unix::fs::symlink(&outside, root.join(location)).unwrap();
        assert!(folder.cleanup().is_err(), "{location}");
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"outside bytes");
        assert!(fs::symlink_metadata(root.join(location))
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[test]
fn clone_cleanup_retains_a_replaced_destination() {
    let (dir, folder) = fixture();
    let root = dir.path().join("cloned");
    fs::rename(&root, dir.path().join("original")).unwrap();
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git/keep"), b"replacement metadata").unwrap();
    assert!(folder.cleanup().is_err());
    assert_eq!(
        fs::read(root.join(".git/keep")).unwrap(),
        b"replacement metadata"
    );
    assert!(dir.path().join("original/.git/config").exists());
}
