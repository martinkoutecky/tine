//! The one name/day claim guard (`PagePlan::check_page_claim`, contract row
//! `check_page_claim::Twin`): what a create's proposed `title::` claims
//! (REG-CREATE-TITLE-TWIN) and what the held-unindexed part of the guard
//! costs (REG-HELD-UNINDEXED-CLAIM-COST).
use super::*;
use crate::model::GRAPH_PREAMBLE_READS;
use crate::{Refusal, Why};
use tine_core::model::{BlockDto, PageDto};

fn open(files: &[(&str, &str)]) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    for (rel, text) in files {
        fs::write(root.path().join(rel), text).unwrap();
    }
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    (root, store)
}

fn file(rel: &str) -> FileId {
    FileId::from(rel.to_owned())
}

fn reads(probe: impl FnOnce()) -> usize {
    GRAPH_PREAMBLE_READS.with(|reads| reads.set(0));
    probe();
    GRAPH_PREAMBLE_READS.with(|reads| reads.get())
}

fn twin(result: Result<(), Why>) -> Option<String> {
    match result {
        Err(Why::Refused(Refusal::Twin { existing })) => Some(existing.as_str().into()),
        _ => None,
    }
}

/// Gap 1 of the claim-guard follow-ups (card PVTI_lAHOAAbLVc4BhPsyzhLLAl0):
/// every create path that can carry a `title::` claims that title, yet the
/// guard checks only the filename, so a create titled as an existing page
/// gives one page name two files. Fails on 8ef3a47ec on all three paths.
/// Ignored pending the product decision the receipt asks for
/// (RECEIPT-claim-guard.md): Tine accepts this silently today, and Logseq
/// neither refuses it nor keeps two files (an edited `title::` merges the
/// pages; a file that arrives that way is reported, not refused).
#[test]
#[ignore = "product decision pending: refuse, merge or report a create whose title:: names an existing page"]
fn a_creates_title_claims_no_name_another_file_holds() {
    let (_root, store) = open(&[("pages/Existing.md", "- e\n")]);
    store.whole_graph().unwrap();
    let new = file("pages/New.md");
    let mut accepted = Vec::new();

    // The transaction's CreateNew page save, the title in its pre-block.
    let page = PageDto {
        name: "New".into(),
        kind: PageKind::Page,
        title: "New".into(),
        pre_block: Some("title:: Existing".into()),
        blocks: vec![BlockDto {
            raw: "mine".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        guide: false,
    };
    let mut tx = store.page_plan(Some(crate::EditKind::ReplacePage));
    tx.save_page(
        &[crate::EditKind::ReplacePage],
        &PageId::from("pages/New.md"),
        SaveBase::CreateNew,
        &page,
    );
    if !matches!(tx.plan(), Err(Why::Refused(Refusal::Twin { .. }))) {
        accepted.push("CreateNew save");
    }

    // A raw create of page bytes (import, a template written whole).
    let mut plan = store.page_plan(Some(crate::EditKind::ReplacePage));
    plan.create(
        &new,
        crate::Content::Bytes(b"title:: Existing\n\n- mine\n".to_vec()),
    );
    if !matches!(plan.plan(), Err(Why::Refused(Refusal::Twin { .. }))) {
        accepted.push("raw create");
    }

    // The page host's typing save of a new page whose first block is
    // `title:: Existing`: the name production_guard proposes from its bytes.
    let proposed = store
        .graph
        .entry_for_path_in(
            &store.graph.root.join("pages/New.md"),
            Some("title:: Existing\n\n- mine\n"),
        )
        .unwrap();
    assert_eq!(proposed.name, "Existing");
    let result = store
        .page_plan(None)
        .creating_save_owner(&new, Some(&proposed.name), None);
    if twin(result).as_deref() != Some("pages/Existing.md") {
        accepted.push("typing save");
    }
    store.close();
    assert!(
        accepted.is_empty(),
        "a create titled Existing was admitted beside pages/Existing.md: {accepted:?}"
    );
}

/// Hold `count` pages of the graph with no owner publication yet, as the
/// launch install leaves them (`a_page_held_unindexed_through_the_launch_install_still_refuses_a_twin`).
fn hold_unindexed(store: &Store, count: usize) {
    for i in 0..count {
        store.graph.hold_unhosted(&format!("pages/held{i}.md"));
    }
    assert_eq!(store.graph.unindexed_held_keys().len(), count);
}

fn held_graph(count: usize) -> (tempfile::TempDir, Store) {
    let files: Vec<(String, String)> = (0..count)
        .map(|i| {
            (
                format!("pages/held{i}.md"),
                format!("title:: Held {i}\n\n- h\n"),
            )
        })
        .collect();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, text)| (rel.as_str(), text.as_str()))
        .collect();
    let (root, store) = open(&files);
    // The launch load finished: no background install moves the name
    // generation or the watcher's baseline under the test.
    store.whole_graph().unwrap();
    (root, store)
}

