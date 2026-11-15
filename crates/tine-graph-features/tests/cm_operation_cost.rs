//! CM B4 follow-up (review F2, PC-3): what one whole host operation costs.
//! Helper budgets pin the merge text trace, drawer classification and
//! certificates; these pin a page rename with N referrers, a merge of a
//! source into a survivor with N referrers, a namespace rename with K
//! children, and a plain merge, through the real host operation, from the
//! call through the host's cleanup, including preflight validation and
//! certificate handling. Each count is stated beside its lower bound.
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tine_graph_features::pages;
use tine_store::cost_counters::{self, Counts};
use tine_store::Store;

static CASE_LOCK: Mutex<()> = Mutex::new(());

/// One operation's bill: the store's primitive counts plus the core's
/// reference span walks and certificate checks.
#[derive(Debug, Clone, Copy)]
struct Bill {
    io: Counts,
    walks: u64,
    certificates: u64,
    call: Duration,
}

impl Bill {
    /// Whole-file reads the operation made itself: the store's page reads
    /// and the feature crates' bounded reads. Watcher and observation
    /// re-reads (contract §5.4) are billed apart.
    fn reads(&self) -> u64 {
        self.io.full_reads + self.io.store_reads + self.io.preamble_reads + self.io.hash_reads
    }
}

/// Write `files` into a fresh graph, backdated past the watcher's racy
/// window, open it, start a host, and measure `op` from the call through the
/// host's cleanup.
fn measure(
    files: &[(String, String)],
    op: impl FnOnce(&Store, &tine_store::PageHost) -> std::io::Result<()>,
    check: impl FnOnce(&Path),
) -> Bill {
    measure_in(files, Trash::Fresh, op, check)
}

/// The graph's trash folder when the operation starts.
#[derive(Clone, Copy, PartialEq)]
enum Trash {
    /// Absent: the first deletion creates it, which freshens its ancestors, so
    /// each deletion in the next two seconds syncs the whole chain.
    Fresh,
    /// Present with every ancestor's modification time past the host's
    /// `SETTLED` window: a graph that has trashed a page before.
    Settled,
}

/// [`measure`] with the trash folder in the given state.
/// A directory handle that can set its times. Windows opens a directory only
/// with backup semantics, and setting times needs FILE_WRITE_ATTRIBUTES
/// (hosted CI 43383656044: plain `File::open` was "Access is denied").
fn open_dir_for_times(dir: &Path) -> fs::File {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)
            .unwrap()
    }
    #[cfg(not(windows))]
    {
        fs::File::open(dir).unwrap()
    }
}

fn measure_in(
    files: &[(String, String)],
    trash: Trash,
    op: impl FnOnce(&Store, &tine_store::PageHost) -> std::io::Result<()>,
    check: impl FnOnce(&Path),
) -> Bill {
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::Builder::new()
        .prefix("cm-op-cost-")
        .tempdir()
        .unwrap();
    let root = temp.path().join("graph");
    let old = SystemTime::now() - Duration::from_secs(3600);
    for (rel, text) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    if trash == Trash::Settled {
        let folder = root.join("logseq/.tine-trash/pages");
        fs::create_dir_all(&folder).unwrap();
        // Innermost first: setting a directory's time does not touch its
        // parent's, but creating an entry does.
        for dir in folder.ancestors().take_while(|dir| dir.starts_with(&root)) {
            open_dir_for_times(dir).set_modified(old).unwrap();
        }
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, &temp.path().join("app")).unwrap();
    let started = std::time::Instant::now();
    while !host.quiescent_for_tests() {
        assert!(started.elapsed().as_secs() < 30, "the host never settled");
        std::thread::yield_now();
    }
    cost_counters::reset();
    tine_core::text_trace::work::take();
    let called = std::time::Instant::now();
    op(&store, &host).unwrap();
    let call = called.elapsed();
    let done = std::time::Instant::now();
    while !host.quiescent_for_tests() {
        assert!(done.elapsed().as_secs() < 60, "the host never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    let io = cost_counters::snapshot();
    let (walks, certificates) = tine_core::text_trace::work::take();
    check(&root);
    drop(host);
    store.close();
    Bill {
        io,
        walks,
        certificates,
        call,
    }
}

fn referrers(n: usize, blocks: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| {
            (
                format!("pages/Ref{i:05}.md"),
                format!("- links [[Target]] {i}\n").repeat(blocks),
            )
        })
        .collect()
}

fn others(n: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| {
            (
                format!("pages/Other{i:05}.md"),
                format!("- unrelated {i}\n"),
            )
        })
        .collect()
}

