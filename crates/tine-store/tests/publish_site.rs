use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tine_store::{OpenOptions, Store};

#[test]
fn staged_writer_publishes_files_and_retires_previous_site() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("tine-site-writer-{}-{unique}", std::process::id()));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let (store, _, _) = Store::open(&root, OpenOptions::default()).unwrap();

    let first = store
        .publish_site(&mut |writer| {
            writer.write("index.html", b"old")?;
            writer.write("assets/app.js", b"script")
        })
        .unwrap();
    assert_eq!(first.files, 2);
    assert_eq!(fs::read(root.join("publish/index.html")).unwrap(), b"old");
    assert_eq!(
        fs::read(root.join("publish/assets/app.js")).unwrap(),
        b"script"
    );

    let second = store
        .publish_site(&mut |writer| writer.write("index.html", b"new"))
        .unwrap();
    assert_eq!(second.files, 1);
    assert_eq!(fs::read(root.join("publish/index.html")).unwrap(), b"new");
    assert!(!root.join("publish/assets/app.js").exists());

    let failure = store.publish_site(&mut |writer| writer.write("../outside", b"bad"));
    assert!(failure.is_err());
    assert!(!root.join("outside").exists());
    assert_eq!(fs::read(root.join("publish/index.html")).unwrap(), b"new");

    store.close();
    fs::remove_dir_all(root).unwrap();
}

// The asset-reference scanner over-collects (orphan detection must not miss a
// reference), so prose after `assets/` becomes a candidate name. A candidate
// longer than a file name may be must be skipped like a missing asset, not
// fail the whole publication (the Guide's own `assets/` sentence did).
#[test]
fn prose_after_assets_prefix_does_not_fail_publication_assets() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("assets/pic.png"), b"png").unwrap();
    let prose = "x".repeat(300);
    fs::write(
        root.join("pages/a.md"),
        format!("- Files in `assets/ {prose}` are watched.\n- ![p](../assets/pic.png)\n"),
    )
    .unwrap();
    let (store, _, _) = Store::open(root, OpenOptions::default()).unwrap();
    let corpus = store.whole_graph().unwrap().corpus();
    let assets = tine_store::publication_assets(&store, &corpus).unwrap();
    let names: Vec<_> = assets.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["assets/pic.png"]);
    store.close();
}