/// Gap 2 (REG-HELD-UNINDEXED-CLAIM-COST): the guard's held-unindexed part
/// reads each such page's preamble at most once per hold, not on every
/// create. 0, 1 and 50 held unindexed pages whose files the watcher has
/// since seen change (their retired rows no longer name them): the first
/// check reads each of them once, every later check none. On 8ef3a47ec every check read all of them.
#[test]
fn a_creates_held_claim_check_reads_each_unindexed_page_once_per_hold() {
    for count in [0, 1, 50] {
        let (_root, store) = held_graph(count);
        hold_unindexed(&store, count);
        store
            .graph
            .note_held_changed(&store.graph.unindexed_held_keys());
        let entry = |rel: &str| {
            store
                .graph
                .page_entry_in(&store.graph.root.join(rel), Some(""))
                .unwrap()
        };
        let fresh = entry("pages/Fresh.md");
        let first = reads(|| assert_eq!(store.graph.unindexed_held_claimant(&fresh, None), None));
        assert_eq!(
            first, count,
            "{count} held: the first check names each held page once"
        );
        let other = entry("pages/Other.md");
        let later = reads(|| assert_eq!(store.graph.unindexed_held_claimant(&other, None), None));
        assert_eq!(later, 0, "{count} held: a later check reads no held page");
        // The guard as a create asks it, its name lookup warm: no read.
        store.graph.find_entry("Another", PageKind::Page);
        let create = reads(|| {
            store
                .page_plan(None)
                .page_create_checks(&file("pages/Another.md"), "Another")
                .unwrap()
        });
        assert_eq!(
            create, 0,
            "{count} held: a create's guard reads no held page"
        );
        if count > 0 {
            let last = count - 1;
            let claimed = entry(&format!("pages/Held {last}.md"));
            let found = reads(|| {
                assert_eq!(
                    store.graph.unindexed_held_claimant(&claimed, None),
                    Some(store.graph.root.join(format!("pages/held{last}.md"))),
                    "{count} held: the held title is still claimed"
                )
            });
            assert_eq!(found, 0, "{count} held: found by its name, no read");
        }
        store.close();
    }
}

/// A page held after the cache installed its row retires that row to the
/// hold (R1): the guard names it from that row, with no read at all.
#[test]
fn a_held_page_whose_row_was_installed_is_named_without_a_read() {
    let (_root, store) = held_graph(50);
    hold_unindexed(&store, 50);
    let claimed = store
        .graph
        .page_entry_in(&store.graph.root.join("pages/Held 7.md"), Some(""))
        .unwrap();
    let found = reads(|| {
        assert_eq!(
            store.graph.unindexed_held_claimant(&claimed, None),
            Some(store.graph.root.join("pages/held7.md"))
        )
    });
    assert_eq!(found, 0, "seeded from the retired rows");
    store.close();
}

/// The memo is only as old as the watcher's view, like every other claimant
/// index the guard reads: a held page retitled outside Tine is named again
/// once a reconcile forwards it to its owner, and the old title is free.
#[test]
fn a_held_page_retitled_outside_is_named_again_after_the_watcher_sees_it() {
    let (root, store) = held_graph(1);
    hold_unindexed(&store, 1);
    let entry = |rel: &str| {
        store
            .graph
            .page_entry_in(&store.graph.root.join(rel), Some(""))
            .unwrap()
    };
    let held = store.graph.root.join("pages/held0.md");
    assert_eq!(
        store
            .graph
            .unindexed_held_claimant(&entry("pages/Held 0.md"), None),
        Some(held.clone())
    );
    let path = root.path().join("pages/held0.md");
    fs::write(&path, "title:: Retitled\n\n- h\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(
        store
            .graph
            .unindexed_held_claimant(&entry("pages/Retitled.md"), None),
        Some(held),
        "the new title is claimed"
    );
    assert_eq!(
        store
            .graph
            .unindexed_held_claimant(&entry("pages/Held 0.md"), None),
        None,
        "the old title is free"
    );
    store.close();
}