fn check_referrers(root: &Path, n: usize, blocks: usize) {
    for i in 0..n {
        assert_eq!(
            fs::read_to_string(root.join(format!("pages/Ref{i:05}.md"))).unwrap(),
            format!("- links [[Renamed]] {i}\n").repeat(blocks)
        );
    }
}

fn rename(n: usize) -> Bill {
    let mut files = others(40);
    files.extend(referrers(n, 1));
    files.push(("pages/Target.md".into(), "- the target\n".into()));
    measure(
        &files,
        |store, host| pages::rename_page_expected(store, host, "Target", "Renamed", None),
        |root| {
            assert!(!root.join("pages/Target.md").exists());
            assert_eq!(
                fs::read_to_string(root.join("pages/Renamed.md")).unwrap(),
                "- the target\n"
            );
            check_referrers(root, n, 1);
        },
    )
}

fn merge_rename(n: usize) -> Bill {
    merge_rename_sized(n, 1, 40)
}

/// A merge of `Target` into `Renamed` with `n` referrers of `blocks` blocks
/// each, among `unrelated` other pages.
fn merge_rename_sized(n: usize, blocks: usize, unrelated: usize) -> Bill {
    let mut files = others(unrelated);
    files.extend(referrers(n, blocks));
    files.push(("pages/Target.md".into(), "- the target\n".into()));
    files.push(("pages/Renamed.md".into(), "- the survivor\n".into()));
    measure(
        &files,
        |store, host| {
            pages::rename_or_merge_page(
                store,
                host,
                "Target",
                "Renamed",
                None,
                Some("pages/Renamed.md"),
                &[],
            )
            .map(drop)
        },
        |root| {
            assert!(!root.join("pages/Target.md").exists());
            assert_eq!(
                fs::read_to_string(root.join("pages/Renamed.md")).unwrap(),
                "- the survivor\n- the target\n"
            );
            check_referrers(root, n, blocks);
        },
    )
}

fn namespace(k: usize) -> Bill {
    namespace_sized(k, 40)
}

fn namespace_sized(k: usize, unrelated: usize) -> Bill {
    namespace_with(k, unrelated, false, Trash::Fresh)
}

/// A namespace rename of `k` children among `unrelated` other pages. With
/// `titled`, each child carries its own `title::` naming the old identity,
/// which the move rebinds to the new one.
fn namespace_with(k: usize, unrelated: usize, titled: bool, trash: Trash) -> Bill {
    let mut files = others(unrelated);
    files.push(("pages/Ns.md".into(), "- ns\n".into()));
    let head = |ns: &str, i: usize| {
        if titled {
            format!("title:: {ns}/c{i:03}\n\n")
        } else {
            String::new()
        }
    };
    for i in 0..k {
        files.push((
            format!("pages/Ns%2Fc{i:03}.md"),
            format!("{}- child {i}\n", head("Ns", i)),
        ));
    }
    files.push(("pages/Ref.md".into(), "- links [[Ns/c000]]\n".into()));
    measure_in(
        &files,
        trash,
        |store, host| pages::rename_page_expected(store, host, "Ns", "Space", None),
        |root| {
            for i in 0..k {
                assert!(!root.join(format!("pages/Ns%2Fc{i:03}.md")).exists());
                assert_eq!(
                    fs::read_to_string(root.join(format!("pages/Space%2Fc{i:03}.md"))).unwrap(),
                    format!("{}- child {i}\n", head("Space", i))
                );
            }
            assert_eq!(
                fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
                "- links [[Space/c000]]\n"
            );
        },
    )
}

