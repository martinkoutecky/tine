//! KGrsc (merge reviews 6fe4b654e / 8c2bfe7fc follow-ups 1, 2; DS-2, TE-5): the
//! rename helpers' flows killed at every write boundary they reach, then a
//! cold relaunch. The window's sequence, through the host calls the app's
//! commands make (`page_open`, `page_submit`, `page_save_now`, `page_owed`,
//! `page_move`, `rename_page`): each helper edit is one guarded page save
//! (the tag union, then the source's `tags::` line; one `alias::` edit per
//! other declarer; the own-alias release), then one request (a merge, a
//! rename, or one two-page move). A real process abort before every
//! ProductionIo call of every write phase the flow reaches (the G7 and
//! `home_rename` pattern: the planted fault counts calls over the host's
//! life), a cold start on the same app data, and what the window does at
//! relaunch (save the pages the host restored).
//!
//! Asserted at each kill and again after the relaunch: every file holds whole
//! bytes of one of the flow's states; no block text and no tag or alias is
//! lost; no two files claim one name; after the relaunch the graph is exactly
//! one of the flow's named states (the contract's "helpers are not atomic"
//! prefixes, the finished request, or for the day move the duplicate SPEC-s2
//! §1 accepts). The window-side matrix is `src/renameHelpers.interrupt.test.ts`;
//! the page host's own move kills are `page_host/production_move_crash_tests.rs`.
//! Release timing of the full day move is the ignored `day_move_timings`.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tine_graph_features::pages::{rename_page_as, RenameChoice};
use tine_store::host_faults::{abort_before, Phase};
use tine_store::{EditKind, PageHost, PageId, Store};

const WORKER: &str = "cm_helper_native_crash::crash_worker";
/// Every phase a page save, a merge's deletion or a drafted move reaches.
const PHASES: [Phase; 11] = [
    Phase::PageTemp,
    Phase::PageRename,
    Phase::PageSync,
    Phase::TrashMove,
    Phase::TrashSync,
    Phase::CustodyWrite,
    Phase::CustodyRetire,
    Phase::DraftTemp,
    Phase::DraftRename,
    Phase::DraftSync,
    Phase::DraftUnlink,
];
/// More boundaries of one phase than any flow reaches (asserted by the last
/// one being unreached).
const MAX_CALLS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Flow {
    /// Tag union on the survivor, the source's `tags::` line removed, merge.
    Tag,
    /// "Use the name": two declarers drop the alias, then the rename.
    Alias,
    /// A day with its own alias moved to a page: alias released, then the move.
    Day,
}

impl Flow {
    fn files(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Flow::Tag => &[
                ("pages/New.md", "tags:: b, c\n\n- kept note\n"),
                (
                    "pages/Old.md",
                    "tags:: a, b\n\n- gone note\n- gone parent\n  - nested note\n",
                ),
                ("pages/Ref.md", "- see [[Old]] and [[New]]\n"),
            ],
            Flow::Alias => &[
                ("pages/Machine Learning.md", "alias:: AI, ML\n\n- ml body\n"),
                ("pages/Models.md", "alias:: ML\n\n- models body\n"),
                ("pages/ML notes.md", "- notes body\n- notes second\n"),
                ("pages/Ref.md", "- see [[ML notes]]\n"),
            ],
            Flow::Day => &[
                (
                    "journals/2026_11_18.md",
                    "alias:: Trip\n\n- day note\n- TODO call\n  - child note\n",
                ),
                ("pages/Trip.md", "- trip plan\n"),
            ],
        }
    }
    /// Block texts that must be on some page at every boundary.
    fn notes(self) -> &'static [&'static str] {
        match self {
            Flow::Tag => &["kept note", "gone note", "gone parent", "nested note"],
            Flow::Alias => &["ml body", "models body", "notes body", "notes second"],
            Flow::Day => &["day note", "TODO call", "child note", "trip plan"],
        }
    }
    /// `(property key, value)` pairs that must stay declared on some page.
    fn kept(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Flow::Tag => &[("tags", "a"), ("tags", "b"), ("tags", "c")],
            Flow::Alias => &[("alias", "AI")],
            Flow::Day => &[],
        }
    }
}

// ---- the window's calls, as the app's commands make them ----

struct Window<'a> {
    host: &'a PageHost,
    store: &'a Store,
    session: u64,
    next: Cell<u64>,
    keys: RefCell<BTreeMap<String, String>>,
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

