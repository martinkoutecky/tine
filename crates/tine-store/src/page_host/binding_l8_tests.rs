use super::*;

// Audit 2026-10-22 L8-01: production-boundary regressions.
// In-scope honest event under test: a sync client or the user's file manager
// moves `pages/sub` elsewhere and leaves a directory symlink in its place
// while a save is queued (or while Tine is stopped holding a draft). The
// store refuses a write through a symlinked folder at admission
// (`a_write_through_a_symlinked_folder_is_refused`); these ask whether the
// later host I/O re-checks it.

/// Every file under `dir` (recursively) with its bytes.
#[cfg(unix)]
fn l8_files(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(l8_files(&path));
        } else {
            out.push((
                path.display().to_string(),
                fs::read_to_string(&path).unwrap_or_default(),
            ));
        }
    }
    out.sort();
    out
}

#[cfg(unix)]
fn l8_retarget(root: &std::path::Path, outside: &std::path::Path) {
    fs::rename(root.join("pages/sub"), outside.join("sub")).unwrap();
    std::os::unix::fs::symlink(outside.join("sub"), root.join("pages/sub")).unwrap();
}

fn l8_retained(host: &PageHost, key: &str, expected: &str) {
    let key = key.to_owned();
    let expected = expected.as_bytes().to_vec();
    host.driver
        .shared
        .read(move |state| {
            let host = &state.progress.host;
            assert_eq!(host.pages[&key].buf.as_deref(), Some(expected.as_slice()));
            assert!(!host.pages[&key].clean());
        })
        .unwrap();
}

#[cfg(unix)]
#[test]
fn l8_01_a_queued_save_does_not_follow_a_retargeted_folder() {
    let mut escaped = Vec::new();
    for watch in [crate::WatchMode::Notify, crate::WatchMode::Poll] {
        let live = Live::watching(&[("pages/sub/a.md", "- one\n")], watch);
        let outside = tempfile::tempdir().unwrap();
        let (key, page) = live.open("pages/sub/a.md");
        live.pause(true);
        let id = live.submit(&key, "- typed\n", page.version, None).unwrap();
        l8_retarget(&live.root, outside.path());
        live.pause(false);
        let answer = live.answer(&key, id);
        let target = outside.path().join("sub/a.md");
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::read_to_string(&target).unwrap() == "- one\n" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let files = l8_files(outside.path());
        eprintln!(
            "L8-01 queued save ({watch:?}): answer {:?}; outside after: {files:?}; unsaved {:?}",
            answer.answer,
            live.host.draft_status()
        );
        l8_retained(&live.host, &key, "- typed\n");
        live.until_recoverable(&[(key.clone(), answer.answer.as_ref().unwrap().version)]);
        assert!(live.host.draft_status().unsaved.iter().any(|p| p.failing));
        live.host.stop();
        if files != vec![(target.display().to_string(), "- one\n".to_owned())] {
            escaped.push(format!("{watch:?}: {files:?}"));
        }
    }
    assert!(
        escaped.is_empty(),
        "the save left the graph through the retargeted folder: {escaped:?}"
    );
}

#[cfg(unix)]
#[test]
fn l8_01_a_queued_delete_does_not_follow_a_retargeted_folder() {
    let live = Live::new(&[("pages/sub/a.md", "- one\n")]);
    let outside = tempfile::tempdir().unwrap();
    live.pause(true);
    let result = std::thread::scope(|scope| {
        let host = &live.host;
        let applied = scope.spawn(move || {
            host.apply(op(
                vec![(
                    "pages/sub/a.md",
                    Effect::Delete,
                    Expected::Bytes(bytes("- one\n")),
                )],
                Input::Refuse,
            ))
        });
        std::thread::sleep(Duration::from_millis(200));
        let pending = !applied.is_finished();
        l8_retarget(&live.root, outside.path());
        live.pause(false);
        (pending, applied.join().unwrap())
    });
    let files = l8_files(outside.path());
    eprintln!(
        "L8-01 queued delete: pending at retarget {}; apply {:?}; outside after: {files:?}; graph trash holds it: {}",
        result.0,
        result.1,
        trashed(&live, "- one\n")
    );
    live.host.stop();
    assert!(
        result.0,
        "the delete was not still queued when the folder moved"
    );
    assert!(result.1.is_err(), "the queued deletion must refuse");
    assert_eq!(
        files,
        vec![(
            outside.path().join("sub/a.md").display().to_string(),
            "- one\n".to_owned()
        )],
        "the delete removed the outside file through the retargeted folder"
    );
}