fn plain_merge() -> Bill {
    let mut files = others(40);
    files.push(("pages/Source.md".into(), "- the source\n".into()));
    files.push(("pages/Survivor.md".into(), "- the survivor\n".into()));
    measure(
        &files,
        |store, host| pages::merge_pages(store, host, "pages/Source.md", "pages/Survivor.md"),
        |root| {
            assert!(!root.join("pages/Source.md").exists());
            assert_eq!(
                fs::read_to_string(root.join("pages/Survivor.md")).unwrap(),
                "- the survivor\n- the source\n"
            );
        },
    )
}

/// Per-unit counts: the growth from `few` to `many` units, divided out.
fn per_unit(few: &Bill, many: &Bill, units: u64) -> [(&'static str, u64); 10] {
    let d = |a: u64, b: u64| {
        assert_eq!(
            (b - a) % units,
            0,
            "a fractional per-unit count: {few:?} vs {many:?}"
        );
        (b - a) / units
    };
    let (f, m) = (few.io, many.io);
    [
        ("reads", d(few.reads(), many.reads())),
        ("files written", d(f.files_written, m.files_written)),
        ("fsyncs", d(f.fsyncs, m.fsyncs)),
        ("file renames", d(f.file_renames, m.file_renames)),
        ("parses", d(f.parses, m.parses)),
        // An observation re-read (the host re-reading a held page the
        // watcher saw change, contract 5.4) is billed apart from `reads`; its
        // input check is one of `parse_input_checks`, so it leaves that count
        // too (it made a run come out one over, in about one run in five).
        (
            "parse input checks",
            d(
                f.parse_input_checks - f.observation_reads,
                m.parse_input_checks - m.observation_reads,
            ),
        ),
        ("reference walks", d(few.walks, many.walks)),
        ("certificate checks", d(few.certificates, many.certificates)),
        (
            "transaction rewrites",
            d(f.transaction_rewrites, m.transaction_rewrites),
        ),
        ("directory listings", d(f.readdir, m.readdir)),
    ]
}

/// Assert each per-unit count is at most its pinned value, naming the lower
/// bound (PRINCIPLES 3). `pinned` and `bound` are in [`per_unit`]'s order.
fn pin(op: &str, counts: [(&str, u64); 10], pinned: [u64; 10], bound: [u64; 10]) {
    for (((name, got), want), lower) in counts.into_iter().zip(pinned).zip(bound) {
        assert!(
            got <= want,
            "PC-3: one {op} costs {got} {name} per unit, pinned at {want} (lower bound {lower}). \
             A count above its pin is new cruft on the whole host operation: find what \
             re-reads, re-parses or re-checks the same page"
        );
        assert!(
            got >= lower,
            "{op}: {name} {got} is below its lower bound {lower}: the counter lost a call site"
        );
    }
}

/// Per referrer, page rename and merge alike. Lower bounds: one planning
/// read plus one guarded read under the lock; one write, one publishing
/// rename and its two syncs (file, directory); one reference walk; one parse
/// (the published page's index parse); one input check of the new bytes; one
/// certificate check; no in-transaction rewrite and no directory listing.
/// Actual: five reads (planner, PagePlan staging, host admission load, final
/// pre-rename guard, publication; R5/D4R, `rename_io.rs`) with an input
/// check on each of the three page-byte reads, and two certificate checks
/// (the proposal's own at construction, the store's at preflight reuse).
const REFERRER_PINNED: [u64; 10] = [5, 1, 2, 1, 1, 3, 1, 2, 0, 0];
const REFERRER_BOUND: [u64; 10] = [2, 1, 2, 1, 1, 1, 1, 1, 0, 0];

/// Per namespace child moved. Lower bounds: two reads as a referrer; one
/// write of the destination and one rename each for it and the source's
/// trash step, with their syncs (file, directory, trash directory); one
/// reference walk; one parse; one input check; one certificate check.
/// Actual: the destination plus the source's custody marker (none on
/// Windows), three renames, ten syncs (the recoverable move protocol,
/// D-20261021-GQ3), one reference walk (the planner's proposal, carried
/// into the move step: `MoveRewrite::Prepared` on `PagePlan::move_file`) and, as for a
/// referrer, two certificate checks (the proposal's own, the store's at
/// preflight reuse) and no in-transaction rewrite.
const CHILD_PINNED: [u64; 10] = [5, 2, 10, 3, 1, 3, 1, 2, 0, 0];
const CHILD_BOUND: [u64; 10] = [2, 1, 3, 2, 1, 1, 1, 1, 0, 0];

#[test]
fn a_rename_costs_a_bounded_constant_per_referrer() {
    let _integration_guard = crate::exclusive_integration_lock();
    let (few, many) = (rename(2), rename(32));
    eprintln!("rename: 2 referrers {few:?}\n32 referrers {many:?}");
    pin(
        "page rename",
        per_unit(&few, &many, 30),
        REFERRER_PINNED,
        REFERRER_BOUND,
    );
    // The moved source's own work, once: one walk (the planner's proposal,
    // carried into its move step) and the five reads and two certificate
    // checks of a referrer.
    assert!(
        few.walks <= 3 && few.certificates <= 6 && few.reads() <= 16,
        "{few:?}"
    );
}

#[test]
fn a_merge_with_referrers_costs_a_rename_per_referrer() {
    let _integration_guard = crate::exclusive_integration_lock();
    let (few, many) = (merge_rename(2), merge_rename(32));
    eprintln!("merge: 2 referrers {few:?}\n32 referrers {many:?}");
    pin(
        "merge",
        per_unit(&few, &many, 30),
        REFERRER_PINNED,
        REFERRER_BOUND,
    );
    // The survivor and source once: each read for the proposal and guarded,
    // one certified survivor, no in-transaction rewrite.
    assert_eq!(few.io.transaction_rewrites, 0, "{few:?}");
    assert!(
        few.certificates <= 8 && few.reads() <= 22 && few.io.parses <= 5,
        "{few:?}"
    );
}

#[test]
fn a_namespace_rename_costs_a_bounded_constant_per_child() {
    let _integration_guard = crate::exclusive_integration_lock();
    let (few, many) = (namespace(2), namespace(32));
    eprintln!("namespace: 2 children {few:?}\n32 children {many:?}");
    pin(
        "namespace rename",
        per_unit(&few, &many, 30),
        CHILD_PINNED,
        CHILD_BOUND,
    );
}

/// Per namespace child moved in a graph whose trash folder already exists and
/// is settled (the common case: [`CHILD_PINNED`] measures the first trash use,
/// where each deletion syncs the folder's three ancestors too). The trash
/// sync is then the folder alone: one fsync, not four. Fixture note: the
/// 10 above includes those 3 only because the fixture's first deletion
/// creates the folder.
const CHILD_STEADY_PINNED: [u64; 10] = [5, 2, 7, 3, 1, 3, 1, 2, 0, 0];

#[test]
fn a_namespace_rename_in_a_settled_graph_syncs_the_trash_once_per_child() {
    let _integration_guard = crate::exclusive_integration_lock();
    let run = |k| namespace_with(k, 40, false, Trash::Settled);
    let (few, many) = (run(2), run(32));
    eprintln!("settled namespace: 2 children {few:?}\n32 children {many:?}");
    let counts = per_unit(&few, &many, 30);
    pin(
        "namespace rename in a settled graph",
        counts,
        CHILD_STEADY_PINNED,
        CHILD_BOUND,
    );
    // Exactly the first-use case's ten, less the trash folder's three
    // ancestors: pin() only bounds above, and a lost sync is a protocol change.
    assert_eq!(counts[2], ("fsyncs", 7), "{few:?} vs {many:?}");
}

/// PC-3, shared rename map and O(1) title match. A namespace rename queues
/// one move per child, all for one map of K+1 pairs. Lower bounds: no copy of
/// the map (the planner builds it once and the steps share it), and for the
/// moved pages' own titles one index of the map's pairs plus one probe per
/// titled child. Before, every move copied the map (K copies of K+1 pairs) and
/// every title scanned it from the start (about K²/2 pairs in all).
#[test]
fn a_namespace_rename_shares_its_map_and_matches_titles_in_constant_time() {
    let _integration_guard = crate::exclusive_integration_lock();
    for titled in [false, true] {
        let run = |k| namespace_with(k, 40, titled, Trash::Fresh);
        let (few, many) = (run(2), run(32));
        eprintln!("titled={titled}: 2 children {few:?}\n32 children {many:?}");
        for (k, bill) in [(2u64, few), (32, many)] {
            assert_eq!(
                bill.io.rename_map_clones, 0,
                "PC-3: {k} children copied the rename map {} times; the steps share one: {bill:?}",
                bill.io.rename_map_clones
            );
            let probes = bill.io.rename_title_probes;
            let (floor, ceiling) = if titled { (k, 2 * k + 1) } else { (0, 0) };
            assert!(
                (floor..=ceiling).contains(&probes),
                "PC-3: {k} {} children matched titles with {probes} map entries examined, \
                 expected {floor}..={ceiling} (one index of the map, one probe per titled child): \
                 {bill:?}",
                if titled { "titled" } else { "untitled" },
            );
        }
    }
}

/// A merge without renames: both pages read for the proposal and guarded,
/// one certified survivor write, the source trashed. Lower bounds: two
/// planning reads plus two guarded reads; one write; no reference walk (no
/// name changes); one certificate check; two parses (both pages for the
/// header split; the survivor's index parse may reuse one).
#[test]
fn a_plain_merge_costs_a_constant() {
    let _integration_guard = crate::exclusive_integration_lock();
    let bill = plain_merge();
    eprintln!("plain merge: {bill:?}");
    let c = bill.io;
    assert_eq!(
        bill.walks, 0,
        "no name changes, no reference walk: {bill:?}"
    );
    assert_eq!(c.transaction_rewrites, 0, "{bill:?}");
    for (name, got, pinned, lower) in [
        ("reads", bill.reads(), 11, 4),
        ("certificate checks", bill.certificates, 4, 1),
        ("parses", c.parses, 3, 2),
        ("files written", c.files_written, 3, 1),
        ("file renames", c.file_renames, 4, 2),
        ("fsyncs", c.fsyncs, 16, 3),
    ] {
        assert!(
            (lower..=pinned).contains(&got),
            "PC-3: a plain merge costs {got} {name}, pinned at {pinned} (lower bound {lower}): {bill:?}"
        );
    }
}

/// PC-3 timing: the whole host operation grows linearly. A merge into a
/// survivor with 8 and with 64 referrers of 60 blocks each, and a namespace
/// rename with 8 and 64 children, on a 600-page graph: the larger is at most
/// twice the linear extrapolation of the smaller plus a fixed allowance (the
/// best of three runs each, so a loaded runner's stall does not decide it).
/// Counts catch slow cruft; this catches complexity nothing counts.
#[test]
fn whole_operations_stay_linear_in_time() {
    let _integration_guard = crate::exclusive_integration_lock();
    let best = |run: &dyn Fn() -> Bill| (0..3).map(|_| run().call).min().unwrap();
    for (op, small, large) in [
        (
            "merge",
            best(&|| merge_rename_sized(8, 60, 600)),
            best(&|| merge_rename_sized(64, 60, 600)),
        ),
        (
            "namespace rename",
            best(&|| namespace_sized(8, 600)),
            best(&|| namespace_sized(64, 600)),
        ),
    ] {
        eprintln!("{op}: 8 units {small:?}, 64 units {large:?}");
        assert!(
            large <= small * 16 + Duration::from_millis(500),
            "PC-3: a {op} of 64 units took {large:?}, beyond twice the linear growth of \
             8 units ({small:?}): something in the operation grew faster than its pages"
        );
    }
}
