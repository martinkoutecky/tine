use tine_core::model::PageKind;
use tine_graph_features::{assets, pages};
use tine_store::{OpenOptions, Store, StoreError};

fn page_target(store: &Store, path: &str) -> Result<std::path::PathBuf, String> {
    pages::source_path_for_os_handoff(store, "", PageKind::Page, Some(path)).map_err(|error| {
        match error {
            pages::PageReadError::Source(reason) => reason,
            pages::PageReadError::Store(StoreError::InvalidTarget(_)) => "invalid page path".into(),
            pages::PageReadError::Store(error) => format!("{error:?}"),
            pages::PageReadError::Load(error) => format!("{error:?}"),
            pages::PageReadError::EmptyAlias => "alias has no owner".into(),
        }
    })
}

fn asset_target(store: &Store, name: &str) -> Result<std::path::PathBuf, String> {
    assets::path_for_os_handoff(store, name).map_err(|error| match error {
        assets::AssetAccessError::BadName => "bad asset name".into(),
        assets::AssetAccessError::Store(StoreError::InvalidTarget(_)) => "invalid asset".into(),
        assets::AssetAccessError::Store(error) => format!("{error:?}"),
        assets::AssetAccessError::StreamSymlink => "asset symlinks cannot be streamed".into(),
    })
}

#[test]
fn open_targets_require_existing_regular_files() {
    let root = std::env::temp_dir().join(format!(
        "tine-handoff-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for area in ["pages", "journals", "assets"] {
        std::fs::create_dir_all(root.join(area)).unwrap();
    }
    std::fs::write(root.join("pages/Good.md"), "- good\n").unwrap();
    std::fs::write(root.join("assets/good.bin"), b"good").unwrap();
    std::fs::create_dir(root.join("pages/Directory.md")).unwrap();
    std::fs::create_dir(root.join("assets/directory.bin")).unwrap();
    let (store, _, _) = Store::open(&root, OpenOptions::default()).unwrap();

    assert_eq!(
        page_target(&store, "pages/Good.md").unwrap(),
        root.join("pages/Good.md")
    );
    assert_eq!(
        asset_target(&store, "good.bin").unwrap(),
        root.join("assets/good.bin")
    );
    assert!(page_target(&store, "pages/Missing.md").is_err());
    assert!(asset_target(&store, "missing.bin").is_err());
    assert_eq!(
        page_target(&store, "pages/Directory.md").unwrap_err(),
        "page source is not a file"
    );
    assert_eq!(
        asset_target(&store, "directory.bin").unwrap_err(),
        "invalid asset"
    );

    #[cfg(unix)]
    {
        std::fs::write(root.join("journals/Cross.md"), "- cross\n").unwrap();
        std::os::unix::fs::symlink(root.join("journals/Cross.md"), root.join("pages/Cross.md"))
            .unwrap();
        assert_eq!(
            page_target(&store, "pages/Cross.md").unwrap(),
            root.join("journals/Cross.md")
        );
        let outside = root.with_extension("outside.md");
        std::fs::write(&outside, "- outside\n").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("pages/Escape.md")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("assets/escape.bin")).unwrap();
        assert_eq!(
            page_target(&store, "pages/Escape.md").unwrap_err(),
            "page source escapes graph directories"
        );
        assert_eq!(
            asset_target(&store, "escape.bin").unwrap_err(),
            "invalid asset"
        );
        std::fs::remove_file(outside).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}
