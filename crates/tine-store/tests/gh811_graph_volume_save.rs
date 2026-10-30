//! GH #811: on the reporter's Android phone every page save of 0.9.0-beta.1
//! ended Uncertain (diagnostics: published 0, uncertain 6), so creating today's
//! journal from the template never published and the Journals feed failed.
//! These cases passed on the API 33 and 35 emulators (CI run 41017745975), so
//! the device's cause is still open. These cases put the graph on the volume named by
//! `TINE_HOST_FS_ROOT` (ci.yml android-git-storage runs them on the emulator's
//! shared storage) and the app data beside the test binary's temp dir, as on a
//! phone. Without the variable they run on the ordinary temp dir.
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use tine_core::date::JournalDate;
use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::{EditKind, PageHost, PageId, PageOperation, Store};

fn volume() -> tempfile::TempDir {
    match std::env::var_os("TINE_HOST_FS_ROOT") {
        Some(base) => tempfile::tempdir_in(base).unwrap(),
        None => tempfile::tempdir().unwrap(),
    }
}

#[cfg(unix)]
fn describe(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        "dev={} ino={} len={} mtime={}.{:09} ctime={}.{:09}",
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec()
    )
}

#[cfg(not(unix))]
fn describe(metadata: &fs::Metadata) -> String {
    format!(
        "len={} modified={:?}",
        metadata.len(),
        metadata.modified().ok()
    )
}

/// The premise of the post-sync publication check (M3-1): a file's metadata
/// read through its open handle right after publication equals a later
/// path-based read when nothing touched the file. Raw syscalls only, so a
/// failure names the filesystem, not Tine.
#[test]
fn a_published_files_handle_and_path_metadata_agree_on_the_graph_volume() {
    let _integration_guard = crate::integration_lock();
    let dir = volume();
    let target = dir.path().join("2026_10_30.md");
    let temporary = dir.path().join(".2026_10_30.md.tmp");
    let mut file = fs::File::create(&temporary).unwrap();
    file.write_all(b"- from the daily template\n").unwrap();
    file.sync_all().unwrap();
    drop(file);
    fs::rename(&temporary, &target).unwrap();
    let handle = fs::File::open(&target).unwrap().metadata().unwrap();
    // diagnostic only: Windows cannot open a folder without backup semantics
    let directory = fs::File::open(dir.path()).and_then(|d| d.sync_all());
    let path = fs::symlink_metadata(&target).unwrap();
    let (handle, path) = (describe(&handle), describe(&path));
    println!("GH811-STAMP handle: {handle}");
    println!("GH811-STAMP path:   {path}");
    println!("GH811-STAMP directory sync: {directory:?}");
    assert_eq!(handle, path, "handle and path metadata differ");
}

fn graph(root: &Path) {
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("pages/P.md"), "- a page\n").unwrap();
}

/// The app's journal-template create (graph.ts ensureJournalTemplateForDay →
/// createPage): open today's absent journal on the page host, submit the
/// template's text, and wait until it is published.
#[test]
fn todays_journal_created_on_the_host_publishes_on_the_graph_volume() {
    let _integration_guard = crate::integration_lock();
    let volume = volume();
    let app = tempfile::tempdir().unwrap();
    let root = volume.path().join("graph");
    graph(&root);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let (sink, mail) = mpsc::channel();
    let send = move |m| {
        let _ = sink.send(serde_json::to_value(m).unwrap());
    };
    let host = PageHost::start(&store, app.path(), "gh811", |_, _| {}, send).unwrap();
    let mut seen = Vec::new();
    let mut answer = |id: u64| loop {
        let value = mail.recv_timeout(Duration::from_secs(10)).unwrap();
        if value["answer"]["id"] == id {
            return value;
        }
        seen.push(value);
    };
    let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
    let session = reloaded["session"].as_u64().unwrap();
    let id = reloaded["nextId"].as_u64().unwrap();
    let today = JournalDate::today();
    let stem = today.file_stem();
    let rel = format!("journals/{stem}.md");
    let opened = host
        .open(session, id, &PageId::from(rel.as_str()), &today.title())
        .unwrap();
    let key = serde_json::to_value(opened).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let version = answer(id)["answer"]["version"].as_u64().unwrap();
    let doc = PageDto {
        name: stem.clone(),
        kind: PageKind::Journal,
        title: today.title(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b".into(),
            raw: "from the daily template".into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        guide: false,
    };
    host.submit(
        session,
        id + 1,
        &key,
        &doc,
        version,
        None,
        &[EditKind::CreatePage],
    )
    .unwrap();
    let version = answer(id + 1)["answer"]["version"].as_u64().unwrap();
    let published = host.wait_published(session, &[(key, version, None)], Duration::from_secs(10));
    seen.extend(mail.try_iter());
    let storage = store.diagnostics()["storage"].clone();
    println!("GH811-SAVE published={published:?} storage={storage}");
    for message in &seen {
        println!("GH811-SAVE mail {message}");
    }
    assert_eq!(published, PageOperation::Applied, "published");
    assert_eq!(storage["pageSaves"]["uncertain"], 0, "no uncertain save");
    assert!(fs::read_to_string(root.join(&rel))
        .unwrap()
        .contains("from the daily template"));
    drop(host);
    store.close();
}
