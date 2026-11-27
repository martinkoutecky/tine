//! GH #540: the recovery panel's kept originals at the binding boundary
//! (`PageHost::kept_originals`, `kept_text`), against a real Store and the
//! driver thread. Each test keeps an original the way a user does (a rename
//! cancelled after its source's removal published, then a process death)
//! and starts a live host on what that left.
use super::*;
use crate::page_host::io::{KeptOriginal, KeptText};
use std::sync::atomic::Ordering;
use tine_core::model::PageKind;

const SOURCE: &str = "pages/a.md";
const RECEIVER: &str = "pages/c.md";
const REFERRER: &str = "pages/b.md";
/// Windows line ends: Copy hands them over exactly.
const ORIGINAL: &str = "- old\r\n- two\r\n";

/// A live host on a graph whose rename of `pages/a.md` (bytes `original`)
/// to `pages/c.md` was cancelled with a's removal published; `extra` files
/// are on disk before the host starts.
fn kept(original: &[u8], extra: &[(&str, &[u8])]) -> Live {
    let dir = sandbox();
    let (graph, app) = (dir.path().join("graph"), dir.path().join("app"));
    for area in ["pages", "journals", "logseq"] {
        fs::create_dir_all(graph.join(area)).unwrap();
    }
    fs::create_dir_all(&app).unwrap();
    fs::write(graph.join(SOURCE), original).unwrap();
    fs::write(graph.join(REFERRER), "- see [[a]]\n").unwrap();
    crate::page_host::production_tests::keep_an_original(
        &graph,
        &app,
        "test-graph",
        [SOURCE, RECEIVER, REFERRER],
    );
    assert!(!graph.join(SOURCE).exists(), "the original is not on disk");
    for (path, body) in extra {
        fs::write(graph.join(path), body).unwrap();
    }
    Live::start_in(dir, false, crate::WatchMode::Notify, DraftOwnership::Owned)
}

fn list(live: &Live) -> Vec<KeptOriginal> {
    live.host
        .kept_originals(live.host.session())
        .expect("answered")
}

/// The one row the panel lists.
fn listed(live: &Live) -> KeptOriginal {
    let rows = list(live);
    assert_eq!(rows.len(), 1, "{rows:?}");
    rows.into_iter().next().unwrap()
}

fn copy(live: &Live, row: &KeptOriginal) -> KeptText {
    live.host
        .kept_text(live.host.session(), &row.key, row.origin)
}

/// Every file under `dir`, with its bytes.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap_or_default());
            }
        }
    }
    out
}

/// Acceptance 1 and 15: CRLF and LF originals are listed with their path,
/// name and kind, and Copy original hands over the exact text (the string
/// the clipboard writer receives); listing and copying write nothing (the
/// graph and app directories are byte-identical), publish no edit kind and
/// send no mail.
#[test]
fn the_list_names_each_kept_original_and_copy_gives_its_exact_text() {
    for original in [ORIGINAL, "title:: Named\n\n- old\n"] {
        let live = kept(original.as_bytes(), &[]);
        let row = listed(&live);
        let named = if original == ORIGINAL { "a" } else { "Named" };
        assert_eq!(
            (
                row.key.as_str(),
                row.path.as_str(),
                row.name.as_deref(),
                row.kind
            ),
            (SOURCE, SOURCE, Some(named), Some(PageKind::Page))
        );
        assert_eq!((row.len, row.text), (original.len(), true));
        assert_eq!(live.host.draft_status().kept, 1, "the launch count");
        while live.mail.try_recv().is_ok() {}
        let before = tree(live._dir.path());
        let kinds = live.kinds.lock().unwrap().len();
        for _ in 0..3 {
            assert_eq!(listed(&live), row);
            let KeptText::Text { text } = copy(&live, &row) else {
                panic!("Copy original")
            };
            assert_eq!(text.as_bytes(), original.as_bytes(), "byte for byte");
        }
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(tree(live._dir.path()), before, "nothing is written");
        assert_eq!(live.kinds.lock().unwrap().len(), kinds, "no edit kind");
        assert!(live.mail.try_recv().is_err(), "no mail");
        assert_eq!(listed(&live), row, "still kept");
        live.host.stop();
    }
}

