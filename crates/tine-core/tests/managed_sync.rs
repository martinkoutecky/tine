use std::fs;
use std::path::{Path, PathBuf};

use tine_core::model::{Graph, PageKind};
use uuid::Uuid;

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tine-managed-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(path.join("pages")).unwrap();
        fs::create_dir_all(path.join("journals")).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn update_chunks(root: &Path) -> Vec<PathBuf> {
    fn visit(path: &Path, output: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(&entry.path(), output);
            } else if entry.path().extension().and_then(|value| value.to_str()) == Some("chunk")
                && entry.path().to_string_lossy().contains("/sessions/")
            {
                output.push(entry.path());
            }
        }
    }
    let mut output = Vec::new();
    visit(&root.join(".tine-sync/v1"), &mut output);
    output
}

#[test]
fn delivered_operation_projects_to_identical_markdown_on_a_second_graph() {
    let left = TestDir::new("replica-left");
    let right = TestDir::new("replica-right");
    let page_path = left.path().join("pages/Page.md");
    fs::write(&page_path, "- original\n").unwrap();

    let left_graph = Graph::open(left.path());
    let enabled = left_graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    assert_eq!(enabled.migration.pages_changed, 1);
    assert_eq!(enabled.migration.blocks_changed, 1);
    assert!(fs::read_to_string(&page_path).unwrap().contains("id:: "));

    copy_tree(left.path(), right.path());
    let right_graph = Graph::open(right.path());
    assert!(right_graph
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap());

    let mut page = left_graph
        .load_named("Page", PageKind::Page)
        .unwrap()
        .unwrap();
    page.blocks[0].raw = page.blocks[0].raw.replacen("original", "edited on left", 1);
    left_graph.save_page(&page, page.rev.as_deref()).unwrap();
    let updates = update_chunks(left.path());
    assert_eq!(updates.len(), 1);

    let incoming = right.path().join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(&updates[0], incoming.join(updates[0].file_name().unwrap())).unwrap();
    let pull = right_graph.pull_managed_sync().unwrap();
    assert_eq!(pull.imported_chunks, 1);
    assert_eq!(pull.changes.len(), 1);
    assert_eq!(
        fs::read_to_string(right.path().join("pages/Page.md")).unwrap(),
        fs::read_to_string(&page_path).unwrap()
    );
}

