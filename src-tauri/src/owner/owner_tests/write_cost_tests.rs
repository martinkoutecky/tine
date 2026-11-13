//! What a CLI write that names its page costs (PC-3, cards FUceo, FUchU): a
//! write to an existing page, a write that creates its page, and the losing
//! one of two commands that create one new page under two spellings. The
//! counts are the owner's own per-graph counts (`owner::op_counts`); the
//! timing budget follows `seen::last_changed_tests::block_dates_walk_timing_budget`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::super::op_counts::{take_counts, OpCounts};
use super::super::write::{graph_turn, tests_line_len};
use super::parallel_writes::disk;
use super::Graph;

const FILLER_PAGES: usize = 200;

/// A graph with [`FILLER_PAGES`] more pages than the fixture, served over its socket.
fn served() -> (Graph, Arc<crate::state::GraphSlot>, impl Sized) {
    let graph = Graph::new();
    for i in 0..FILLER_PAGES {
        std::fs::write(
            graph.root.join(format!("pages/filler {i}.md")),
            format!("- filler {i} links [[a]]\n"),
        )
        .unwrap();
    }
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = Arc::new(graph.slot());
    let served = super::endpoint(&graph, slot.clone());
    (graph, slot, served)
}

fn add(graph: &Graph, args: Value) -> Value {
    graph
        .connect()
        .call("block.add", args.clone())
        .unwrap_or_else(|e| panic!("{args}: {e:?}"))
}

/// Wait, on this graph's own line, until `len` verbs stand in it.
fn line_reaches(slot: &crate::state::GraphSlot, len: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while tests_line_len(slot) < len {
        assert!(Instant::now() < deadline, "the line never reached {len}");
        std::thread::yield_now();
    }
}

/// Two creators of one new page under two spellings, in a known order: both
/// resolve the name before either holds a turn (the test holds the graph's),
/// so `first` creates the page and `second` finds it in its own turn. Returns
/// the time from the release to both replies.
fn race(graph: &Graph, slot: &crate::state::GraphSlot, first: &str, second: &str) -> Duration {
    let held = graph_turn(slot).unwrap_or_else(|_| panic!("the line is empty"));
    std::thread::scope(|scope| {
        let send = |name: &str, len| {
            let args = json!({ "page": name, "content": format!("- by {name}") });
            let call = scope.spawn(move || add(graph, args));
            line_reaches(slot, len);
            call
        };
        let a = send(first, 2);
        let b = send(second, 3);
        let start = Instant::now();
        drop(held);
        let (a, b) = (a.join().unwrap(), b.join().unwrap());
        let elapsed = start.elapsed();
        assert_eq!(a["page"], b["page"], "{a} {b}");
        elapsed
    })
}

/// The upper median of `samples`.
fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

/// Timings catch added complexity nothing counts (PC-3). Each command dials the
/// owner, as `tine add` does. On the base of this budget (round 1, debug build,
/// loaded host) a command took about 50 ms and a raced pair 16 ms; the budget is
/// five times that, per command, as wide as the debug budget it copies. Each
/// kind is judged by its median, as `sorted_search_budget` is: added work slows
/// every command, while a stall of the machine (a hosted runner, the lib's
/// parallel tests beside this one) slows a few, and a mean let those decide.
/// Hosted Windows run 42741207249 failed on a raced pair's mean, 389 ms beside
/// 140 ms adds, with no samples to tell a stall from a cost; they are printed
/// now. A machine slow for more than half of a kind's commands still fails.
#[test]
fn page_creation_timing_budget() {
    const N: usize = 16;
    let (graph, slot, _served) = served();
    let time = |args: &dyn Fn(usize) -> Value| -> Vec<Duration> {
        (0..N)
            .map(|i| {
                let start = Instant::now();
                add(&graph, args(i));
                start.elapsed()
            })
            .collect()
    };
    let existing = time(&|i| json!({ "page": "a", "content": format!("- more {i}") }));
    let created = time(&|i| json!({ "page": format!("New {i}"), "content": "- new" }));
    let lost: Vec<Duration> = (0..N)
        .map(|i| race(&graph, &slot, &format!("Pair {i}"), &format!("pair {i}")))
        .collect();
    assert!(disk(&graph, "pages/Pair 0.md").contains("- by pair 0"));
    for (what, samples) in [
        ("an add to an existing page", existing),
        ("an add creating a page", created),
        ("a raced creating pair", lost),
    ] {
        let typical = median(samples.clone());
        eprintln!(
            "page creation, {N} on {FILLER_PAGES} pages: {what}: median {typical:?} of {samples:?}"
        );
        assert!(
            typical < Duration::from_millis(250),
            "{what} exceeded the 250 ms debug budget: median {typical:?} of {samples:?}"
        );
    }
}