/// Tine stopped holding a durable draft of `pages/sub/a.md`; the folder is
/// reorganized while it is stopped; relaunch (fresh store) recovers the
/// draft and saves it.
#[cfg(unix)]
#[test]
fn l8_01_a_recovered_draft_does_not_follow_a_folder_retargeted_while_stopped() {
    use crate::page_host::drafts;
    let dir = tempfile::tempdir().unwrap();
    let graph = dir.path().join("graph");
    fs::create_dir_all(graph.join("pages/sub")).unwrap();
    fs::create_dir_all(graph.join("journals")).unwrap();
    fs::write(graph.join("pages/sub/a.md"), "- one\n").unwrap();
    let app = dir.path().join("app");
    let drafts_dir = app.join("drafts-v2/test-graph");
    fs::create_dir_all(&drafts_dir).unwrap();
    let record = Record {
        page: "pages/sub/a.md".into(),
        wseq: 1,
        version: 1,
        base: Base::Known(Some(Arc::from(b"- one\n".as_slice()))),
        bytes: Some(Arc::from(b"- recovered\n".as_slice())),
    };
    fs::write(
        drafts_dir.join(drafts::page_name("pages/sub/a.md")),
        drafts::encode(&[record]),
    )
    .unwrap();
    let outside = tempfile::tempdir().unwrap();
    l8_retarget(&graph, outside.path());
    let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
    let view = store.whole_graph_reconciled().unwrap();
    let mut listed: Vec<String> = (view.graph.pages.slots())
        .map(|(_, (entry, _))| format!("{:?}", entry.rel_path))
        .collect();
    listed.sort();
    eprintln!("L8-01 recovered draft: cold store lists {listed:?}");
    drop(view);
    let host = PageHost::start(&store, &app, "test-graph", |_, _| {}, |_| {}).unwrap();
    let target = outside.path().join("sub/a.md");
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(&target).unwrap() == "- one\n" && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let files = l8_files(outside.path());
    eprintln!(
        "L8-01 recovered draft: outside after: {files:?}; status {:?}",
        host.draft_status()
    );
    l8_retained(&host, "pages/sub/a.md", "- recovered\n");
    assert!(host
        .draft_status()
        .unsaved
        .iter()
        .any(|p| p.recovered && p.failing));
    assert!(crate::page_host::drafts::scan(
        fs::read_dir(&drafts_dir)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                Some((
                    path.file_name()?.to_str()?.to_owned(),
                    fs::read(&path).ok()?,
                ))
            })
            .collect()
    )
    .logical
    .values()
    .any(|r| r.bytes.as_deref() == Some(b"- recovered\n".as_slice())));
    host.stop();
    assert_eq!(
        files,
        vec![(target.display().to_string(), "- one\n".to_owned())],
        "the recovered draft was saved outside the graph"
    );
}

/// S3 B2: resolving a moved leaf must not resolve a case-variant folder
/// retargeted while stopped. The original parent guard remains authoritative.
#[cfg(unix)]
#[test]
fn recovered_leaf_alias_does_not_follow_a_retargeted_case_variant_parent() {
    let dir = tempfile::tempdir().unwrap();
    let graph = dir.path().join("graph");
    fs::create_dir_all(graph.join("pages/Sub")).unwrap();
    fs::write(graph.join("pages/Sub/a.md"), "- original\n").unwrap();
    // On a folding volume sub is already the real directory's alias; only
    // case-sensitive volumes can construct this distinct directory symlink.
    if graph.join("pages/sub").exists() {
        return;
    }
    std::os::unix::fs::symlink("Sub", graph.join("pages/sub")).unwrap();
    let app = dir.path().join("app");
    let drafts_dir = app.join("drafts-v2/test-graph");
    fs::create_dir_all(&drafts_dir).unwrap();
    let record = Record {
        page: "pages/sub/a.md".into(),
        wseq: 1,
        version: 1,
        base: Base::Known(Some(bytes("- original\n"))),
        bytes: Some(bytes("- recovered\n")),
    };
    fs::write(
        drafts_dir.join(crate::page_host::drafts::page_name(&record.page)),
        crate::page_host::drafts::encode(&[record]),
    )
    .unwrap();
    let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
    assert_eq!(
        store.disk_spelling(&PageId::from("pages/sub/a.md")),
        Some(PageId::from("pages/Sub/a.md"))
    );
    let host = PageHost::start_for_tests(&store, &app).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if host
            .draft_status()
            .unsaved
            .iter()
            .any(|page| page.recovered && page.failing)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "recovery did not report its refused target"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        host.spelling("pages/sub/a.md"),
        PageId::from("pages/sub/a.md")
    );
    assert_eq!(
        fs::read(graph.join("pages/Sub/a.md")).unwrap(),
        b"- original\n"
    );
    l8_retained(&host, "pages/sub/a.md", "- recovered\n");
    host.stop();
    store.close();
}

