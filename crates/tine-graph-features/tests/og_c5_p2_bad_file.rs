//! og checkpoint 5, packet P2: one bad file never aborts a whole list
//! (REG-OG-C5-P2-BAD-FILE). The journal feed, the duplicate-day list and the
//! conflict inventory serve every readable file and report the rest; the
//! parser-comparison sources report what they skipped.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::{conflicts, journals};
use tine_store::{FileId, Store};

fn fixture(label: &str, files: &[(&str, &[u8])]) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-og-c5-p2-bad-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "assets"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, body) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    (root, store)
}

/// A directory entry whose name is not UTF-8 (in-scope: a sync service or an
/// external tool delivering a foreign-encoded filename).
#[cfg(unix)]
fn write_non_utf8_name(dir: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::OsStr::from_bytes(b"bad-\xff-name.md");
    fs::write(dir.join(name), "- unreachable\n").unwrap();
}

const BAD_UTF8: &[u8] = b"- bad \xff\n";

#[test]
fn journal_feed_serves_readable_days_and_reports_the_bad_one() {
    let (root, store) = fixture(
        "feed",
        &[
            ("journals/2026_01_01.md", b"- day one\n"),
            ("journals/2026_01_02.md", BAD_UTF8),
            ("journals/2026_01_03.md", b"- day three\n"),
        ],
    );
    #[cfg(unix)]
    write_non_utf8_name(&root.join("journals"));
    store.scan_refresh().unwrap();
    let feed = journals::feed_page(&store, 10, None).expect("one bad journal blocked the feed");
    let names: Vec<&str> = feed.pages.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        names,
        ["journals/2026_01_03.md", "journals/2026_01_01.md"],
        "readable days are served in order"
    );
    assert!(feed.done);
    assert!(
        feed.unreadable
            .iter()
            .any(|row| row.starts_with("journals/2026_01_02.md")),
        "the bad day is reported: {:?}",
        feed.unreadable
    );
    #[cfg(unix)]
    assert!(
        feed.unreadable.iter().any(|row| row.contains("bad-")),
        "the undecodable name is reported: {:?}",
        feed.unreadable
    );
    // Paging across the bad day keeps the cursor moving.
    let first = journals::feed_page(&store, 1, None).unwrap();
    let rest = journals::feed_page(&store, 1, first.next_before_day).unwrap();
    assert_eq!(rest.pages[0].id.as_str(), "journals/2026_01_01.md");
    assert!(rest.done);
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_days_list_every_day_and_name_the_unreadable_file() {
    let (root, store) = fixture(
        "duplicates",
        &[
            ("journals/2026_06_19.md", b"- good\n"),
            ("journals/2026_06_19.org", BAD_UTF8),
            ("journals/2026_06_20.md", b"- twin md\n"),
            ("journals/2026_06_20.org", b"* twin org\n"),
        ],
    );
    #[cfg(unix)]
    write_non_utf8_name(&root.join("journals"));
    store.scan_refresh().unwrap();
    let days = journals::journal_conflicts(&store).expect("one bad preview blocked the list");
    assert_eq!(days.len(), 2, "both duplicate days are listed");
    let files: Vec<_> = days.iter().flat_map(|day| day.files.iter()).collect();
    let bad = files
        .iter()
        .find(|file| file.path == "journals/2026_06_19.org")
        .unwrap();
    assert!(
        bad.preview_error.is_some(),
        "the unreadable file says so instead of an empty preview"
    );
    let good = files
        .iter()
        .find(|file| file.path == "journals/2026_06_20.org")
        .unwrap();
    assert_eq!(good.preview, "twin org");
    assert!(good.preview_error.is_none());
    store.close();
    let _ = fs::remove_dir_all(root);
}

const COPY: &str = "pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md";

#[test]
fn conflict_inventory_keeps_healthy_conflicts_beside_bad_files() {
    let big = {
        let mut bytes = b"<<<<<<< HEAD\n".to_vec();
        bytes.resize(tine_store::PARSE_INPUT_MAX_BYTES as usize + 16, b'x');
        bytes
    };
    let (root, store) = fixture(
        "inventory",
        &[
            ("pages/Foo.md", b"- mine\n"),
            (COPY, b"- theirs\n"),
            (
                "pages/Marked.md",
                b"<<<<<<< HEAD\n- bad \xff\n=======\n- other\n>>>>>>> branch\n",
            ),
            ("pages/Big.md", &big),
            ("journals/2026_06_19.md", b"- good\n"),
            ("journals/2026_06_19.org", BAD_UTF8),
        ],
    );
    #[cfg(unix)]
    write_non_utf8_name(&root.join("pages"));
    store.scan_refresh().unwrap();
    let inventory =
        conflicts::conflict_inventory(&store).expect("one bad page blocked the conflict inventory");
    let copy_id = format!("copy:{COPY}");
    assert!(
        inventory.queue.iter().any(|object| object.id == copy_id),
        "the healthy sync copy stays reviewable: {:?}",
        inventory.queue.iter().map(|o| &o.id).collect::<Vec<_>>()
    );
    for bad in ["pages/Marked.md", "pages/Big.md", "journals/2026_06_19.org"] {
        assert!(
            inventory.unreadable.iter().any(|row| row.starts_with(bad)),
            "{bad} is reported: {:?}",
            inventory.unreadable
        );
    }
    #[cfg(unix)]
    assert!(inventory.unreadable.iter().any(|row| row.contains("bad-")));

    let queue = conflicts::ConflictQueue::default();
    let cached = queue.inventory(&store).expect("the cached queue builds");
    assert!(cached.queue.iter().any(|object| object.id == copy_id));
    // A bad file arriving later is reported by the incremental refresh too,
    // and the healthy conflict stays.
    fs::write(
        root.join("pages/Late.md"),
        b"<<<<<<< HEAD\n\xff\n>>>>>>> x\n",
    )
    .unwrap();
    queue
        .refresh_files(&store, &[FileId::from("pages/Late.md".to_owned())])
        .expect("one bad changed file blocked the refresh");
    let refreshed = queue.inventory(&store).unwrap();
    assert!(refreshed.queue.iter().any(|object| object.id == copy_id));
    assert!(refreshed
        .unreadable
        .iter()
        .any(|row| row.starts_with("pages/Late.md")));
    // Repaired, it leaves the report.
    fs::write(root.join("pages/Late.md"), b"- fine\n").unwrap();
    queue
        .refresh_files(&store, &[FileId::from("pages/Late.md".to_owned())])
        .unwrap();
    assert!(!queue
        .inventory(&store)
        .unwrap()
        .unreadable
        .iter()
        .any(|row| row.starts_with("pages/Late.md")));
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parser_sources_report_what_they_skip() {
    let (root, store) = fixture(
        "sources",
        &[("pages/Good.md", b"- good\n"), ("pages/Bad.md", BAD_UTF8)],
    );
    let sources = tine_graph_features::sources::graph_source_files(&store, false).unwrap();
    assert_eq!(
        sources
            .files
            .iter()
            .map(|f| f.rel.as_str())
            .collect::<Vec<_>>(),
        ["pages/Good.md"]
    );
    assert!(
        sources
            .skipped
            .iter()
            .any(|row| row.starts_with("pages/Bad.md")),
        "{:?}",
        sources.skipped
    );
    store.close();
    let _ = fs::remove_dir_all(root);
}