/// Acceptance 2 (and 9's other half): listing and copying never touch the
/// graph's discovery records. The file now at `pages/a.md` has a preamble
/// Tine cannot read; its direct-read failure is the only record of what
/// it could be called, and it stays recorded through List and Copy, so a
/// page that file might be named is still refused as on develop.
#[test]
fn listing_and_copying_leave_a_discovery_failure_recorded() {
    let live = kept(
        b"title:: Named\n\n- old\n",
        &[(SOURCE, b"title:: \xff\xfe\n- b\n")],
    );
    let path = live.root.join(SOURCE);
    let _ = live.store.graph.entry_for_path(&path);
    let recorded = |live: &Live| live.store.graph.name_discovery_recorded(&path);
    assert!(recorded(&live), "the live file's read failure is recorded");
    let row = listed(&live);
    assert_eq!(
        row.name.as_deref(),
        Some("Named"),
        "named from the kept text"
    );
    assert!(matches!(copy(&live, &row), KeptText::Text { .. }));
    assert!(recorded(&live), "List and Copy leave it recorded");
    live.host.stop();
}

/// Acceptance 3: a path the graph's settings hide after the list was taken
/// keeps its row (by key and origin), without a name or kind, and Copy
/// original still gives its text.
#[test]
fn a_hidden_path_keeps_its_row_and_its_copy() {
    let live = kept(ORIGINAL.as_bytes(), &[]);
    let before = listed(&live);
    assert!(before.name.is_some());
    let config = live.root.join("logseq/config.edn");
    fs::write(config, "{:hidden [\"pages/a.md\"]}\n").unwrap();
    live.store.refresh(crate::Depth::Rebuild).unwrap();
    let KeptText::Text { text } = copy(&live, &before) else {
        panic!("Copy of a row listed before the path was hidden")
    };
    assert_eq!(text, ORIGINAL);
    let row = listed(&live);
    assert_eq!((row.name, row.kind), (None, None), "no page now");
    assert_eq!(
        (row.key, row.origin, row.text),
        (before.key, before.origin, true)
    );
    live.host.stop();
}

/// Acceptance 4 and 5: a row of another origin (a stale row) and a
/// receiver's key are no kept original: `Missing`. A row belongs to the
/// session that listed it: another session's List and Copy are not
/// answered, and after a window reload the old session's are not either,
/// while the new session lists the same original.
#[test]
fn stale_rows_are_missing_and_other_sessions_are_unanswered() {
    let live = kept(ORIGINAL.as_bytes(), &[]);
    let row = listed(&live);
    assert!(
        list(&live).iter().all(|r| r.key != RECEIVER),
        "no receiver row"
    );
    let session = live.host.session();
    assert_eq!(
        live.host.kept_text(session, &row.key, row.origin + 1),
        KeptText::Missing
    );
    assert_eq!(
        live.host.kept_text(session, RECEIVER, row.origin),
        KeptText::Missing
    );
    assert_eq!(live.host.kept_originals(session + 1), None);
    let other = live.host.kept_text(session + 1, &row.key, row.origin);
    assert_eq!(other, KeptText::Unavailable);
    live.host.window_reloaded();
    assert_ne!(live.host.session(), session);
    assert_eq!(live.host.kept_originals(session), None);
    assert_eq!(
        live.host.kept_text(session, &row.key, row.origin),
        KeptText::Unavailable
    );
    assert_eq!(listed(&live), row, "the new session lists it");
    live.host.stop();
}