// Audit 2026-10-22 L8-02: a late owner at the creating save.
// In-scope honest event: sync or an external editor delivers a file that
// claims the name of a page the user has open but not yet created.

/// What the first save of the open absent page `pages/A.md` does after
/// `arrival` lands (and the watcher publishes it) between open and submit.
fn l8_02_first_save_after(arrival: (&str, &[u8]), refuses: bool) -> (bool, Vec<u8>, String) {
    let live = Live::watching(&[("pages/other.md", "- x\n")], crate::WatchMode::Notify);
    let (key, page) = live.open("pages/A.md");
    fs::write(live.root.join(arrival.0), arrival.1).unwrap();
    assert!(live
        .store
        .watcher_drained_for_tests(Duration::from_secs(20)));
    let id = live.submit(&key, "- typed\n", page.version, None);
    let answer = id.map(|id| live.answer(&key, id).answer);
    let created = live.root.join("pages/A.md");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !created.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    let exists = created.exists();
    if refuses {
        l8_retained(&live.host, &key, "- typed\n");
        live.until_recoverable(&[(
            key.clone(),
            answer.as_ref().unwrap().as_ref().unwrap().version,
        )]);
        let refusal = live.wait(&key, "creating save names its owner", |mail| {
            mail.notice.twin.as_deref() == Some(arrival.0) && mail.notice.save_error
        });
        assert!(refusal.notice.save_error);
    }
    let other = fs::read(live.root.join(arrival.0)).unwrap();
    let report = format!(
        "submit {answer:?}; A.md created {exists}; status {:?}; resolve(A) {:?}",
        live.host.draft_status(),
        live.store
            .whole_graph()
            .ok()
            .map(|view| match view.resolve("A", false) {
                crate::Resolved::Existing { id, others } => format!(
                    "existing {} others {:?}",
                    id.as_str(),
                    others.iter().map(PageId::as_str).collect::<Vec<_>>()
                ),
                crate::Resolved::Alias { .. } => "alias".to_owned(),
                crate::Resolved::Absent { id } => format!("absent {}", id.as_str()),
            })
    );
    live.host.stop();
    (exists, other, report)
}

/// Control: the claimant present at open refuses the open itself.
#[test]
fn l8_02_control_open_refuses_a_name_already_claimed() {
    let live = Live::watching(
        &[("pages/Elsewhere.md", "title:: A\n\n- claim\n")],
        crate::WatchMode::Notify,
    );
    let refused = live
        .host
        .open(live.host.session(), 1, &PageId::from("pages/A.md"), "A");
    eprintln!(
        "L8-02 control: open with claimant present -> {:?}",
        refused.as_ref().map(|o| &o.key)
    );
    assert!(refused.is_err(), "open of a claimed name was admitted");
    live.host.stop();
}

#[test]
fn l8_02_first_save_does_not_create_a_second_owner_of_a_title_claimed_name() {
    let body = b"title:: A\n\n- claim\n";
    let (exists, other, report) = l8_02_first_save_after(("pages/Elsewhere.md", body), true);
    eprintln!("L8-02 title claimant: {report}");
    assert_eq!(other, body, "the claimant was changed");
    assert!(!exists, "a second owner pages/A.md was created: {report}");
}

#[test]
fn l8_02_first_save_does_not_create_a_page_an_unreadable_file_could_own() {
    let body = b"title:: A\ntags:: caf\xe9\n- x\n";
    let (exists, other, report) = l8_02_first_save_after(("pages/Bad.md", body), true);
    eprintln!("L8-02 unreadable owner: {report}");
    assert_eq!(other, body, "the unreadable file was changed");
    assert!(
        !exists,
        "pages/A.md was created beside a possible owner: {report}"
    );
}

/// Control: an unreadable file that cannot own A does not block A.
#[test]
fn l8_02_control_an_unrelated_unreadable_file_does_not_block() {
    // A decodable title followed by an undecodable body: this is a known
    // unrelated owner. Invalid UTF-8 inside the preamble leaves the name
    // unknown, and the bounded fallback conservatively matches `a` in `tags`.
    let body = b"title:: Other\n\n- \xe9\n";
    let (exists, _, report) = l8_02_first_save_after(("pages/Bad.md", body), false);
    eprintln!("L8-02 unrelated unreadable: {report}");
    assert!(
        exists,
        "A was blocked by an unrelated unreadable file: {report}"
    );
}
