//! Uncommitted research probes for discussion #901. All pages are synthetic.
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};
use serde_json::{json, Value};
use tine_core::sync_diff::{DiffRow, RowKind};
use tine_graph_features::{conflicts, live_conflict};
use tine_store::{PageHost, PageId, Store};

const WIN: &str = "pages/Probe.md";
const COPY: &str = "pages/Probe.sync-conflict-20261107-200000-ABCDEFG.md";

fn artifacts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/artifacts/research-concord-suggestions-901")
}

// Mirrors DiffRows.seedSuggestedOrNoLoss, including its parent decisions.
fn seed(rows: &[DiffRow], mode: &str, out: &mut HashMap<String, String>) {
    for row in rows {
        if row.kind != RowKind::Unchanged {
            let decision = if mode == "initial" {
                row.suggestion.as_deref().unwrap_or(match row.kind {
                    RowKind::Added => "mine",
                    RowKind::Removed => "theirs",
                    _ => "both",
                })
            } else {
                mode
            };
            out.insert(row.id.clone(), decision.to_owned());
        }
        seed(&row.children, mode, out);
    }
}

// Research prototype only: an equal parent body can compose child decisions
// without copying both subtrees. Does not infer identity for unmatched blocks.
fn recurse_equal_bodies(rows: &[DiffRow], out: &mut HashMap<String, String>) {
    for row in rows {
        if row.kind == RowKind::Modified && row.mine.as_ref().zip(row.theirs.as_ref())
            .is_some_and(|(m, t)| m.text == t.text) {
            out.insert(row.id.clone(), "mine".to_owned());
        }
        recurse_equal_bodies(&row.children, out);
    }
}

fn fixture(name: &str, base: &str, mine: &str, theirs: &str) -> Value {
    let mut reviews = Vec::new();
    for with_base in [false, true] {
        let bases = if with_base { vec![base.to_owned()] } else { vec![] };
        let mut outcomes = serde_json::Map::new();
        let mut review = None;
        for mode in ["initial", "both", "mine", "theirs", "prototype-recursive-equal-bodies"] {
            let scratch = tempfile::Builder::new().prefix("synthetic-").tempdir_in(artifacts()).unwrap();
            let root = scratch.path();
            for dir in ["pages", "journals", "assets", "logseq", "app-data"] {
                fs::create_dir_all(root.join(dir)).unwrap();
            }
            // Fixture setup simulates provider input. Application writes below use PageHost.
            fs::write(root.join(WIN), mine).unwrap();
            fs::write(root.join(COPY), theirs).unwrap();
            let store = Arc::new(Store::open(root, Default::default()).unwrap().0);
            let host = PageHost::start_for_tests(&store, &root.join("app-data")).unwrap();
            let diff = conflicts::sync_conflict_diff(&store, WIN, COPY, &bases).unwrap().unwrap();
            let mut decisions = HashMap::new();
            seed(&diff.rows, if mode.starts_with("prototype") { "initial" } else { mode }, &mut decisions);
            if mode.starts_with("prototype") {
                recurse_equal_bodies(&diff.rows, &mut decisions);
            }
            conflicts::resolve_sync_conflict(&store, &host, WIN, COPY, &decisions,
                &diff.base_rev, &diff.conflict_rev, diff.merge_base_rev.as_deref(), &bases,
                if mode == "mine" || mode == "theirs" { mode } else { "union" }).unwrap();
            let written = fs::read_to_string(root.join(WIN)).unwrap();
            assert!(!root.join(COPY).exists(), "resolved copy must be retired");
            // Whole-side toolbar choices must reproduce exactly that side's parsed tree.
            if mode == "mine" || mode == "theirs" {
                let expected = tine_core::doc::parse(if mode == "mine" { mine } else { theirs });
                assert_eq!(tine_core::doc::parse(&written).roots, expected.roots, "{name}/{mode}");
            }
            outcomes.insert(mode.into(), json!({"decisions": decisions, "disk": written}));
            review = Some(diff);
            drop(host);
            store.close();
        }
        let diff = review.unwrap();
        println!("{name} base={with_base} three_way={} initial={:?}", diff.three_way, outcomes["initial"]["disk"]);
        reviews.push(json!({"base_available": with_base, "diff": diff, "outcomes": outcomes}));
    }
    let value = json!({"name": name, "base": base, "mine": mine, "theirs": theirs, "reviews": reviews});
    fs::write(artifacts().join(format!("{name}.json")), serde_json::to_string_pretty(&value).unwrap()).unwrap();
    value
}