/// Acceptance 8 and 13: an original that is not UTF-8 (`ff fe 00`) is listed
/// as not text and Copy answers `NotText`, never a lossy string, and it stays
/// kept. Beside it in the same List call, a text original (`78 0d 0a`) under
/// a hidden path (both rows have no name or kind) is text and copies.
#[test]
fn a_binary_original_is_not_text_beside_a_hidden_text_one() {
    let binary: &[u8] = &[0xff, 0xfe, 0x00];
    // A rename of a file that is not text is refused, so the kept text
    // original's preimage is set to those bytes, as a source rewritten by
    // another program before its removal would have kept them.
    let live = kept(ORIGINAL.as_bytes(), &[]);
    live.host.driver.shared.with_state_for_tests(|state| {
        let order = &mut state.progress.host.order;
        let name = order.kept().next().unwrap().0.clone();
        let mut op = order.paired(&name).unwrap().clone();
        for pair in op.pairs.iter_mut().filter(|p| p.kept()) {
            pair.preimage = Some(Arc::from(binary));
        }
        order.adopt(name, op);
    });
    let row = listed(&live);
    assert_eq!((row.text, row.len), (false, 3));
    assert_eq!(copy(&live, &row), KeptText::NotText { len: 3 });
    // A second kept original, its text `x\r\n`, at another hidden path.
    let other = "pages/x.md";
    live.host.driver.shared.with_state_for_tests(|state| {
        let order = &mut state.progress.host.order;
        let name = order.kept().next().unwrap().0.clone();
        let mut op = order.paired(&name).unwrap().clone();
        for pair in &mut op.pairs {
            pair.source = other.into();
            pair.preimage = Some(Arc::from(&b"x\r\n"[..]));
            pair.origin += 1_000;
        }
        order.adopt(format!("{name}-other"), op);
    });
    let config = live.root.join("logseq/config.edn");
    fs::write(config, "{:hidden [\"pages\"]}\n").unwrap();
    live.store.refresh(crate::Depth::Rebuild).unwrap();
    let rows = list(&live);
    assert_eq!(rows.len(), 2, "{rows:?}");
    let by_key = |key: &str| rows.iter().find(|r| r.key == key).unwrap();
    let (binary_row, text_row) = (by_key(SOURCE), by_key(other));
    assert_eq!((binary_row.name.clone(), binary_row.kind), (None, None));
    assert_eq!((text_row.name.clone(), text_row.kind), (None, None));
    assert_eq!((binary_row.text, text_row.text), (false, true));
    assert_eq!(copy(&live, binary_row), KeptText::NotText { len: 3 });
    assert_eq!(
        copy(&live, text_row),
        KeptText::Text {
            text: "x\r\n".into()
        }
    );
    assert_eq!(live.host.draft_status().kept, 2, "both stay kept");
    live.host.stop();
}

/// Acceptance 11: a host that is down (stopped, its driver ending) does not
/// answer List or Copy (`Unavailable`), which the panel tells apart from an
/// original no longer kept (`Missing`).
#[test]
fn a_host_that_is_down_leaves_list_and_copy_unanswered() {
    let live = kept(ORIGINAL.as_bytes(), &[]);
    let row = listed(&live);
    let session = live.host.session();
    (live.host.driver.shared).with_state_for_tests(|state| state.progress.host.stop());
    assert_eq!(live.host.kept_originals(session), None);
    assert_eq!(
        live.host.kept_text(session, &row.key, row.origin),
        KeptText::Unavailable
    );
    live.host.stop();
}

/// Acceptance 6, binding half: with a kept original, an unrelated page's
/// edit, save and publication mail and the host's collection make no search
/// for kept originals (the production counter `Order::kept_searches`); one
/// List and one Copy make one each.
#[test]
fn unrelated_work_makes_no_kept_search() {
    let live = kept(ORIGINAL.as_bytes(), &[("pages/d.md", b"- d\n")]);
    let row = listed(&live);
    let searches = || {
        let read = |state: &retained::HostState| {
            state
                .progress
                .host
                .order
                .kept_searches
                .load(Ordering::Relaxed)
        };
        live.host.driver.shared.read(read).unwrap()
    };
    let start = searches();
    let (key, page) = live.open("pages/d.md");
    live.submit(&key, "- d changed\n", page.version, None)
        .unwrap();
    live.until_disk(&key, "- d changed\n");
    live.until_indexed(&key, "- d changed\n");
    live.close(&key);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(searches(), start, "no search on unrelated work");
    listed(&live);
    assert_eq!(searches(), start + 1, "one per List");
    copy(&live, &row);
    assert_eq!(searches(), start + 2, "one per Copy");
    live.host.stop();
}