impl<'a> Window<'a> {
    fn new(host: &'a PageHost, store: &'a Store) -> Self {
        let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
        Self {
            host,
            store,
            session: reloaded["session"].as_u64().unwrap(),
            next: Cell::new(reloaded["nextId"].as_u64().unwrap()),
            keys: RefCell::new(BTreeMap::new()),
        }
    }

    fn id(&self) -> u64 {
        self.next.set(self.next.get() + 1);
        self.next.get() - 1
    }

    /// `page_open`, waiting for the host's first read; the page's key and version.
    fn open(&self, path: &str) -> (String, u64) {
        let page = PageId::from(path);
        if !self.keys.borrow().contains_key(path) {
            let name = Path::new(path).file_stem().unwrap().to_string_lossy();
            let opened = self
                .host
                .open(self.session, self.id(), &page, &name)
                .unwrap();
            let key = serde_json::to_value(opened).unwrap()["key"]
                .as_str()
                .unwrap()
                .to_owned();
            self.keys.borrow_mut().insert(path.to_owned(), key);
        }
        wait_for("the host's first read", || {
            self.host.read(&page).unwrap().revision.is_some()
        });
        let version = self.host.read(&page).unwrap().revision.unwrap().1;
        (self.keys.borrow()[path].clone(), version)
    }

    /// `page_owed` is empty: every input admitted so far is published.
    fn published(&self) {
        wait_for("publication", || {
            self.host
                .owed(self.session, None)
                .is_some_and(|owed| owed.is_empty())
        });
    }

    /// One helper edit: the page's header replaced, one guarded save.
    fn set_header(&self, path: &str, header: Option<&str>) {
        let (key, version) = self.open(path);
        let mut doc = self.store.page(&PageId::from(path)).unwrap().doc;
        doc.pre_block = header.map(str::to_owned);
        self.host
            .submit(
                self.session,
                self.id(),
                &key,
                &doc,
                version,
                None,
                &[EditKind::SaveBlock],
            )
            .unwrap();
        self.host.save_now(self.session, &[key]);
        self.published();
    }

    /// `page_move`, then `page_save_now`: every block of `from` but a `Moved to` stub goes to the
    /// end of `into`; both pages are published when it returns.
    fn move_notes(&self, from: &str, into: &str, name: &str) {
        let (from_key, from_version) = self.open(from);
        let (into_key, into_version) = self.open(into);
        let mut day = self.store.page(&PageId::from(from)).unwrap().doc;
        let mut page = self.store.page(&PageId::from(into)).unwrap().doc;
        let mut stub = day.blocks[0].clone();
        stub.raw = format!("Moved to [[{name}]]");
        stub.children.clear();
        page.blocks.append(&mut day.blocks);
        day.blocks = vec![stub];
        self.host
            .move_blocks(
                self.session,
                self.id(),
                (&from_key, &day, from_version),
                (&into_key, &page, into_version),
                &[EditKind::MoveBlocks],
            )
            .unwrap();
        // The window's flush after the move: both saves due now. A hint sent
        // before the host has applied the move is dropped, so it is sent
        // until the pages are published (else the 400 ms send debounce
        // decides the time).
        wait_for("publication", || {
            self.host
                .save_now(self.session, &[into_key.clone(), from_key.clone()]);
            self.host
                .owed(self.session, None)
                .is_some_and(|owed| owed.is_empty())
        });
    }

    /// `rename_page`, as the window sends it after the helper saves.
    fn request(&self, old: &str, new: &str, path: &str, merge_into: Option<&str>) {
        let choice = merge_into.map_or(RenameChoice::Rename, RenameChoice::MergeInto);
        rename_page_as(self.store, self.host, old, new, Some(path), choice, &[]).unwrap();
        self.published();
    }
}

/// The flow's stages, in the window's order; `after(k)` runs when stage k is done.
fn drive(flow: Flow, window: &Window, mut after: impl FnMut(usize)) {
    match flow {
        Flow::Tag => {
            window.set_header("pages/New.md", Some("tags:: b, c, a"));
            after(1);
            window.set_header("pages/Old.md", None);
            after(2);
            window.request("Old", "New", "pages/Old.md", Some("pages/New.md"));
            after(3);
        }
        Flow::Alias => {
            window.set_header("pages/Machine Learning.md", Some("alias:: AI"));
            after(1);
            window.set_header("pages/Models.md", None);
            after(2);
            window.request("ML notes", "ML", "pages/ML notes.md", None);
            after(3);
        }
        Flow::Day => {
            window.set_header("journals/2026_11_18.md", None);
            after(1);
            window.move_notes("journals/2026_11_18.md", "pages/Trip.md", "Trip");
            after(2);
        }
    }
}