/// The counts since the last call, for this test's graph only.
fn spent(slot: &crate::state::GraphSlot) -> OpCounts {
    take_counts(&slot.root_key)
}

const fn cost(turns: u64, views: u64, lookups: u64, saves: u64) -> OpCounts {
    OpCounts {
        turns,
        views,
        lookups,
        flushes: saves,
        reads: saves,
        writes: saves,
    }
}

/// The lower bounds of `add --page NAME` (receipt step 1). An existing page:
/// its own turn, the view the name is resolved in plus the one the content's
/// `((refs))` are checked in, one lookup, one flush, read and save. A new page:
/// the whole graph's turn, a second view and lookup in it (the first view may
/// predate a command ahead in line that created the page).
const EXISTING: OpCounts = cost(1, 2, 1, 1);
const CREATED: OpCounts = cost(1, 3, 2, 1);
/// Of two creators of one new page, the one behind writes to the page the one
/// ahead made, in the graph's turn it already holds: no second turn.
const LOST: OpCounts = cost(1, 3, 2, 1);

fn plus(a: OpCounts, b: OpCounts) -> OpCounts {
    OpCounts {
        turns: a.turns + b.turns,
        views: a.views + b.views,
        lookups: a.lookups + b.lookups,
        flushes: a.flushes + b.flushes,
        reads: a.reads + b.reads,
        writes: a.writes + b.writes,
    }
}

#[test]
fn a_write_that_names_its_page_spends_its_lower_bound() {
    let (graph, slot, _served) = served();
    spent(&slot);
    add(&graph, json!({ "page": "a", "content": "- more" }));
    assert_eq!(spent(&slot), EXISTING, "add to an existing page");
    add(&graph, json!({ "page": "Brand new", "content": "- new" }));
    assert_eq!(spent(&slot), CREATED, "add creating its page");
    add(&graph, json!({ "page": "brand NEW", "content": "- again" }));
    assert_eq!(
        spent(&slot),
        EXISTING,
        "add to that page by another spelling"
    );
    // A date is looked up as a page name and then as the day's journal.
    add(&graph, json!({ "page": "2026-11-02", "content": "- day" }));
    assert_eq!(
        spent(&slot),
        OpCounts {
            lookups: 4,
            ..CREATED
        },
        "a date"
    );
    add(
        &graph,
        json!({ "page": "Nov 2nd, 2026", "content": "- day" }),
    );
    assert_eq!(
        spent(&slot),
        OpCounts {
            lookups: 2,
            ..EXISTING
        },
        "that day again"
    );

    let create = |name: &str| graph.connect().call("page.create", json!({ "name": name }));
    assert!(create("a").is_err());
    assert_eq!(
        spent(&slot),
        cost(0, 1, 1, 0),
        "page create, existing: no turn"
    );
    create("Made").unwrap();
    assert_eq!(spent(&slot), cost(1, 2, 2, 1), "page create, new");
}

#[test]
fn the_creator_behind_in_line_writes_in_the_turn_it_holds() {
    let (graph, slot, _served) = served();
    spent(&slot);
    race(&graph, &slot, "Twin", "twin");
    // The test's own hold on the graph's turn is the one extra turn.
    assert_eq!(
        spent(&slot),
        plus(plus(CREATED, LOST), cost(1, 0, 0, 0)),
        "creator ahead plus creator behind"
    );
    let text = disk(&graph, "pages/Twin.md");
    assert!(
        text.contains("- by Twin") && text.contains("- by twin"),
        "{text}"
    );
}