fn initial(case: &Value, with_base: bool) -> &str {
    case["reviews"][usize::from(with_base)]["outcomes"]["initial"]["disk"].as_str().unwrap()
}

#[test]
fn research_901_a_one_block_edited_on_both_devices() {
    let _guard = crate::integration_lock();
    fs::create_dir_all(artifacts()).unwrap();
    let overlap = fixture("a-overlapping", "- You upload a chat export to the archive every Monday.\n",
        "- You upload a chat export to the archive every Tuesday.\n",
        "- You upload a chat export to the archive every Friday.\n");
    assert_eq!(tine_core::doc::parse(initial(&overlap, true)).roots.len(), 2);
    let disjoint = fixture("a-disjoint", "- You upload a chat export to the archive every Monday.\n",
        "- We upload a chat export to the archive every Monday.\n",
        "- You upload a chat export to the archive every Friday.\n");
    assert_eq!(tine_core::doc::parse(initial(&disjoint, true)).roots.len(), 1);
    assert_eq!(tine_core::doc::parse(initial(&disjoint, false)).roots.len(), 2);
    let rewrite = fixture("a-unpaired-rewrites", "- You upload a chat export to the archive every Monday.\n",
        "- Upload the exported conversation into our weekly storage folder.\n",
        "- Archive your chat transcript using the new import workflow.\n");
    let rows = rewrite["reviews"][1]["diff"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r["suggestion"].is_string()), "both unmatched replacements are suggested as additions");
    assert_eq!(tine_core::doc::parse(initial(&rewrite, true)).roots.len(), 2);
    let triangle = fixture("a-base-pairs-but-sides-do-not", "- abcdefghij klmnopqrst uvwxyzabcd efghijklmn opqrstuvwx\n",
        "- 123456ghij klmnopqrst uvwxyzabcd efghijklmn opqrstuvwx\n",
        "- abcdefghij klmnopqrst uvwxyzabcd efghijklmn opqr123456\n");
    assert_eq!(triangle["reviews"][1]["diff"]["rows"].as_array().unwrap().len(), 2);
    assert_eq!(tine_core::doc::parse(initial(&triangle, true)).roots.len(), 2);
    fixture("a-persisted-id-rewrites", "- You upload a chat export to the archive every Monday.\n  id:: 11111111-1111-4111-8111-111111111111\n",
        "- Upload the exported conversation into our weekly storage folder.\n  id:: 11111111-1111-4111-8111-111111111111\n",
        "- Archive your chat transcript using the new import workflow.\n  id:: 11111111-1111-4111-8111-111111111111\n");
}

#[test]
fn research_901_b_rewrite_reorder_and_additions() {
    let _guard = crate::integration_lock();
    fs::create_dir_all(artifacts()).unwrap();
    fixture("b-flat-rewrite-add", "- Project alpha legacy outline\n- Project beta legacy outline\n",
        "- Project alpha legacy outline\n- Project beta legacy outline\n- Local capture to retain\n",
        "- Revised beta plan for launch\n- Revised alpha plan for launch\n");
    let nested = fixture("b-nested-rewrite-add", "- Projects\n  - Project alpha legacy outline\n  - Project beta legacy outline\n",
        "- Projects\n  - Project alpha legacy outline\n  - Project beta legacy outline\n  - Local capture to retain\n",
        "- Projects\n  - Revised beta plan for launch\n  - Revised alpha plan for launch\n");
    assert_eq!(tine_core::doc::parse(initial(&nested, true)).roots.len(), 2);
    fixture("b-flat-reorder-add", "- Alpha project\n- Beta project\n- Gamma project\n",
        "- Alpha project\n- Beta project\n- Gamma project\n- Local capture to retain\n",
        "- Gamma project\n- Alpha project\n- Beta project\n");
    // K=2 history candidates are not proof of a real common ancestor.
    fixture("b-wrong-ledger-candidate", "- unrelated previous edition\n",
        "- old local structure\n- Local capture to retain\n", "- new synced structure\n");
    fixture("b-atomic-section-rewrite", "- Old alpha outline\n  - Old detail\n",
        "- Old alpha outline\n  - Old detail\n  - Local capture to retain\n",
        "- Completely different replacement structure\n  - New detail\n");
}

