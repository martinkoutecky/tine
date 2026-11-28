use super::*;

const TEST_ASSET_MAX_BYTES: u64 = 16;

#[test]
fn nested_asset_reads_and_openers_stay_inside_assets() {
    let root = crate::test_support::fresh_dir("tine-nested-assets-");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets/sub")).unwrap();
    std::fs::write(root.join("assets/sub/x.png"), b"png").unwrap();
    std::fs::write(root.join("outside.png"), b"outside").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(
        read_asset(&store, "sub/x.png", TEST_ASSET_MAX_BYTES).unwrap(),
        b"png"
    );
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
        assert!(
            read_asset(&store, bad, TEST_ASSET_MAX_BYTES).is_err(),
            "{bad}"
        );
        assert!(validate_stream_asset(&store, bad).is_err(), "{bad}");
        assert!(path_for_os_handoff(&store, bad).is_err(), "{bad}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&root, root.join("assets/escape")).unwrap();
        assert!(read_asset(&store, "escape/outside.png", TEST_ASSET_MAX_BYTES).is_err());
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
    let root = crate::test_support::fresh_dir("tine-asset-open-");
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

#[test]
fn a_bounded_asset_read_refuses_an_oversized_file_and_accepts_the_limit() {
    const EXACT_MAX_BYTES: u64 = 5;
    const TOO_SMALL_MAX_BYTES: u64 = EXACT_MAX_BYTES - 1;
    let root = crate::test_support::fresh_dir("tine-bounded-asset-read-");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("assets/large.pdf"), b"12345").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(
        read_asset(&store, "large.pdf", EXACT_MAX_BYTES).unwrap(),
        b"12345",
        "a file exactly at the limit is read"
    );
    assert!(
        matches!(
            read_asset(&store, "large.pdf", TOO_SMALL_MAX_BYTES),
            Err(AssetAccessError::Store(StoreError::TooLarge {
                limit: 4,
                ..
            }))
        ),
        "one byte over the limit is refused before any bytes are returned"
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

/// Through the store: for a sole reference in each spelling the link-target
/// owner reads, the orphan list leaves out exactly the file the reader shows
/// and trash refuses it. Look-alikes the reader does not show stay unused.
#[test]
fn orphans_and_trash_see_the_file_the_reader_shows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for folder in ["pages", "journals", "assets/sub"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    let page = [
        "- ![x](../ASSETS/upper.png)",
        "- ![x](../assets%5Cbackslash.png)",
        "- ![x](../assets/sub%5Cnested.png)",
        "- {{img ../assets/macro.png, 200}}",
        "- [[../assets/org-bare.png]]",
        "- <video poster=\"../assets/poster.png\"></video>",
        "- [:img {:src \"../assets/hiccup.png\"}]",
        "- ![x](../assets/photo%23v2.png)",
        "- ![x](../assets/demo%2520v2.png)",
    ];
    std::fs::write(root.join("pages/a.md"), page.join("\n") + "\n").unwrap();
    let shown = [
        "upper.png",
        "backslash.png",
        "macro.png",
        "org-bare.png",
        "poster.png",
        "hiccup.png",
        "photo#v2.png",
        "demo%20v2.png",
    ];
    // Look-alikes the reader does not show: the twice-decoded spelling, and
    // a file nothing names.
    let others = ["demo v2.png", "unused.png"];
    for name in shown.iter().chain(&others).chain(&["sub/nested.png"]) {
        std::fs::write(root.join("assets").join(name), name.as_bytes()).unwrap();
    }
    let (store, _, _) = Store::open(root, Default::default()).unwrap();

    let orphans: Vec<String> = orphan_assets(&store)
        .unwrap()
        .into_iter()
        .map(|asset| asset.name)
        .collect();
    for name in shown {
        assert!(
            !orphans.contains(&name.to_owned()),
            "{name} listed as unused: {orphans:?}"
        );
    }
    assert!(orphans.contains(&"unused.png".to_owned()), "{orphans:?}");
    assert!(orphans.contains(&"demo v2.png".to_owned()), "{orphans:?}");

    for name in shown {
        assert_eq!(
            trash_asset(&store, name).unwrap(),
            TrashOutcome::Referenced,
            "{name}"
        );
        assert!(root.join("assets").join(name).exists(), "{name}");
    }
    assert_eq!(
        trash_asset(&store, "unused.png").unwrap(),
        TrashOutcome::Trashed
    );
    store.close();
}

/// Through the store: an in-use attachment is never offered for trash when a
/// surface opens it under another spelling than develop's liveness kept (a
/// case-folding volume's other case, an uppercase folder with an escape and
/// a query, a player macro, a bare PDF the reader opens in `assets/`, an
/// `hls__` pointer, an entity-spelled HTML source). The orphan list leaves
/// each out and the trash recheck refuses it; a file nothing names stays
/// unused and trashable.
#[test]
fn an_in_use_attachment_is_never_offered_for_trash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for folder in ["pages", "journals", "assets"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    let page = [
        "- ![x](../assets/Pic.png)",
        "- ![x](../ASSETS/a%20b.png?rev=1)",
        "- {{video ../assets/clip.mp4}}",
        "- [g](guide.pdf)",
        "- <img src=\"../assets/r&#101;al.png\">",
    ];
    std::fs::write(root.join("pages/a.md"), page.join("\n") + "\n").unwrap();
    std::fs::write(
        root.join("pages/hls__report.md"),
        "file-path:: ../ASSETS/my report.pdf\n\n- highlight\n",
    )
    .unwrap();
    let shown = [
        "pic.png",
        "a b.png",
        "clip.mp4",
        "guide.pdf",
        "real.png",
        "my report.pdf",
    ];
    for name in shown.iter().chain(&["unused.png"]) {
        std::fs::write(root.join("assets").join(name), name.as_bytes()).unwrap();
    }
    let (store, _, _) = Store::open(root, Default::default()).unwrap();
    let orphans: Vec<String> = orphan_assets(&store)
        .unwrap()
        .into_iter()
        .map(|asset| asset.name)
        .collect();
    let listed: Vec<&str> = shown
        .iter()
        .copied()
        .filter(|name| orphans.iter().any(|o| o == name))
        .collect();
    assert!(listed.is_empty(), "in use but listed as unused: {listed:?}");
    assert_eq!(orphans, ["unused.png"]);
    for name in shown {
        assert_eq!(
            trash_asset(&store, name).unwrap(),
            TrashOutcome::Referenced,
            "{name}"
        );
        assert!(root.join("assets").join(name).exists(), "{name}");
    }
    assert_eq!(
        trash_asset(&store, "unused.png").unwrap(),
        TrashOutcome::Trashed
    );
    store.close();
}

/// Through the store, the second review's traces: a whitespace-only name the
/// fold empties, `hls__` pointers through the parser (mixed-case Markdown
/// key, Org directive, Org drawer, an extensionless PDF outside `assets/`),
/// an image macro split at U+FEFF, and a graph-file link through a second
/// `assets` segment to an extensionless file. Each stays off the orphan list
/// and the trash recheck refuses it.
#[test]
fn the_second_reviews_in_use_files_are_never_offered_for_trash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for folder in ["pages", "journals", "assets"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    let page = [
        "- [blank](../assets/%20)",
        "- [tab](../assets/%09)",
        "- [nbsp](../assets/%C2%A0)",
        "- [wide](../assets/%E3%80%80)",
        "- {{img ../assets/x.png\u{feff}100}}",
        "- [x](%61ssets/../%61ssets/README)",
    ];
    std::fs::write(root.join("pages/a.md"), page.join("\n") + "\n").unwrap();
    for (file, text) in [
        ("pages/hls__m.md", "File-Path:: ../assets/m.pdf\n\n- h\n"),
        ("pages/hls__r.md", "file-path:: /outside/report\n\n- h\n"),
        ("pages/hls__o.org", "#+File-Path: o.pdf\n\n* h\n"),
        (
            "pages/hls__d.org",
            ":PROPERTIES:\n:file-path: d.pdf\n:END:\n\n* h\n",
        ),
    ] {
        std::fs::write(root.join(file), text).unwrap();
    }
    // Windows cannot hold a file named only a space or a tab, so those two links name no file there.
    let shown: Vec<&str> = [
        " ", "\t", "\u{a0}", "\u{3000}", "x.png", "README", "m.pdf", "report", "o.pdf", "d.pdf",
    ]
    .into_iter()
    .filter(|name| !cfg!(windows) || !matches!(*name, " " | "\t"))
    .collect();
    for name in shown.iter().chain(&["unused.png"]) {
        std::fs::write(root.join("assets").join(name), b"x").unwrap();
    }
    let (store, _, _) = Store::open(root, Default::default()).unwrap();
    let orphans: Vec<String> = orphan_assets(&store)
        .unwrap()
        .into_iter()
        .map(|asset| asset.name)
        .collect();
    assert_eq!(orphans, ["unused.png"], "in use but listed as unused");
    for name in shown {
        assert_eq!(
            trash_asset(&store, name).unwrap(),
            TrashOutcome::Referenced,
            "{name:?}"
        );
        assert!(root.join("assets").join(name).exists(), "{name:?}");
    }
    store.close();
}

/// Orphaned media's trash files an unused asset under the trash's `assets/`
/// folder even when the pages or journals folder is configured as
/// `"assets/"` or `"assets"`, a sync-conflict name included; Settings then counts it as
/// asset trash and Empty asset trash removes it.
#[test]
fn an_asset_trash_files_under_assets_whatever_the_text_folders_are() {
    let photo = "photo.png";
    let copy = "photo.sync-conflict-20240101-000000-ABCDEFG.png";
    // (folder of the trashed copy, Settings' asset count, Empty asset
    // trash's result, entries left in the trash) per case that differs.
    let mut wrong = Vec::new();
    for config in [
        r#"{:pages-directory "assets/"}"#,
        r#"{:journals-directory "assets/"}"#,
        r#"{:pages-directory "assets/" :journals-directory "assets/"}"#,
        r#"{:pages-directory "assets"}"#,
        r#"{:journals-directory "assets"}"#,
        r#"{:pages-directory "assets" :journals-directory "assets"}"#,
        r#"{:pages-directory "assets" :journals-directory "assets/"}"#,
    ] {
        for name in [photo, copy] {
            let root = crate::test_support::fresh_dir("tine-asset-trash-overlap-");
            std::fs::create_dir_all(root.join("logseq")).unwrap();
            std::fs::create_dir_all(root.join("assets")).unwrap();
            std::fs::write(root.join("logseq/config.edn"), config).unwrap();
            std::fs::write(root.join("assets").join(name), b"png-bytes").unwrap();
            let store = Store::open(&root, Default::default()).unwrap().0;
            assert_eq!(
                trash_asset(&store, name).unwrap(),
                TrashOutcome::Trashed,
                "{config} {name}"
            );
            let trash = root.join("logseq/.tine-trash");
            let mut folders = Vec::new();
            for kind in std::fs::read_dir(&trash).unwrap() {
                let kind = kind.unwrap();
                for entry in std::fs::read_dir(kind.path()).unwrap() {
                    let entry = entry.unwrap().file_name().to_string_lossy().into_owned();
                    if entry.ends_with(&format!("__{name}")) {
                        folders.push(kind.file_name().to_string_lossy().into_owned());
                    }
                }
            }
            let stats = store.trash_stats().unwrap();
            let counted = stats
                .iter()
                .find(|(k, _, _)| *k == TrashKind::Asset)
                .map(|s| s.1);
            let purged = store.purge_asset_trash().unwrap();
            let left = std::fs::read_dir(&trash)
                .unwrap()
                .flat_map(|kind| std::fs::read_dir(kind.unwrap().path()).unwrap())
                .count();
            let seen = (folders, counted, purged, left);
            if seen != (vec!["assets".to_owned()], Some(1), (1, 9), 0) {
                wrong.push(format!("{config} {name}: {seen:?}"));
            }
            store.close();
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A note or a sync-conflict copy of one that Orphaned media trashes is never
/// filed under the trash's `assets/` folder, whatever the configured pages and
/// journals folders: it keeps its pages, journals or conflicts folder, and goes
/// to `other/` where its path alone would give `assets/`. Empty asset trash
/// leaves it in the trash, and Settings does not count it as asset trash.
#[test]
fn a_trashed_note_never_becomes_asset_trash() {
    let mut wrong = Vec::new();
    for (pages, journals) in [
        (None, None),
        (Some("assets"), None),
        (None, Some("assets")),
        (Some("assets"), Some("assets")),
        (Some("assets"), Some("assets/")),
        (Some("assets/"), Some("assets")),
        (Some("assets/"), Some("assets/")),
    ] {
        let mut config = String::from("{");
        if let Some(pages) = pages {
            config += &format!(r#":pages-directory "{pages}" "#);
        }
        if let Some(journals) = journals {
            config += &format!(r#":journals-directory "{journals}""#);
        }
        config += "}";
        for (name, conflict) in [
            ("Note.md", false),
            ("Note.markdown", false),
            ("Note.org", false),
            ("Note.MD", false),
            ("Note.ORG", false),
            ("Note (conflicted copy 2026-08-01).md", true),
            ("Note (SFConflict 2026-08-01-10-00-00).md", true),
            ("Note.sync-conflict-20260705-141233-A2B3C4D.md", true),
            ("Note.sync-conflict-20190201-124559.md", true),
        ] {
            // The folder the copy's path gives, `other/` standing in for `assets/`.
            let folder = match (pages == Some("assets"), journals == Some("assets")) {
                (true, _) | (_, true) if conflict => "conflicts",
                (true, _) => "pages",
                (_, true) => "journals",
                _ => "other",
            };
            let root = crate::test_support::fresh_dir("tine-asset-trash-text-");
            std::fs::create_dir_all(root.join("logseq")).unwrap();
            std::fs::create_dir_all(root.join("assets")).unwrap();
            std::fs::write(root.join("logseq/config.edn"), &config).unwrap();
            std::fs::write(root.join("assets").join(name), b"- note").unwrap();
            let store = Store::open(&root, Default::default()).unwrap().0;
            assert_eq!(
                trash_asset(&store, name).unwrap(),
                TrashOutcome::Trashed,
                "{config} {name}"
            );
            let counted = store
                .trash_stats()
                .unwrap()
                .iter()
                .find(|(k, _, _)| *k == TrashKind::Asset)
                .map(|s| s.1);
            store.purge_asset_trash().unwrap();
            let trash = root.join("logseq/.tine-trash");
            let mut kept = Vec::new();
            for kind in std::fs::read_dir(&trash).unwrap() {
                let kind = kind.unwrap();
                for entry in std::fs::read_dir(kind.path()).unwrap() {
                    let entry = entry.unwrap();
                    if entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(&format!("__{name}"))
                    {
                        assert_eq!(std::fs::read(entry.path()).unwrap(), b"- note");
                        kept.push(kind.file_name().to_string_lossy().into_owned());
                    }
                }
            }
            let seen = (kept, counted.unwrap_or(0));
            if seen != (vec![folder.to_owned()], 0) {
                wrong.push(format!("{config} {name}: {seen:?}"));
            }
            store.close();
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