/// What one `block.edit` of an existing page's block spends below the owner
/// (card DuDHQ), from the request to the host's quiescence, at every page
/// size: the old bytes parsed once (by the serializer), the new bytes twice
/// (the serializer's round-trip check, and the publication's row, R8), never
/// a third time by the owner; one name read (the owner's read of the page),
/// not a second for the written bytes. The reads are the owner's read, the
/// plan's staging, the host's admission, the save's base guard and the
/// publication read (GH #623); the files and syncs are the page's temp file
/// and its directory, and the host operation's draft (temp, directory, retire;
/// storage contract: a host operation drafts). On the base of this test the
/// owner re-parsed and re-named the bytes it had just serialized: 5 parses and
/// 2 name reads per edit, about 650 ms of a 3.8 s debug 2 MiB edit. The
/// counters are process-global: the cases run in a child process of their own.
#[test]
fn a_block_edit_spends_its_lower_bound() {
    const CHILD: &str = "TINE_BLOCK_EDIT_COST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(tine_store::test_binary::own_test_binary())
            .args([
                "--exact",
                "owner::owner_tests::write_cost::a_block_edit_spends_its_lower_bound",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let (out, err) = (
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(output.status.success(), "{out}\n{err}");
        assert!(
            out.contains("1 passed"),
            "the child ran no case:\n{out}\n{err}"
        );
        println!("{out}\n{err}");
        return;
    }
    use tine_store::cost_counters;
    let id = format!("  id:: {}\n", super::UUID);
    for (label, old, text) in [
        ("1-block", format!("- one\n{id}"), "edited".to_owned()),
        (
            "60-block",
            format!("- one\n{id}{}", "- filler block\n".repeat(59)),
            "edited".to_owned(),
        ),
        ("2 MiB", format!("- one\n{id}"), "x".repeat(2 << 20)),
        (
            "already 2 MiB",
            format!("- {}\n{id}", "x".repeat(2 << 20)),
            format!("{}!", "x".repeat(2 << 20)),
        ),
    ] {
        let graph = Graph::seeded(Default::default(), &[("pages/a.md", &old)]);
        // Every fixture file older than the watcher's racy window (TE-9).
        let aged = std::time::SystemTime::now() - Duration::from_secs(3600);
        for entry in std::fs::read_dir(graph.root.join("pages")).unwrap() {
            let file = std::fs::File::options()
                .write(true)
                .open(entry.unwrap().path());
            file.unwrap().set_modified(aged).unwrap();
        }
        graph.store.refresh(tine_store::Depth::Bytes).unwrap();
        let slot = Arc::new(graph.slot());
        let _served = super::endpoint(&graph, slot.clone());
        let mut conn = graph.connect();
        let quiet = || {
            let quiet = crate::page_commands::with_host(&slot, |host| {
                host.quiesce_for_tests(Duration::from_secs(30))
            });
            assert_eq!(quiet, Ok(true), "{label}: the host settles");
        };
        quiet();
        spent(&slot);
        cost_counters::reset();
        let start = Instant::now();
        let written = conn
            .call("block.edit", json!({ "handle": super::UUID, "text": text }))
            .unwrap();
        let elapsed = start.elapsed();
        quiet();
        let c = cost_counters::snapshot();
        assert_eq!(written["kind"], "written", "{label}: {written}");
        let new = std::fs::metadata(graph.root.join("pages/a.md"))
            .unwrap()
            .len();
        println!("{label}: {new} B page, {elapsed:?}, {c:?}");
        let counts = (
            c.parses,
            c.old_source_parses,
            c.full_reads,
            c.store_reads,
            c.preamble_reads,
            c.readdir,
        );
        assert_eq!(
            counts,
            (4, 1, 4, 1, 1, 0),
            "{label}: parses, old-source parses, full reads, store reads, name reads, listings"
        );
        assert_eq!((c.fsyncs, c.files_written), (5, 2), "{label}: syncs, files");
        // Target is one (PC-3). Current staging and serialization each call
        // PagePlan::path; pin both and report the excess, without changing I/O.
        assert_eq!(c.save_path_resolutions, 2, "{label}: validated save paths");
        // The page, and the draft carrying the new and the old page under its header.
        let carried = 2 * new + old.len() as u64;
        let header = c.bytes_written.checked_sub(carried);
        assert!(
            header.is_some_and(|header| header <= 256),
            "{label}: {} bytes written for a {new} B page over {} B",
            c.bytes_written,
            old.len()
        );
        assert_eq!(
            spent(&slot),
            cost(1, 1, 1, 1),
            "{label}: the owner's own counts"
        );
    }
}
