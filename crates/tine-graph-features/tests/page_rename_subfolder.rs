//! GH #904: a rename changes the basename, retaining each page's folder.
use std::{fs, sync::Arc};
use tine_graph_features::pages;
use tine_store::{PageHost, Store};

fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Arc<Store>, tempfile::TempDir, PageHost) {
    let root = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = root.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let store = Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    let app = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app.path()).unwrap();
    (root, store, app, host)
}

#[test]
fn a_plain_rename_keeps_markdown_and_org_subfolders() {
    let _guard = crate::integration_lock();
    for (ext, body) in [("md", "- body\n"), ("org", "* body\n")] {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("pages/folder/nested")).unwrap();
        let old = format!("pages/folder/nested/Old.{ext}");
        fs::write(root.path().join(&old), body).unwrap();
        let store = Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
        let app = tempfile::tempdir().unwrap();
        let host = PageHost::start_for_tests(&store, app.path()).unwrap();
        pages::rename_page_expected(&store, &host, "Old", "New", Some(&old)).unwrap();
        assert_eq!(
            fs::read_to_string(root.path().join(format!("pages/folder/nested/New.{ext}"))).unwrap(),
            body
        );
        assert!(!root.path().join(&old).exists());
        assert!(!root.path().join(format!("pages/New.{ext}")).exists());
        store.close();
    }
}

#[test]
fn namespace_and_merge_renames_keep_each_files_parent() {
    let _guard = crate::integration_lock();
    for (new, survivor) in [
        ("New", None),
        ("New", Some("pages/target/New.md")),
        ("Nick", Some("pages/target/New.md")),
    ] {
        let mut files = vec![
            ("logseq/config.edn", "{:file/name-format :triple-lowbar}\n"),
            ("pages/source/Old.md", "- source [[Old/Child]]\n"),
            ("pages/children/Old___Child.org", "* child [[Old]]\n"),
            ("pages/Ref.md", "- [[Old]] [[Old/Child]]\n"),
        ];
        if survivor.is_some() {
            files.push(("pages/target/New.md", "alias:: Nick\n\n- survivor\n"));
        }
        let (root, store, _app, host) = fixture(&files);
        pages::rename_or_merge_page(
            &store,
            &host,
            "Old",
            new,
            Some("pages/source/Old.md"),
            survivor,
            &[],
        )
        .unwrap();
        assert!(!root.path().join("pages/source/Old.md").exists());
        let selected = if survivor.is_some() { "New" } else { new };
        let child = root
            .path()
            .join(format!("pages/children/{selected}___Child.org"));
        assert!(fs::read_to_string(child)
            .unwrap()
            .contains(&format!("[[{selected}]]")));
        assert_eq!(
            fs::read_to_string(root.path().join("pages/Ref.md")).unwrap(),
            format!("- [[{selected}]] [[{selected}/Child]]\n")
        );
        let root_result = survivor.unwrap_or("pages/source/New.md");
        let body = fs::read_to_string(root.path().join(root_result)).unwrap();
        assert!(body.contains("source"));
        if survivor.is_some() {
            assert!(body.contains("survivor"));
        }
        assert!(!root.path().join("pages/New.md").exists());
        store.close();
    }
}

#[test]
fn a_subfolder_destination_collision_never_overwrites_another_file() {
    let _guard = crate::integration_lock();
    // A title gives the occupied destination another identity, so the physical
    // no-replace check (rather than a name collision) must protect it.
    for destination in ["New.md", "new.md"] {
        let occupied = format!("pages/sub/{destination}");
        let files = [
            ("pages/sub/Old.md", "- source\n"),
            (occupied.as_str(), "title:: Other\n\n- keeper\n"),
        ];
        let (root, store, _app, host) = fixture(&files);
        let occupied_target = root.path().join("pages/sub/New.md").exists();
        let result = pages::rename_page_expected(&store, &host, "Old", "New", None);
        if occupied_target {
            assert!(result.is_err());
            assert_eq!(
                fs::read_to_string(root.path().join("pages/sub/Old.md")).unwrap(),
                "- source\n"
            );
        } else {
            // On a case-sensitive volume these are distinct entries with
            // distinct effective names. The title:: Other file stays intact.
            result.unwrap();
            assert_eq!(
                fs::read_to_string(root.path().join("pages/sub/New.md")).unwrap(),
                "- source\n"
            );
        }
        assert_eq!(
            fs::read_to_string(root.path().join(&occupied)).unwrap(),
            files[1].1
        );
        store.close();
    }
}

#[test]
fn a_subfolder_case_twin_with_the_destination_name_refuses() {
    let _guard = crate::integration_lock();
    let (root, store, _app, host) = fixture(&[
        ("pages/sub/Old.md", "- source\n"),
        ("pages/elsewhere/new.md", "- keeper\n"),
    ]);
    assert!(pages::rename_page_expected(&store, &host, "Old", "New", None).is_err());
    assert_eq!(
        fs::read_to_string(root.path().join("pages/sub/Old.md")).unwrap(),
        "- source\n"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("pages/elsewhere/new.md")).unwrap(),
        "- keeper\n"
    );
    store.close();
}

#[test]
fn a_case_only_rename_keeps_the_subfolder_under_a_custom_pages_directory() {
    let _guard = crate::integration_lock();
    let (root, store, _app, host) = fixture(&[
        ("logseq/config.edn", "{:pages-directory \"notes\"}\n"),
        ("notes/folder/Old.md", "- source\n"),
    ]);
    pages::rename_page_expected(&store, &host, "Old", "OLD", None).unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("notes/folder/OLD.md")).unwrap(),
        "- source\n"
    );
    assert!(!root.path().join("notes/OLD.md").exists());
    store.close();
}
