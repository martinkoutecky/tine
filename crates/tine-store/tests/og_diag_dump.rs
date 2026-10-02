//! GH #623 follow-up: the diagnostics dump carries launch timings and graph
//! shape as numbers only. I-5 (privacy boundary): no page name, path or text.
use serde_json::Value;
use std::fs;
use std::time::{Duration, Instant};
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

const SECRET_NAME: &str = "Zyxwvu-Distinctive-Page-Name";
const SECRET_BODY: &str = "quixotic-planted-body-sentence";

fn fixture() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["pages", "journals", "logseq"] {
        fs::create_dir_all(dir.path().join(sub)).unwrap();
    }
    let root = dir.path();
    fs::write(
        root.join(format!("pages/{SECRET_NAME}.md")),
        format!("- {SECRET_BODY}\n  key:: value\n  - nested [[Hub]] #tag\n    - deeper ((00000000-0000-4000-8000-000000000001))\n"),
    )
    .unwrap();
    fs::write(
        root.join("pages/Hub.md"),
        format!("- links back to [[{SECRET_NAME}]]\n- {{{{query (todo TODO)}}}}\n"),
    )
    .unwrap();
    fs::write(root.join("pages/Windowsline.md"), "- one\r\n- two\r\n").unwrap();
    fs::write(
        root.join("pages/Notes (conflicted copy 2026-09-01).md"),
        "- c\n",
    )
    .unwrap();
    fs::write(root.join("pages/Cafe\u{301}.md"), "- decomposed accent\n").unwrap();
    fs::write(root.join("journals/2026_09_20.md"), "- a day\n").unwrap();
    let store = Store::open(root, Default::default()).unwrap().0;
    let deadline = Instant::now() + Duration::from_secs(60);
    while !store.is_graph_ready().unwrap() {
        assert!(Instant::now() < deadline, "graph never became ready");
        std::thread::sleep(Duration::from_millis(5));
    }
    (dir, store)
}

fn n(value: &Value) -> u64 {
    value
        .as_u64()
        .unwrap_or_else(|| panic!("not a number: {value}"))
}

#[test]
fn the_dump_never_names_the_graph() {
    let (dir, store) = fixture();
    let dump = store.diagnostics().to_string();
    let root = dir.path().to_string_lossy().to_string();
    for planted in [
        SECRET_NAME,
        SECRET_BODY,
        "Hub",
        "Windowsline",
        "decomposed",
        "conflicted",
        &root,
    ] {
        assert!(
            !dump.to_lowercase().contains(&planted.to_lowercase()),
            "I-5: diagnostics output leaked {planted:?}: {dump}"
        );
    }
    // Numbers, booleans, null and the closed vocabulary only: every string
    // value is one of the status/outcome/trigger tokens.
    fn strings(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::String(text) => out.push(text.clone()),
            Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
            Value::Object(map) => map.values().for_each(|item| strings(item, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    strings(&store.diagnostics(), &mut found);
    let closed = [
        "ready",
        "loading",
        "failed",
        "closed",
        "installed",
        "cache_already_built",
        "file_changed",
        "install_declined",
        "cancelled",
        "rescan_command",
        "load_recovery",
        "watch_install",
        "watch_rescan_event",
        "poll_cycle",
    ];
    for text in found {
        assert!(
            closed.contains(&text.as_str()),
            "unexpected string value {text:?}"
        );
    }
}

#[test]
fn shape_counts_what_the_graph_holds() {
    let (_dir, store) = fixture();
    let shape = store.diagnostics()["shape"].clone();
    assert_eq!(shape["ready"], true);
    assert_eq!(n(&shape["journals"]), 1);
    assert_eq!(n(&shape["conflictNamedFiles"]), 1);
    assert_eq!(n(&shape["nonNfcFileNames"]), 1);
    assert_eq!(n(&shape["crlfFilesAtLastLoad"]), 1);
    assert_eq!(n(&shape["maxNestingDepth"]["max"]), 3);
    assert!(n(&shape["linksPerPage"]["max"]) >= 1);
    assert_eq!(n(&shape["tagsPerPage"]["max"]), 1);
    assert_eq!(n(&shape["blockRefsPerPage"]["max"]), 1);
    assert_eq!(n(&shape["queriesPerPage"]["max"]), 1);
    assert_eq!(n(&shape["propertiesPerPage"]["max"]), 1);
    // Hub links to the planted page and is linked from it: one referrer each.
    assert_eq!(n(&shape["inDegree"]["max"]), 1);
    assert_eq!(
        n(&shape["fileBytes"]["n"]),
        n(&shape["fileBytes"]["n"]).max(1)
    );
    assert!(n(&shape["fileBytes"]["max"]) > 0);
}

#[test]
fn launch_phases_separate_reading_from_parsing() {
    let (_dir, store) = fixture();
    let dump = store.diagnostics();
    let launch = &dump["launch"];
    assert_eq!(launch["status"], "ready");
    assert!(launch["readyMs"].as_f64().is_some(), "ready time recorded");
    assert!(launch["openMs"].as_f64().is_some(), "open time recorded");
    assert!(launch["baselineWalk"]["files"].as_u64().unwrap() >= 5);
    let passes = launch["loadPasses"].as_array().unwrap();
    let pass = passes
        .iter()
        .find(|pass| pass["outcome"] == "installed")
        .expect("an installed pass");
    assert!(n(&pass["read"]["files"]) >= 5 && n(&pass["read"]["bytes"]) > 0);
    assert!(pass["read"]["ms"].is_number() && pass["parse"]["ms"].is_number());
    assert!(n(&pass["parse"]["files"]) >= 5);
    // Read and parse are SUMMED worker-thread time; the parallel wall time and
    // worker count ride beside them so the sums are interpretable.
    assert!(pass["parallel"]["wallMs"].is_number());
    assert!(n(&pass["parallel"]["workers"]) >= 1);
    assert!(n(&launch["fillRevs"]["files"]) >= 5);
}

#[test]
fn rescan_and_saves_are_recorded() {
    let (_dir, store) = fixture();
    let before = n(&store.diagnostics()["fullDiffs"]["total"]);
    store.scan_refresh().unwrap();
    let dump = store.diagnostics();
    assert_eq!(n(&dump["fullDiffs"]["total"]), before + 1);
    let last = dump["fullDiffs"]["recent"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(last["trigger"], "rescan_command");
    assert!(n(&last["files"]) >= 5);

    let id = PageId::from("pages/Hub.md");
    let mut read = store.page(&id).unwrap();
    read.doc.blocks[0].raw = "edited".into();
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &read.doc
        ),
        SaveOutcome::Saved(_)
    ));
    let saves = store.diagnostics()["saves"].clone();
    assert!(n(&saves["total"]) >= 1);
    let last = saves["recent"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["committed"], true);
    assert!(last["writerWaitMs"].is_number() && last["totalMs"].is_number());
}