#[test]
fn research_901_c_neighbouring_edits_flat_and_nested() {
    let _guard = crate::integration_lock();
    fs::create_dir_all(artifacts()).unwrap();
    let flat = fixture("c-flat-neighbours", "- Prepare a detailed weekly report\n- Publish the status notes every Monday\n",
        "- Prepare a detailed monthly report\n- Publish the status notes every Monday\n",
        "- Prepare a detailed weekly report\n- Publish the status notes every Friday\n");
    assert_eq!(tine_core::doc::parse(initial(&flat, true)).roots.len(), 2);
    let nested = fixture("c-nested-neighbours", "- Projects\n  - Prepare a detailed weekly report\n  - Publish the status notes every Monday\n",
        "- Projects\n  - Prepare a detailed monthly report\n  - Publish the status notes every Monday\n",
        "- Projects\n  - Prepare a detailed weekly report\n  - Publish the status notes every Friday\n");
    assert_eq!(tine_core::doc::parse(initial(&nested, true)).roots.len(), 2, "the unchanged parent body duplicates");
    assert_eq!(nested["reviews"][1]["diff"]["rows"][0]["children"].as_array().unwrap().len(), 2);
    assert!(nested["reviews"][1]["diff"]["rows"][0].get("suggestion").is_none());
    let prototype = nested["reviews"][1]["outcomes"]["prototype-recursive-equal-bodies"]["disk"].as_str().unwrap();
    assert_eq!(prototype, "- Projects\n  - Prepare a detailed monthly report\n  - Publish the status notes every Friday\n");
    // A different child decision has no effect while the parent is Both.
    let base = tine_core::doc::parse(nested["base"].as_str().unwrap());
    let mine = tine_core::doc::parse(nested["mine"].as_str().unwrap());
    let theirs = tine_core::doc::parse(nested["theirs"].as_str().unwrap());
    let diff = tine_core::sync_diff::diff3_docs(&base, &mine, &theirs);
    let mut first = HashMap::new();
    seed(&diff.rows, "initial", &mut first);
    let mut second = first.clone();
    second.insert("0.0".into(), "theirs".into());
    assert_eq!(tine_core::sync_diff::merge_blocks3(Some(&base.roots), &mine.roots, &theirs.roots, None, &first).unwrap(),
        tine_core::sync_diff::merge_blocks3(Some(&base.roots), &mine.roots, &theirs.roots, None, &second).unwrap());
}

#[test]
fn research_901_d_move_and_edit() {
    let _guard = crate::integration_lock();
    fs::create_dir_all(artifacts()).unwrap();
    fixture("d-sibling-move-edit", "- Anchor alpha\n- Prepare a detailed weekly report\n- Anchor omega\n",
        "- Prepare a detailed weekly report\n- Anchor alpha\n- Anchor omega\n",
        "- Anchor alpha\n- Prepare a detailed monthly report\n- Anchor omega\n");
    let with_id = fixture("d-sibling-move-edit-with-id", "- Anchor alpha\n- Prepare a detailed weekly report\n  id:: 11111111-1111-4111-8111-111111111111\n- Anchor omega\n",
        "- Prepare a detailed weekly report\n  id:: 11111111-1111-4111-8111-111111111111\n- Anchor alpha\n- Anchor omega\n",
        "- Anchor alpha\n- Prepare a detailed monthly report\n  id:: 11111111-1111-4111-8111-111111111111\n- Anchor omega\n");
    // Characterizes an existing safety defect, not desired behavior: both
    // sides still contain alpha, but conflicting move classifications delete it.
    assert!(!initial(&with_id, true).contains("Anchor alpha"));
    assert!(initial(&with_id, false).contains("Anchor alpha"));
    fixture("d-cross-parent-move-edit-with-id", "- Inbox\n  - Prepare a detailed weekly report\n    id:: 11111111-1111-4111-8111-111111111111\n- Archive\n",
        "- Inbox\n- Archive\n  - Prepare a detailed weekly report\n    id:: 11111111-1111-4111-8111-111111111111\n",
        "- Inbox\n  - Prepare a detailed monthly report\n    id:: 11111111-1111-4111-8111-111111111111\n- Archive\n");
}