// ---- graph snapshots ----

type Snapshot = BTreeMap<String, String>;

fn snapshot(root: &Path) -> Snapshot {
    let mut files = Snapshot::new();
    for dir in ["pages", "journals"] {
        let Ok(entries) = fs::read_dir(root.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            // A kill between a save's temp file and its rename leaves the
            // hidden temp beside the page (`strays`); it is not a page.
            if entry.path().is_file() && !entry.file_name().to_string_lossy().starts_with('.') {
                files.insert(
                    format!("{dir}/{}", entry.file_name().to_string_lossy()),
                    fs::read_to_string(entry.path()).unwrap(),
                );
            }
        }
    }
    files
}

/// Hidden save temporaries a kill left in the page directories.
fn strays(root: &Path) -> usize {
    ["pages", "journals"]
        .iter()
        .filter_map(|dir| fs::read_dir(root.join(dir)).ok())
        .flat_map(|entries| entries.flatten())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
        .count()
}

fn build(flow: Flow, root: &Path) {
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (path, text) in flow.files() {
        fs::write(root.join(path), text).unwrap();
    }
}

fn open_store(root: &Path) -> std::sync::Arc<Store> {
    let store = std::sync::Arc::new(Store::open(root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    store
}

/// The graph after each stage of an uninterrupted run: index k, after k stages.
fn reference(flow: Flow) -> Vec<Snapshot> {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("graph");
    build(flow, &root);
    let store = open_store(&root);
    let app = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app.path()).unwrap();
    let window = Window::new(&host, &store);
    let mut states = vec![snapshot(&root)];
    drive(flow, &window, |_| states.push(snapshot(&root)));
    drop(window);
    drop(host);
    store.close();
    states
}

/// What a kill may leave (before the relaunch): per path, the bytes of any
/// state, or no file where some state has none.
fn check_whole(flow: Flow, label: &str, now: &Snapshot, states: &[Snapshot]) {
    for (path, text) in now {
        let known = states.iter().any(|s| s.get(path) == Some(text));
        assert!(
            known,
            "{flow:?} {label}: {path} holds bytes of no state:\n{text}"
        );
    }
    for state in states {
        for path in state.keys() {
            assert!(
                now.contains_key(path)
                    || states.iter().any(|s| !s.contains_key(path))
                    || flow == Flow::Day,
                "{flow:?} {label}: {path} is gone though every state has it"
            );
        }
    }
}

/// Nothing lost, and no two files claim one name.
fn check_kept(flow: Flow, label: &str, now: &Snapshot) {
    for note in flow.notes() {
        assert!(
            now.values().any(|text| text
                .lines()
                .any(|l| l.trim_start_matches([' ', '\t', '-']).trim() == *note)),
            "{flow:?} {label}: “{note}” is on no page:\n{now:#?}"
        );
    }
    for (key, value) in flow.kept() {
        let prefix = format!("{key}:: ");
        assert!(
            now.values().any(|text| text.lines().any(|line| {
                line.strip_prefix(&prefix)
                    .is_some_and(|rest| rest.split(',').any(|v| v.trim() == *value))
            })),
            "{flow:?} {label}: {key} “{value}” is on no page:\n{now:#?}"
        );
    }
    let names: Vec<String> = now
        .keys()
        .map(|path| {
            Path::new(path)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_lowercase()
        })
        .collect();
    assert_eq!(
        names.iter().collect::<BTreeSet<_>>().len(),
        names.len(),
        "{flow:?} {label}: two files claim one name: {names:?}"
    );
}

// ---- the kill ----

#[test]
fn crash_worker() {
    let _integration_guard = crate::integration_lock();
    let Ok(root) = std::env::var("TINE_KGRSC_ROOT") else {
        return;
    };
    let flow = match std::env::var("TINE_KGRSC_FLOW").unwrap().as_str() {
        "tag" => Flow::Tag,
        "alias" => Flow::Alias,
        _ => Flow::Day,
    };
    let phase = PHASES[std::env::var("TINE_KGRSC_PHASE")
        .unwrap()
        .parse::<usize>()
        .unwrap()];
    let n = std::env::var("TINE_KGRSC_N").unwrap().parse().unwrap();
    let store = open_store(Path::new(&root));
    abort_before(phase, n);
    let app = std::env::var("TINE_KGRSC_APP").unwrap();
    let host = PageHost::start_for_tests(&store, Path::new(&app)).unwrap();
    let window = Window::new(&host, &store);
    drive(flow, &window, |_| {});
    drop(window);
    drop(host);
    // Exit 3: this boundary is not on the flow's path (a panic is 101).
    std::process::exit(3);
}

/// The window at relaunch: the host restored what it held; the pages with
/// unsaved input are saved, and the graph is read once it stops changing.
fn relaunch(root: &Path, app: &Path) -> Snapshot {
    let store = open_store(root);
    let host = PageHost::start_for_tests(&store, app).unwrap();
    let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
        .as_u64()
        .unwrap();
    let (mut last, mut since) = (snapshot(root), Instant::now());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let owed = host.owed(session, None).unwrap_or_default();
        if !owed.is_empty() {
            let keys: Vec<String> = owed.into_iter().map(|(key, _)| key).collect();
            host.save_now(session, &keys);
        }
        std::thread::sleep(Duration::from_millis(20));
        let now = snapshot(root);
        if now != last {
            (last, since) = (now, Instant::now());
        } else if since.elapsed() > Duration::from_millis(400)
            && host.owed(session, None).is_some_and(|owed| owed.is_empty())
        {
            break;
        }
        assert!(Instant::now() < deadline, "the relaunch never settled");
    }
    drop(host);
    store.close();
    last
}

