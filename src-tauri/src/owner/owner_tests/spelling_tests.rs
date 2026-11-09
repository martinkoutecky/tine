//! Which page a name passed to the CLI means when that page does not exist yet
//! (cards FUceo, FUchU), against a real owner over its socket. Two spellings of
//! one new page are one page whether the commands run in turn or at once; a
//! date no ordinary page claims is that day's journal, as `[[date]]` is in the
//! app (`WholeGraph::link_target`).

use std::sync::Arc;

use serde_json::{json, Value};
use tine_store::{PageId, Resolved};

use super::super::client::Failure;
use super::parallel_writes::{all_landed, at_once, disk, failure_code, ROUNDS, WRITERS};
use super::Graph;

/// The files in `dir` whose name, ignoring case, is `stem.md`.
fn files_named(graph: &Graph, dir: &str, stem: &str) -> Vec<String> {
    let want = format!("{}.md", stem.to_lowercase());
    let Ok(entries) = std::fs::read_dir(graph.root.join(dir)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.to_lowercase() == want)
        .map(|name| format!("{dir}/{name}"))
        .collect()
}

fn add(graph: &Graph, args: Value) -> Result<Value, Failure> {
    graph.connect().call("block.add", args)
}

/// The page `[[name]]` opens in the app: the one answer (`link_target`), then
/// the file (or the file a new page would get) that answer names.
fn app_opens(graph: &Graph, name: &str) -> String {
    let view = graph.store.whole_graph().unwrap();
    let link = view.link_target(name);
    match view.resolve(&link.name, link.is_journal) {
        Resolved::Existing { id, .. } | Resolved::Absent { id } => id.as_str().to_owned(),
        Resolved::Alias { owners } => owners[0].as_str().to_owned(),
    }
}

/// Two spellings of one absent page, written at once, are one page (FUceo).
/// Run in turn they already were: the second finds the first's file.
#[test]
fn parallel_adds_to_two_spellings_of_an_absent_page_land_in_one_page() {
    let graph = Graph::new();
    let slot = Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot);
    for round in 0..ROUNDS {
        let args: Vec<Value> = (0..WRITERS)
            .map(|i| {
                let name = if i % 2 == 0 { "Twin" } else { "twin" };
                json!({ "page": format!("{name} {round}"), "content": format!("- block {i}") })
            })
            .collect();
        let results = at_once(&graph, "block.add", &args);
        all_landed(&results, &format!("round {round}"));
        let files = files_named(&graph, "pages", &format!("twin {round}"));
        assert_eq!(files.len(), 1, "round {round}: {files:?}");
        let text = disk(&graph, &files[0]);
        for i in 0..WRITERS {
            let line = format!("- block {i}\n");
            assert_eq!(text.matches(&line).count(), 1, "{line}in\n{text}");
        }
    }
    for name in ["Seq", "seq"] {
        add(
            &graph,
            json!({ "page": name, "content": format!("- {name}") }),
        )
        .unwrap();
    }
    assert_eq!(files_named(&graph, "pages", "seq").len(), 1);
}

/// `page create` for two spellings at once: one creates the page, the rest are
/// refused as `page-exists`, not as the store's twin check.
#[test]
fn parallel_creates_of_two_spellings_make_one_page_and_refuse_the_rest() {
    let graph = Graph::new();
    let slot = Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot);
    for round in 0..ROUNDS {
        let args: Vec<Value> = (0..WRITERS)
            .map(|i| {
                let name = if i % 2 == 0 { "Made" } else { "made" };
                json!({ "name": format!("{name} {round}") })
            })
            .collect();
        let results = at_once(&graph, "page.create", &args);
        let created = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(created, 1, "round {round}: {results:?}");
        for refused in results.iter().filter(|r| r.is_err()) {
            assert_eq!(failure_code(refused), "refused", "round {round}");
            let Err(Failure::Owner(body)) = refused else {
                unreachable!()
            };
            assert_eq!(
                body.detail.as_ref().unwrap()["reason"],
                "page-exists",
                "round {round}: {body:?}"
            );
        }
        assert_eq!(
            files_named(&graph, "pages", &format!("made {round}")).len(),
            1,
            "round {round}"
        );
    }
}

