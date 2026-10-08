use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tine_graph_features::sources::graph_source_files;
use tine_store::{OpenOptions, Store};

#[test]
fn parser_sources_keep_path_order_and_size_limit() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("tine-sources-{}-{unique}", std::process::id()));
    fs::create_dir_all(root.join("pages/nested")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("pages/nested/B.org"), b"* B\n").unwrap();
    fs::write(root.join("pages/A.md"), b"- A\n").unwrap();
    fs::write(root.join("journals/2026_09_25.md"), b"- day\n").unwrap();
    fs::write(
        root.join("pages/too-large.md"),
        vec![b'x'; 8 * 1024 * 1024 + 1],
    )
    .unwrap();
    fs::write(root.join("pages/ignored.txt"), b"ignored").unwrap();
    let (store, _, _) = Store::open(&root, OpenOptions::default()).unwrap();

    let sources = graph_source_files(&store, false).unwrap();
    assert!(
        sources
            .skipped
            .iter()
            .any(|row| row.starts_with("pages/too-large.md: ")),
        "the size-limited file is reported, not silently dropped: {:?}",
        sources.skipped
    );
    let pages = sources.files;
    assert_eq!(
        pages
            .iter()
            .map(|file| file.rel.as_str())
            .collect::<Vec<_>>(),
        ["pages/A.md", "pages/nested/B.org"]
    );
    assert_eq!(pages[0].text, "- A\n");
    assert_eq!(pages[0].bytes, 4);
    assert_eq!(pages[1].format, "org");
    let all = graph_source_files(&store, true).unwrap().files;
    assert_eq!(
        all.iter().map(|file| file.rel.as_str()).collect::<Vec<_>>(),
        ["journals/2026_09_25.md", "pages/A.md", "pages/nested/B.org"]
    );

    store.close();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn parser_sources_do_not_follow_links() {
    use std::os::unix::fs::symlink;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("tine-source-links-{}-{unique}", std::process::id()));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("pages/real.md"), b"- real\n").unwrap();
    symlink("real.md", root.join("pages/link.md")).unwrap();
    let (store, _, _) = Store::open(&root, OpenOptions::default()).unwrap();
    let files = graph_source_files(&store, false).unwrap().files;
    assert_eq!(
        files
            .iter()
            .map(|file| file.rel.as_str())
            .collect::<Vec<_>>(),
        ["pages/real.md"]
    );
    store.close();
    fs::remove_dir_all(root).unwrap();
}

/// I-22/I-12: every file `is_graph_text` accepts (`.markdown`, upper-case
/// extensions) has a format answer; imported or synced graphs carry them.
#[test]
fn parser_sources_accept_every_graph_text_extension() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("tine-source-ext-{}-{unique}", std::process::id()));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/a.markdown"), b"- a\n").unwrap();
    fs::write(root.join("pages/B.MD"), b"- b\n").unwrap();
    fs::write(root.join("pages/C.ORG"), b"* c\n").unwrap();
    let (store, _, _) = Store::open(&root, OpenOptions::default()).unwrap();
    let files = graph_source_files(&store, false).unwrap().files;
    let formats: Vec<_> = files
        .iter()
        .map(|file| (file.rel.as_str(), file.format.as_str()))
        .collect();
    assert_eq!(
        formats,
        [
            ("pages/B.MD", "md"),
            ("pages/C.ORG", "org"),
            ("pages/a.markdown", "md")
        ]
    );
    store.close();
    fs::remove_dir_all(root).unwrap();
}
