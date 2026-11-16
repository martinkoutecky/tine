use super::*;
use std::collections::BTreeMap;
use std::fs;

fn payloads(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn collect(root: &Path, at: &Path, files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                collect(root, &entry.path(), files);
            } else {
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

pub(crate) fn initialize_fixture(root: &Path, data: &Path, url: &str) {
    let store = tine_store::Store::open(root, Default::default()).unwrap().0;
    init_graph(&store, root, data).unwrap();
    configure_remote(root, url).unwrap();
}

#[test]
fn init_refuses_an_existing_invalid_git_entry_without_changing_pages() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/a.md"), b"- unchanged\r\n").unwrap();
    let store = tine_store::Store::open(&root, Default::default())
        .unwrap()
        .0;
    fs::create_dir(root.join(".git")).unwrap();
    assert!(init_graph(&store, &root, &dir.path().join("device")).is_err());
    assert_eq!(
        fs::read(root.join("pages/a.md")).unwrap(),
        b"- unchanged\r\n"
    );
    assert_eq!(fs::read_dir(root.join(".git")).unwrap().count(), 0);
}

pub(crate) fn report_footprint(label: &str, root: &Path) {
    fn count(root: &Path) -> (u64, u64) {
        let mut total = (0, 0);
        for item in fs::read_dir(root).unwrap() {
            let item = item.unwrap();
            let m = item.metadata().unwrap();
            let (files, bytes) = if m.is_dir() {
                count(&item.path())
            } else {
                (1, m.len())
            };
            total.0 += files;
            total.1 += bytes;
        }
        total
    }
    let (files, bytes) = count(root);
    eprintln!("{label}: retained footprint {files} files, {bytes} bytes");
}

#[test]
fn init_uses_guarded_ignore_and_autocommit_without_changing_page_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("logseq/.tine-trash/pages")).unwrap();
    let page = b"- literal CRLF\r\n  - child\r\n";
    fs::write(root.join("pages/a.md"), page).unwrap();
    fs::write(
        root.join("logseq/.tine-trash/pages/deleted.md"),
        b"- trash\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), b"existing-rule\r\n").unwrap();
    let store = tine_store::Store::open(&root, Default::default())
        .unwrap()
        .0;
    let before = payloads(&root);
    init_graph(&store, &root, &dir.path().join("app-data")).unwrap();
    let after = payloads(&root);
    let changed: Vec<_> = after
        .iter()
        .filter(|(name, bytes)| before.get(*name) != Some(*bytes))
        .collect();
    eprintln!("init: {} distinct files written, {} final payload bytes (excludes repeated metadata writes and device settings)", changed.len(), changed.iter().map(|(_, bytes)| bytes.len()).sum::<usize>());
    assert!(changed
        .iter()
        .all(|(name, _)| name.starts_with(".git") || name.as_os_str() == ".gitignore"));
    assert_eq!(fs::read(root.join("pages/a.md")).unwrap(), page);
    assert_eq!(
        fs::read(root.join(".gitignore")).unwrap(),
        b"existing-rule\r\n.syncthing.*\r\nlogseq/.tine-*\r\n"
    );
    let repo = git2::Repository::open(&root).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(commit.parent_count(), 0);
    assert_eq!(commit.message().unwrap(), "Tine: initial graph");
    let identity = crate::owner::git_identity::user_identity(&repo, &crate::settings::your_name());
    assert_eq!(commit.author().name().unwrap(), identity.name.as_str());
    let tree = commit.tree().unwrap();
    assert!(tree
        .get_path(Path::new("logseq/.tine-trash/pages/deleted.md"))
        .is_err());
    assert_eq!(
        repo.find_blob(tree.get_path(Path::new("pages/a.md")).unwrap().id())
            .unwrap()
            .content(),
        page
    );
    assert!(!autocommit::GitSettings::default().policy.commits());
    assert!(init_graph(&store, &root, &dir.path().join("app-data"))
        .unwrap_err()
        .contains("already"));
    report_footprint("init", &root);
}
