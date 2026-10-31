//! GH #811: on the reporter's Android phone every page save of 0.9.0-beta.1
//! ended Uncertain (diagnostics: published 0, uncertain 6), so creating today's
//! journal from the template never published and the Journals feed failed.
//! These cases passed on the API 33 and 35 emulators (CI run 41017745975), so
//! the device's cause is still open; round 2 adds API 36 (Android 16, the
//! reporter's), repeated creates and replacements, and a sync client working
//! in the same folder (the reporter syncs the graph with Syncthing). These cases put the graph on the volume named by
//! `TINE_HOST_FS_ROOT` (ci.yml android-git-storage runs them on the emulator's
//! shared storage) and the app data beside the test binary's temp dir, as on a
//! phone. Without the variable they run on the ordinary temp dir.
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// What a sync client does in the graph while Tine saves (GH #811 round 2,
/// Syncthing on the reporter's tablet): its scanner lists the folder, stats
/// every file and reads the changed ones; its puller delivers another file
/// as a hidden temporary renamed into place.
fn sync_client(dir: &Path, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<u64> {
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        let mut passes = 0;
        let mut seen = std::collections::HashMap::new();
        while !stop.load(Ordering::Relaxed) {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let Ok(metadata) = fs::symlink_metadata(&path) else {
                    continue;
                };
                let stamp = describe(&metadata);
                if seen.insert(path.clone(), stamp.clone()) != Some(stamp) {
                    let _ = fs::read(&path);
                }
            }
            let delivered = dir.join(".syncthing.delivered.md.tmp");
            if fs::write(&delivered, format!("- delivery {passes}\n")).is_ok() {
                let _ = fs::rename(&delivered, dir.join("delivered.md"));
            }
            passes += 1;
            std::thread::yield_now();
        }
        passes
    })
}

/// One Tine-shaped publication with raw syscalls: hidden temporary written,
/// fsynced and closed, renamed over (or onto) `target`, the handle's stamp
/// read after the rename, the folder synced, then the path's stamp. Returns
/// the two stamps and the stamps 100 ms and 1 s later.
fn publish(target: &Path, bytes: &[u8]) -> [String; 4] {
    let temporary = target.with_file_name(format!(
        ".{}.tine-tmp",
        target.file_name().unwrap().to_string_lossy()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);
    fs::rename(&temporary, target).unwrap();
    let mut handle = fs::File::open(target).unwrap();
    let opened = handle.metadata().unwrap();
    std::io::Read::read_to_end(&mut handle, &mut Vec::new()).unwrap();
    drop(handle);
    // diagnostic only: Windows cannot open a folder without backup semantics
    let _ = fs::File::open(target.parent().unwrap()).and_then(|d| d.sync_all());
    let after = |wait: u64| {
        std::thread::sleep(Duration::from_millis(wait));
        fs::symlink_metadata(target).map_or_else(|e| format!("lstat failed: {e}"), |m| describe(&m))
    };
    [describe(&opened), after(0), after(100), after(900)]
}

/// The premise of the post-sync publication check (M3-1): a file's metadata
/// read through its open handle right after publication equals a later
/// path-based read when nothing touched the file. Raw syscalls only, so a
/// failure names the filesystem, not Tine. Creates and replacements, alone
/// and beside a sync client working in the same folder.
#[test]
fn a_published_files_handle_and_path_metadata_agree_on_the_graph_volume() {
    let _integration_guard = crate::integration_lock();
    let dir = volume();
    let mut mismatches = Vec::new();
    for client in [false, true] {
        let stop = Arc::new(AtomicBool::new(false));
        let worker = client.then(|| sync_client(dir.path(), stop.clone()));
        for round in 0..10 {
            for (kind, name) in [
                ("create", format!("c{round}-{client}.md")),
                ("replace", "r.md".into()),
            ] {
                let target = dir.path().join(name);
                let [opened, path, later, last] =
                    publish(&target, format!("- {kind} {round}\n").as_bytes());
                let line = format!(
                    "client={client} {kind} round={round}\n  handle: {opened}\n  path:   {path}\n  +100ms: {later}\n  +1s:    {last}"
                );
                if round == 0 || opened != path || path != later || later != last {
                    println!("GH811-STAMP {line}");
                }
                if opened != path {
                    mismatches.push(line);
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
        if let Some(worker) = worker {
            println!("GH811-STAMP sync client passes: {}", worker.join().unwrap());
        }
    }
    assert!(
        mismatches.is_empty(),
        "handle and path metadata differ:\n{}",
        mismatches.join("\n")
    );
}

fn graph(root: &Path) {
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("pages/P.md"), "- a page\n").unwrap();
}

/// The app's journal-template create (graph.ts ensureJournalTemplateForDay →
/// createPage): open today's absent journal on the page host, submit the
/// template's text, and wait until it is published; then the user's next
/// edits of that page, each waited for. With `client`, a sync client works in
/// the journals folder meanwhile (the reporter's Syncthing).
fn todays_journal_saves(client: bool) {
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
    let stop = Arc::new(AtomicBool::new(false));
    let worker = client.then(|| sync_client(&root.join("journals"), stop.clone()));
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
    let mut id = reloaded["nextId"].as_u64().unwrap();
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
    let mut version = answer(id)["answer"]["version"].as_u64().unwrap();
    let mut outcomes = Vec::new();
    for edit in 0..6 {
        let raw = if edit == 0 {
            "from the daily template".to_string()
        } else {
            format!("from the daily template, edit {edit}")
        };
        let doc = PageDto {
            name: stem.clone(),
            kind: PageKind::Journal,
            title: today.title(),
            pre_block: None,
            blocks: vec![BlockDto {
                id: "b".into(),
                raw,
                ..Default::default()
            }],
            rev: None,
            format: Format::Md,
            read_only: false,
            guide: false,
        };
        let kinds = if edit == 0 {
            [EditKind::CreatePage]
        } else {
            [EditKind::SaveBlock]
        };
        id += 1;
        host.submit(session, id, &key, &doc, version, None, &kinds)
            .unwrap();
        version = answer(id)["answer"]["version"].as_u64().unwrap();
        outcomes.push(host.wait_published(
            session,
            &[(key.clone(), version, None)],
            Duration::from_secs(10),
        ));
    }
    stop.store(true, Ordering::Relaxed);
    if let Some(worker) = worker {
        println!("GH811-SAVE sync client passes: {}", worker.join().unwrap());
    }
    seen.extend(mail.try_iter());
    let storage = store.diagnostics()["storage"].clone();
    println!("GH811-SAVE client={client} published={outcomes:?} storage={storage}");
    for message in &seen {
        println!("GH811-SAVE mail {message}");
    }
    assert!(
        outcomes.iter().all(|o| *o == PageOperation::Applied),
        "published"
    );
    assert_eq!(storage["pageSaves"]["uncertain"], 0, "no uncertain save");
    assert!(fs::read_to_string(root.join(&rel))
        .unwrap()
        .contains("from the daily template, edit 5"));
    drop(host);
    store.close();
}

#[test]
fn todays_journal_created_on_the_host_publishes_on_the_graph_volume() {
    todays_journal_saves(false);
}

#[test]
fn todays_journal_saves_publish_beside_a_sync_client_on_the_graph_volume() {
    todays_journal_saves(true);
}