#[test]
fn research_901_live_rewrite_and_ignored_child_decisions() {
    let _guard = crate::integration_lock();
    fs::create_dir_all(artifacts()).unwrap();
    let base = "- Projects\n  - Old section alpha\n  - Old section beta\n";
    let mine = "- Projects\n  - Old section alpha\n  - Old section beta\n  - Local capture to retain\n";
    let theirs = "- Projects\n  - New outline first\n  - New outline second\n";
    let mut results = Vec::new();
    for with_base in [false, true] {
    for mode in ["initial", "synced-parent", "synced-all"] {
    let scratch = tempfile::Builder::new().prefix("live-").tempdir_in(artifacts()).unwrap();
    for dir in ["pages", "journals", "assets", "logseq", "app-data"] {
        fs::create_dir_all(scratch.path().join(dir)).unwrap();
    }
    fs::write(scratch.path().join(WIN), mine).unwrap();
    let store = Arc::new(Store::open(scratch.path(), Default::default()).unwrap().0);
    let draft = store.page(&PageId::from(WIN)).unwrap().doc;
    let base_rev: String = tine_store::FileRev::from_bytes(base.as_bytes()).into();
    fs::write(scratch.path().join(WIN), theirs).unwrap();
    let bases = if with_base { vec![base.to_owned()] } else { vec![] };
    let diff = live_conflict::live_conflict_diff(&store, WIN, &draft, Some(&base_rev), &bases).unwrap();
    let mut decisions = HashMap::new();
    seed(&diff.rows, if mode == "synced-all" { "theirs" } else { "initial" }, &mut decisions);
    // User selects the synced parent body and keeps local new blocks below.
    if mode == "synced-parent" { decisions.insert("0".into(), "theirs".into()); }
    let merged = live_conflict::merge_live_conflict(&store, WIN, &draft, &diff.conflict_rev,
        diff.merge_base_rev.as_deref(), &bases, &decisions, "union").unwrap();
    let host = PageHost::start_for_tests(&store, &scratch.path().join("app-data")).unwrap();
    // Same guarded host writer used by existing live-conflict integration tests.
    crate::test_host_writer::save_on(&host, &store,
        tine_store::EditKind::ReplacePage, &PageId::from(WIN),
        tine_store::SaveBase::Existing(tine_store::FileRev::from(diff.conflict_rev.clone())), &merged).unwrap();
    let disk = fs::read_to_string(scratch.path().join(WIN)).unwrap();
    println!("live base={with_base} mode={mode}: {disk:?}");
    if mode == "synced-parent" {
        assert_eq!(disk.contains("Old section alpha"), !with_base);
        assert!(disk.contains("New outline first") && disk.contains("Local capture"));
    }
    if mode == "synced-all" { assert!(!disk.contains("Local capture")); }
    results.push(json!({"base_available": with_base, "mode": mode, "diff": diff, "decisions": decisions, "disk": disk}));
    drop(host);
    store.close();
    }
    }
    let value = json!({"base": base, "mine": mine, "theirs": theirs, "results": results});
    fs::write(artifacts().join("live-rewrite.json"), serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

#[test]
fn research_901_base_selection_and_identical_control() {
    let _guard = crate::integration_lock();
    fs::create_dir_all(artifacts()).unwrap();
    let base = "- You upload a chat export to the archive every Monday.\n";
    let mine = "- You upload a chat export to the archive every Tuesday.\n";
    let theirs = "- You upload a chat export to the archive every Friday.\n";
    let scratch = tempfile::Builder::new().prefix("selection-").tempdir_in(artifacts()).unwrap();
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(scratch.path().join(dir)).unwrap();
    }
    fs::write(scratch.path().join(WIN), mine).unwrap();
    fs::write(scratch.path().join(COPY), theirs).unwrap();
    let store = Store::open(scratch.path(), Default::default()).unwrap().0;
    let mut results = Vec::new();
    for (label, candidates, expected) in [
        ("none", vec![], false),
        ("winner admission only", vec![mine.to_owned()], false),
        ("copy artifact before older base", vec![mine.to_owned(), theirs.to_owned(), base.to_owned()], false),
        ("ancestor after winner admission", vec![mine.to_owned(), base.to_owned()], true),
    ] {
        let diff = conflicts::sync_conflict_diff(&store, WIN, COPY, &candidates).unwrap().unwrap();
        assert_eq!(diff.three_way, expected, "{label}");
        results.push(json!({"label": label, "candidates": candidates, "diff": diff}));
    }
    store.close();
    fs::write(artifacts().join("base-selection.json"), serde_json::to_string_pretty(&results).unwrap()).unwrap();
    let identical = fixture("identical-control", base, mine, mine);
    for review in identical["reviews"].as_array().unwrap() {
        assert_eq!(review["diff"]["blocks_identical"], true);
        assert_eq!(review["outcomes"]["initial"]["disk"], mine);
    }
}