fn run_matrix(flow: Flow, tag: &str, accepted: impl Fn(&[Snapshot]) -> Vec<(String, Snapshot)>) {
    let _guard = crate::integration_lock();
    let states = reference(flow);
    let named = accepted(&states);
    let mut reached = Vec::new();
    let mut outcomes = Vec::new();
    for (p, phase) in PHASES.iter().enumerate() {
        for n in 0..MAX_CALLS {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("graph");
            let app = temp.path().join("app");
            build(flow, &root);
            fs::create_dir_all(&app).unwrap();
            let output = Command::new(tine_store::test_binary::own_test_binary())
                .args(["--exact", WORKER, "--nocapture"])
                .env("TINE_KGRSC_ROOT", &root)
                .env("TINE_KGRSC_APP", &app)
                .env("TINE_KGRSC_FLOW", tag)
                .env("TINE_KGRSC_PHASE", p.to_string())
                .env("TINE_KGRSC_N", n.to_string())
                .output()
                .unwrap();
            let log = String::from_utf8_lossy(&output.stdout);
            assert_ne!(
                output.status.code(),
                Some(101),
                "{flow:?} {phase:?}#{n}: {log}"
            );
            if output.status.code() == Some(3) {
                // The flow ran to its end: it must be the finished request.
                assert_eq!(
                    &snapshot(&root),
                    states.last().unwrap(),
                    "{flow:?} {phase:?}#{n}"
                );
                break;
            }
            assert!(!output.status.success(), "{flow:?} {phase:?}#{n}: {log}");
            assert!(n + 1 < MAX_CALLS, "{flow:?} {phase:?}: raise MAX_CALLS");
            let label = format!("{phase:?}#{n}");
            reached.push(label.clone());
            let killed = snapshot(&root);
            let stray_at_kill = strays(&root);
            check_whole(flow, &label, &killed, &states);
            check_kept(flow, &format!("{label} at the kill"), &killed);
            let reopened = relaunch(&root, &app);
            let stray_after = strays(&root);
            check_kept(flow, &format!("{label} after the relaunch"), &reopened);
            let outcome = named.iter().find(|(_, state)| *state == reopened);
            assert!(
                outcome.is_some(),
                "{flow:?} {label}: the relaunch left a graph no named state is:\n{reopened:#?}"
            );
            let at_kill = named
                .iter()
                .find(|(_, state)| *state == killed)
                .map_or("between states", |(name, _)| name.as_str());
            outcomes.push(format!(
                "{label}: kill left {at_kill}; reopened as {}; hidden temp files {stray_at_kill} -> {stray_after}",
                outcome.unwrap().0
            ));
        }
    }
    eprintln!("KGRSC {flow:?} boundaries reached: {}", reached.len());
    for line in &outcomes {
        eprintln!("KGRSC {flow:?} {line}");
    }
    assert!(
        reached.len() >= 6,
        "{flow:?}: too few boundaries reached: {reached:?}"
    );
}

