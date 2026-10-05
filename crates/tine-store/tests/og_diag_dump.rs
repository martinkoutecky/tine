//! GH #623 follow-up: the diagnostics dump carries launch timings and graph
//! shape as numbers only. I-5 (privacy boundary): no page name, path or text.
use serde_json::Value;
use std::fs;
use std::time::{Duration, Instant, SystemTime};
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
    // Outside the 2 s racy window (§5.4): fresh fixture files would draw the
    // watcher's racy follow-up full diff about 2 s after open, which lands
    // between a test's `before` count and its assertion under load.
    let old = SystemTime::now() - Duration::from_secs(3600);
    for sub in ["pages", "journals"] {
        for entry in fs::read_dir(root.join(sub)).unwrap() {
            fs::File::options()
                .write(true)
                .open(entry.unwrap().path())
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
    }
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
        "rebuild_command",
        "load_recovery",
        "watch_install",
        "watch_rescan_event",
        "poll_cycle",
        "launch_diff",
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
    // GH #623: the watcher baseline comes from the load pass's own stamps and
    // revisions, so neither the open-time baseline walk nor the fill_revs hash
    // pass runs (each is reported only when its fallback does), and the launch
    // diff stats every file once before Ready.
    assert!(
        launch["baselineWalk"].is_null(),
        "no baseline walk: {launch}"
    );
    assert!(launch["fillRevs"].is_null(), "no fill_revs pass: {launch}");
    let launch_diff = dump["fullDiffs"]["recent"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diff| diff["trigger"] == "launch_diff")
        .expect("the launch diff is recorded");
    assert!(n(&launch_diff["files"]) >= 5);
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
}

fn latest_diff(dump: &Value, trigger: &str) -> Option<Value> {
    dump["fullDiffs"]["recent"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|diff| diff["trigger"] == trigger)
        .cloned()
}

#[test]
fn rescan_and_saves_are_recorded() {
    let (_dir, store) = fixture();
    let before = n(&store.diagnostics()["fullDiffs"]["total"]);
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let dump = store.diagnostics();
    // The poll watcher runs its own full diff every cycle, so an entry is found
    // by its trigger, not assumed to be the last one.
    assert!(n(&dump["fullDiffs"]["total"]) > before);
    let rescan = latest_diff(&dump, "rescan_command").expect("rescan recorded");
    assert!(n(&rescan["files"]) >= 5);

    // The Settings button is the forced rebuild and says so in the report.
    store.refresh(tine_store::Depth::Rebuild).unwrap();
    let rebuilt = latest_diff(&store.diagnostics(), "rebuild_command").expect("rebuild recorded");
    assert!(n(&rebuilt["files"]) >= 5);

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

/// ADR 0070: the checkpoint section is numbers and closed tokens too, after a
/// write and a launch served from the checkpoint.
#[test]
fn the_checkpoint_section_is_closed_tokens_only() {
    let (dir, store) = fixture();
    let cp_dir = tempfile::tempdir().unwrap();
    let cp = cp_dir.path().join("graph.bin");
    store.close();
    let open = || {
        Store::open(
            dir.path(),
            tine_store::OpenOptions {
                launch_checkpoint: Some(cp.clone()),
                ..Default::default()
            },
        )
        .unwrap()
        .0
    };
    let first = open();
    first.refresh(tine_store::Depth::Stamps).unwrap();
    assert!(matches!(
        first.write_checkpoint_now(),
        Some(tine_store::CheckpointWrite::Written { .. })
    ));
    let written = first.diagnostics()["checkpoint"].clone();
    assert_eq!(written["last"]["outcome"], "written");
    assert!(n(&written["last"]["fileBytes"]) > 0);
    first.close();
    let second = open();
    second.refresh(tine_store::Depth::Stamps).unwrap();
    let section = second.diagnostics()["checkpoint"].clone();
    assert_eq!(section["load"]["outcome"], "loaded");
    assert!(n(&section["load"]["bytes"]) > 0);
    let text = format!("{written}{section}");
    for planted in [SECRET_NAME, SECRET_BODY, &dir.path().to_string_lossy()] {
        assert!(
            !text.contains(planted),
            "I-5: checkpoint diagnostics leaked {planted:?}: {text}"
        );
    }
    fn strings(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::String(text) => out.push(text.clone()),
            Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
            Value::Object(map) => map.values().for_each(|item| strings(item, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    strings(&written, &mut found);
    strings(&section, &mut found);
    let closed = [
        "loaded",
        "missing",
        "unreadable",
        "format",
        "parser",
        "root",
        "config",
        "length",
        "checksum",
        "decode",
        "raced",
        "written",
        "failed",
        "loading",
        "unpublished",
    ];
    for text in found {
        assert!(
            closed.contains(&text.as_str()),
            "unexpected checkpoint string {text:?}"
        );
    }
}

#[test]
fn page_reads_and_asset_walks_are_recorded_for_the_focus_return_diagnosis() {
    let (_dir, store) = fixture();
    let before = store.diagnostics();
    let walks = n(&before["assetWalks"]["total"]);
    let reads = n(&before["pageWriterWaits"]["all"]["count"]);
    store.page(&PageId::from("pages/Hub.md")).unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let after = store.diagnostics();
    assert_eq!(n(&after["pageWriterWaits"]["all"]["count"]), reads + 1);
    // The watcher thread may add a walk of its own, so at least this one.
    assert!(
        n(&after["assetWalks"]["total"]) > walks,
        "a focus rescan's full asset walk is recorded"
    );
    let recent = after["assetWalks"]["recent"].as_array().unwrap();
    let walk = recent.last().unwrap();
    assert!(walk["writerWaitMs"].is_number() && walk["heldMs"].is_number());
}