/// What the cost costs: creating a page takes the whole graph's turn, so it
/// serializes with every writer; a write to an existing page takes its own
/// page's turn only and waits behind nothing on another page.
#[test]
fn creating_a_page_takes_the_whole_graphs_turn_and_an_existing_page_does_not() {
    let graph = Graph::new();
    let mut slot = graph.slot();
    slot.turn_wait = std::time::Duration::from_millis(150);
    let slot = Arc::new(slot);
    let (_endpoint, _gate) = super::endpoint(&graph, slot.clone());
    let held = super::super::write::page_turn(&slot, &PageId::from("pages/b.md"))
        .unwrap_or_else(|_| panic!("the line is empty"));
    add(&graph, json!({ "page": "a", "content": "- existing" }))
        .expect("an existing page waits behind no other page");
    let late = add(&graph, json!({ "page": "Brand new", "content": "- new" }));
    assert_eq!(failure_code(&late), "refused", "{late:?}");
    let Err(Failure::Owner(body)) = &late else {
        unreachable!()
    };
    assert_eq!(body.detail.as_ref().unwrap()["reason"], "page-busy");
    assert!(files_named(&graph, "pages", "brand new").is_empty());
    drop(held);
    add(&graph, json!({ "page": "Brand new", "content": "- new" })).unwrap();
}

/// A date no ordinary page claims is that day's journal, whichever way it is
/// spelled and whichever command named it first (FUchU; OG
/// `transact-file-tx-if-not-exists!` files such a page under `journals/`).
#[test]
fn a_date_nothing_claims_is_that_days_journal_for_add_and_create() {
    for spelling in ["2026_11_02", "2026-11-02", "Nov 2nd, 2026"] {
        for page_first in [true, false] {
            let graph = Graph::new();
            let slot = Arc::new(graph.slot());
            let (_endpoint, _gate) = super::endpoint(&graph, slot);
            let by_page = || add(&graph, json!({ "page": spelling, "content": "- by page" }));
            let by_day = || {
                add(
                    &graph,
                    json!({ "journal": "2026-11-02", "content": "- by day" }),
                )
            };
            let (first, second) = if page_first {
                (by_page(), by_day())
            } else {
                (by_day(), by_page())
            };
            let what = format!("{spelling:?}, page first: {page_first}");
            assert_eq!(first.unwrap()["page"], "journals/2026_11_02.md", "{what}");
            assert_eq!(second.unwrap()["page"], "journals/2026_11_02.md", "{what}");
            let text = disk(&graph, "journals/2026_11_02.md");
            assert!(
                text.contains("- by page") && text.contains("- by day"),
                "{what}: {text}"
            );
            assert!(
                files_named(&graph, "pages", "2026_11_02").is_empty(),
                "{what}: a second file under pages/"
            );
        }
    }
    let graph = Graph::new();
    let slot = Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot);
    let made = graph
        .connect()
        .call("page.create", json!({ "name": "2026_11_05" }))
        .unwrap();
    assert_eq!(made["page"], "journals/2026_11_05.md");
    let again = graph
        .connect()
        .call("page.create", json!({ "name": "Nov 5th, 2026" }));
    assert_eq!(failure_code(&again), "refused", "{again:?}");
    assert!(files_named(&graph, "pages", "2026_11_05").is_empty());
}

/// For every kind of name the CLI writes to the file `[[name]]` opens in the
/// app: no second answer to the question (AR-1).
#[test]
fn the_page_a_name_writes_to_is_the_page_the_app_opens() {
    let graph = Graph::new();
    std::fs::create_dir_all(graph.root.join("journals")).unwrap();
    std::fs::write(graph.root.join("journals/2026_11_04.md"), "- day four\n").unwrap();
    std::fs::write(graph.root.join("pages/2026_11_03.md"), "- literal\n").unwrap();
    std::fs::write(
        graph.root.join("pages/Owner.md"),
        "alias:: Shortcut\n- owner\n",
    )
    .unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot);
    let names = [
        "2026_11_02",
        "2026-11-02",
        "Nov 2nd, 2026",
        "Nothing Here",
        "nothing here",
        "2026_11_03",
        "2026-11-03",
        "Nov 3rd, 2026",
        "2026_11_04",
        "2026-11-04",
        "Nov 4th, 2026",
        "Shortcut",
        "shortcut",
        "a",
    ];
    for name in names {
        let opens = app_opens(&graph, name);
        let wrote = add(&graph, json!({ "page": name, "content": "- written" }))
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(wrote["page"], opens.as_str(), "[[{name}]]");
    }
    // The literal file keeps its own spelling (D-20261107-KD); the other
    // spellings of its day are the journal.
    assert!(disk(&graph, "pages/2026_11_03.md").contains("- written"));
    assert!(disk(&graph, "journals/2026_11_03.md").contains("- written"));
}