fn prefixes(states: &[Snapshot]) -> Vec<(String, Snapshot)> {
    states
        .iter()
        .enumerate()
        .map(|(k, state)| {
            let name = match k {
                0 => "nothing done".to_owned(),
                k if k + 1 == states.len() => "request done".to_owned(),
                k => format!("{k} helper save(s) done, request not done"),
            };
            (name, state.clone())
        })
        .collect()
}

#[test]
fn a_tag_union_and_merge_killed_at_each_write_boundary_reopens_losing_nothing() {
    run_matrix(Flow::Tag, "tag", prefixes);
}

#[test]
fn use_the_name_and_rename_killed_at_each_write_boundary_reopens_losing_nothing() {
    run_matrix(Flow::Alias, "alias", prefixes);
}

#[test]
fn an_own_alias_day_move_killed_at_each_write_boundary_reopens_losing_nothing() {
    run_matrix(Flow::Day, "day", |states| {
        let mut named = prefixes(states);
        // SPEC-s2 §1: the receiver published, the day not yet.
        let mut duplicate = states[1].clone();
        duplicate.insert("pages/Trip.md".into(), states[2]["pages/Trip.md"].clone());
        named.push((
            "duplicate (receiver moved, day kept its notes)".to_owned(),
            duplicate,
        ));
        named
    });
}

// ---- release timing of the full day move ----

/// A day of `notes` notes with its own alias, the receiving page, and 200
/// unrelated pages; every file's mtime is an hour back, so the store's
/// fresh-file rehash never joins the measurement.
fn day_graph(notes: usize) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("graph");
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let body: String = (0..notes)
        .map(|i| format!("- note {i} with some text\n"))
        .collect();
    fs::write(
        root.join("journals/2026_11_18.md"),
        format!("alias:: Trip\n\n{body}"),
    )
    .unwrap();
    fs::write(root.join("pages/Trip.md"), "- trip plan\n").unwrap();
    for i in 0..200 {
        fs::write(
            root.join(format!("pages/Other{i:05}.md")),
            format!("- unrelated {i}\n"),
        )
        .unwrap();
    }
    let back = std::time::SystemTime::now() - Duration::from_secs(3600);
    for dir in ["pages", "journals"] {
        for entry in fs::read_dir(root.join(dir)).unwrap().flatten() {
            fs::File::options()
                .write(true)
                .open(entry.path())
                .unwrap()
                .set_modified(back)
                .unwrap();
        }
    }
    (temp, root)
}

/// One run: the own-alias release saved, then the two-page move published.
/// The three times are the alias save, the move, and both together.
fn time_day_move(notes: usize) -> (Duration, Duration, Duration) {
    let (_temp, root) = day_graph(notes);
    let store = open_store(&root);
    let app = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app.path()).unwrap();
    let window = Window::new(&host, &store);
    // The window holds both pages before the helper starts.
    window.open("journals/2026_11_18.md");
    window.open("pages/Trip.md");
    let started = Instant::now();
    window.set_header("journals/2026_11_18.md", None);
    let alias = started.elapsed();
    let moving = Instant::now();
    window.move_notes("journals/2026_11_18.md", "pages/Trip.md", "Trip");
    let moved = moving.elapsed();
    let total = started.elapsed();
    let trip = fs::read_to_string(root.join("pages/Trip.md")).unwrap();
    assert_eq!(trip.lines().count(), notes + 1);
    drop(window);
    drop(host);
    store.close();
    (alias, moved, total)
}

/// Release build, median of 3, load average beside it: run with
/// `cargo test --release -p tine-graph-features --test integration
/// day_move_timings -- --ignored --nocapture`. Recorded, not asserted.
#[test]
#[ignore = "timing measurement, not a gate"]
fn day_move_timings() {
    let _guard = crate::exclusive_integration_lock();
    let load = fs::read_to_string("/proc/loadavg").unwrap_or_default();
    for notes in [50, 5_000] {
        let mut runs: Vec<(Duration, Duration, Duration)> =
            (0..3).map(|_| time_day_move(notes)).collect();
        runs.sort_by_key(|run| run.2);
        let (alias, moved, total) = runs[1];
        eprintln!(
            "KGRSC timing day move {notes} notes: median total {total:?} (alias save {alias:?}, move {moved:?}); totals {:?}; load {}",
            runs.iter().map(|run| run.2).collect::<Vec<_>>(),
            load.trim()
        );
    }
}
