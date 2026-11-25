//! tine#1041: a draft of a page this graph has no file for, held by the host
//! with no window page naming it. It got there by orphan adoption ("Recover
//! into this graph", `root_identity::adopt_namespace`), which moves a missing
//! graph's whole `drafts-v2/<old id>` directory to `drafts-v2/<this graph's
//! id>` with one directory rename and decodes nothing (its own tests in
//! src-tauri prove the bytes move untouched). Here the same directory rename
//! runs between a real host of graph A, which left the draft, and a real host
//! launched on graph B.
use super::*;

const KEY: &str = "pages/Repro 964.md";

/// Graph A's host leaves a draft of `KEY` (its saves fail, the switch stop
/// drafts it); graph B has no `KEY`. B is launched under `graph-b` once
/// before adoption, then A's namespace is renamed onto B's, and B is
/// relaunched over it. Returns B's live host and what B's first launch
/// owed.
fn adopted_into_graph_b() -> (Live, Vec<(PageKey, u64)>) {
    let a = Live::new(&[(KEY, "- base\n")]);
    let (key, page) = a.open(KEY);
    a.faults(super::io::Phase::PageTemp, 1000);
    let id = a.submit(&key, "- mine\n", page.version, None).unwrap();
    a.answer(&key, id);
    assert!(a.host.stop_begin(id, StopMode::Switch));
    assert_eq!(
        a.until_stop(),
        StopState::Ready,
        "the failed save is drafted"
    );
    let app = a.app();
    let Live {
        _dir, host, store, ..
    } = a;
    assert!(host.stop_saved(id, StopMode::Switch).is_ok());
    drop(store);
    let from = app.join("drafts-v2").join("test-graph");
    assert!(
        fs::read_dir(&from).unwrap().count() > 0,
        "graph A left a draft"
    );

    // Graph B: another page, no KEY.
    let root_b = _dir.path().join("graph-b");
    fs::create_dir_all(root_b.join("pages")).unwrap();
    fs::create_dir_all(root_b.join("journals")).unwrap();
    fs::write(root_b.join("pages/Other.md"), "- other\n").unwrap();
    let launch_b = |store: &Arc<Store>| {
        let (sender, mail) = mpsc::channel();
        let kinds = Kinds::default();
        let host = PageHost::start(
            store,
            &app,
            "graph-b",
            DraftOwnership::Owned,
            recording(&kinds),
            move |mail| {
                let _ = sender.send(mail);
            },
        )
        .unwrap();
        (host, mail, kinds)
    };
    let store = Arc::new(Store::open(&root_b, Default::default()).unwrap().0);
    store.whole_graph_reconciled().unwrap();
    let (host, ..) = launch_b(&store);
    let before = host.owed(host.session(), None).unwrap();
    host.stop();

    // Adoption: B's namespace holds no draft (a census-empty skeleton at
    // most, which adoption removes), then A's directory takes its place.
    let to = app.join("drafts-v2").join("graph-b");
    if to.exists() {
        fs::remove_dir_all(&to).unwrap();
    }
    fs::rename(&from, &to).unwrap();
    let (host, mail, kinds) = launch_b(&store);
    let root = store.graph.root.clone();
    (
        Live {
            _dir,
            root,
            store,
            host,
            mail,
            kinds,
            id: std::cell::Cell::new(1000),
        },
        before,
    )
}

fn owed_now(b: &Live) -> Vec<(PageKey, u64)> {
    b.host.owed(b.host.session(), None).unwrap()
}

/// What a window's path route opens (`page_open` of the path): the held
/// text, conflicted, on no file.
fn window_opens(b: &Live) -> (PageKey, MailPage) {
    let id = b.id();
    let key = (b.host)
        .open(b.host.session(), id, &PageId::from(KEY), "Repro 964")
        .unwrap()
        .key;
    let mail = b.answer(&key, id);
    assert!(mail.notice.conflict_reported, "the bar lists the conflict");
    let page = mail.page.unwrap();
    assert!(page.conflict, "the window is shown a conflict");
    assert_eq!(page.disk, Some(DiskToken::NoFile));
    let MailText::Page { dto } = &page.text else {
        panic!("the open answers the held text: {:?}", page.text);
    };
    assert_eq!(dto.blocks[0].raw, "mine");
    (key, page)
}

fn until_owed_empty(b: &Live) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !owed_now(b).is_empty() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

/// The held draft is owed on every barrier, has no file, and the store's
/// read by path says `held-absent` (the path route's answer, which the
/// window turns into one host open of that path).
fn assert_held_absent(b: &Live, before: &[(PageKey, u64)]) {
    assert!(before.is_empty(), "B owed nothing before adoption");
    assert_eq!(owed_now(b).first().map(|(key, _)| key.as_str()), Some(KEY));
    assert!(!b.root.join(KEY).exists());
    let unsaved = b.host.draft_status().unsaved;
    assert!(unsaved
        .iter()
        .any(|p| p.path == KEY && p.recovered && p.conflict));
    assert!(matches!(
        b.store.page(&PageId::from(KEY)),
        Err(crate::StoreError::NotFound(
            crate::store::PageReadSource::HeldAbsent
        ))
    ));
}

#[test]
fn an_adopted_draft_of_a_missing_page_is_owed_and_read_as_held_absent() {
    let (b, before) = adopted_into_graph_b();
    settle();
    assert_held_absent(&b, &before);
    b.host.stop();
}

/// "Use disk version" on the opened draft: the host discards it, creates no
/// file, and owes nothing, so the switch can proceed.
#[test]
fn an_adopted_draft_opened_by_path_then_use_disk_version_clears_the_debt() {
    let (b, before) = adopted_into_graph_b();
    settle();
    assert_held_absent(&b, &before);
    let (key, _) = window_opens(&b);
    let id = b.id();
    b.host.discard(b.host.session(), id, &key).unwrap();
    assert!(b.answer(&key, id).answer.is_some());
    b.close(&key);
    assert!(until_owed_empty(&b), "Use disk version empties owed");
    assert!(!b.root.join(KEY).exists());
    assert!(b.drafts_in("graph-b").is_empty(), "the draft is retired");
    assert!(b.host.draft_status().unsaved.is_empty());
    b.host.stop();
}

/// "Keep mine" on the opened draft: the host writes the text to the path,
/// and owes nothing.
#[test]
fn an_adopted_draft_opened_by_path_then_keep_mine_writes_it_and_clears_the_debt() {
    let (b, before) = adopted_into_graph_b();
    settle();
    assert_held_absent(&b, &before);
    let (key, page) = window_opens(&b);
    let resolve = page.disk.clone().unwrap_or(DiskToken::NoFile);
    let id = b
        .submit(&key, "- mine\n", page.version, Some(&resolve))
        .unwrap();
    assert!(b.answer(&key, id).answer.is_some_and(|answer| answer.took));
    b.close(&key);
    assert!(until_owed_empty(&b), "Keep mine empties owed");
    assert_eq!(b.disk(&key), "- mine\n");
    assert!(b.drafts_in("graph-b").is_empty(), "the draft is retired");
    assert!(b.host.draft_status().unsaved.is_empty());
    b.host.stop();
}

impl Live {
    /// Page draft vehicles in `graph_id`'s draft directory.
    fn drafts_in(&self, graph_id: &str) -> Vec<String> {
        let dir = self.app().join("drafts-v2").join(graph_id);
        let mut names: Vec<_> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap())
                    .filter(|entry| entry.file_type().unwrap().is_file())
                    .map(|entry| entry.file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}
