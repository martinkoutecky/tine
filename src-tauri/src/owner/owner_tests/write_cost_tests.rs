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

/// Timings catch added complexity nothing counts (PC-3). Each command dials the
/// owner, as `tine add` does. On the base of this budget (round 1, debug build,
/// loaded host) a command took about 50 ms and a raced pair 16 ms; the budget is
/// five times that, per command, as wide as the debug budget it copies.
#[test]
fn page_creation_timing_budget() {
    const N: usize = 16;
    let (graph, slot, _served) = served();
    let time = |args: &dyn Fn(usize) -> Value| {
        let start = Instant::now();
        for i in 0..N {
            add(&graph, args(i));
        }
        start.elapsed()
    };
    let existing = time(&|i| json!({ "page": "a", "content": format!("- more {i}") }));
    let created = time(&|i| json!({ "page": format!("New {i}"), "content": "- new" }));
    let lost: Duration = (0..N)
        .map(|i| race(&graph, &slot, &format!("Pair {i}"), &format!("pair {i}")))
        .sum();
    assert!(disk(&graph, "pages/Pair 0.md").contains("- by pair 0"));
    eprintln!(
        "page creation, {N} each on {FILLER_PAGES} pages: existing {:?}/add, created {:?}/add, \
         raced pair {:?}",
        existing / N as u32,
        created / N as u32,
        lost / N as u32,
    );
    for (what, elapsed) in [
        ("an add to an existing page", existing),
        ("an add creating a page", created),
        ("a raced creating pair", lost),
    ] {
        let each = elapsed / N as u32;
        assert!(
            each < Duration::from_millis(250),
            "{what} exceeded the 250 ms debug budget: {each:?}"
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