#[test]
fn watcher_imports_external_markdown_and_persists_ids_for_new_blocks() {
    let dir = TestDir::new("external");
    let path = dir.path().join("pages/Page.md");
    fs::write(&path, "- original\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let before_updates = update_chunks(dir.path()).len();
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{}- added externally\n", original)).unwrap();

    let changed = graph
        .sync_file(&path)
        .expect("external projection is reported");
    assert_eq!(changed.name, "Page");
    let projected = fs::read_to_string(&path).unwrap();
    assert!(projected.contains("added externally"));
    assert_eq!(projected.matches("id:: ").count(), 2);
    assert_eq!(update_chunks(dir.path()).len(), before_updates + 1);
}

#[test]
fn stale_projection_is_rejected_before_an_operation_chunk_is_written() {
    let dir = TestDir::new("stale");
    let path = dir.path().join("pages/Page.md");
    fs::write(&path, "- original\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let mut stale = graph.load_named("Page", PageKind::Page).unwrap().unwrap();
    stale.blocks[0].raw = stale.blocks[0].raw.replacen("original", "stale edit", 1);
    fs::write(&path, "- external replacement\n").unwrap();
    let before = update_chunks(dir.path()).len();
    let error = graph.save_page(&stale, stale.rev.as_deref()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(update_chunks(dir.path()).len(), before);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "- external replacement\n"
    );
}

#[test]
fn delivered_delete_removes_only_the_known_projection_on_a_second_graph() {
    let left = TestDir::new("delete-left");
    let right = TestDir::new("delete-right");
    let page_path = left.path().join("pages/Page.md");
    fs::write(&page_path, "- keep until deleted\n").unwrap();

    let left_graph = Graph::open(left.path());
    left_graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    copy_tree(left.path(), right.path());
    let right_graph = Graph::open(right.path());
    right_graph
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();

    left_graph.delete_page("Page", PageKind::Page).unwrap();
    let updates = update_chunks(left.path());
    assert_eq!(updates.len(), 1);
    let incoming = right.path().join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(&updates[0], incoming.join(updates[0].file_name().unwrap())).unwrap();

    let pull = right_graph.pull_managed_sync().unwrap();
    assert_eq!(pull.imported_chunks, 1);
    assert!(pull.changes.iter().any(|change| change.removed));
    assert!(!right.path().join("pages/Page.md").exists());
}

#[test]
fn delivered_rename_moves_the_page_and_rewrites_referrers_as_one_operation() {
    let left = TestDir::new("rename-left");
    let right = TestDir::new("rename-right");
    fs::write(left.path().join("pages/Page.md"), "- target\n").unwrap();
    fs::write(
        left.path().join("pages/Referrer.md"),
        "- points to [[Page]]\n",
    )
    .unwrap();

    let left_graph = Graph::open(left.path());
    left_graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    copy_tree(left.path(), right.path());
    let right_graph = Graph::open(right.path());
    right_graph
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();

    left_graph.rename_page("Page", "Renamed").unwrap();
    let updates = update_chunks(left.path());
    assert_eq!(updates.len(), 1, "rename must be one graph transaction");
    let incoming = right.path().join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(&updates[0], incoming.join(updates[0].file_name().unwrap())).unwrap();

    let pull = right_graph.pull_managed_sync().unwrap();
    assert_eq!(pull.imported_chunks, 1);
    assert!(!right.path().join("pages/Page.md").exists());
    assert_eq!(
        fs::read_to_string(right.path().join("pages/Renamed.md")).unwrap(),
        fs::read_to_string(left.path().join("pages/Renamed.md")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(right.path().join("pages/Referrer.md")).unwrap(),
        fs::read_to_string(left.path().join("pages/Referrer.md")).unwrap()
    );
    assert!(fs::read_to_string(right.path().join("pages/Referrer.md"))
        .unwrap()
        .contains("[[Renamed]]"));
}

#[test]
fn managed_restore_is_durable_before_projection_and_removes_omitted_pages() {
    let dir = TestDir::new("restore");
    let page_path = dir.path().join("pages/Page.md");
    let omitted_path = dir.path().join("pages/Omitted.md");
    fs::write(&page_path, "- backup version\n").unwrap();
    fs::write(&omitted_path, "- omitted from backup\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let backup = fs::read_to_string(&page_path).unwrap();

    let mut page = graph.load_named("Page", PageKind::Page).unwrap().unwrap();
    page.blocks[0].raw = page.blocks[0]
        .raw
        .replacen("backup version", "newer live version", 1);
    graph.save_page(&page, page.rev.as_deref()).unwrap();
    let before = update_chunks(dir.path()).len();

    assert!(graph
        .commit_managed_restore(&[("pages/Page.md".into(), backup.clone())])
        .unwrap());
    assert_eq!(update_chunks(dir.path()).len(), before + 1);
    assert!(fs::read_to_string(&page_path)
        .unwrap()
        .contains("newer live version"));

    // Even unexplained bytes left at the operation-first crash boundary cannot
    // undo an explicit, verified restore when startup resumes projection.
    let unexplained = fs::read_to_string(&page_path).unwrap().replacen(
        "newer live version",
        "unexplained local bytes",
        1,
    );
    fs::write(&page_path, unexplained).unwrap();
    graph.project_all_managed_sync().unwrap();
    assert_eq!(fs::read_to_string(&page_path).unwrap(), backup);
    assert!(!omitted_path.exists());
}

#[test]
fn managed_restore_rejects_pre_identity_backups_before_writing_an_operation() {
    let dir = TestDir::new("restore-old-backup");
    fs::write(dir.path().join("pages/Page.md"), "- current\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let before = update_chunks(dir.path()).len();

    let error = graph
        .commit_managed_restore(&[("pages/Page.md".into(), "- old backup\n".into())])
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(update_chunks(dir.path()).len(), before);
}
