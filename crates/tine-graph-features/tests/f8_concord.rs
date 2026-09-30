//! og family 8 (Concord): the derived conflict queue, the in-page marker
//! diff and resolve, and the sync-copy resolve's merged-decision guard.
//! Ported from master `model_tests.rs` (Concord P4/L5 tests) to og's
//! Store-based feature clients.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::concord_queue::{vcs_conflict_markers, ConflictSource, SideRole};
use tine_core::model::Format;
use tine_core::pdf::{Highlight, Position, Rect};
use tine_core::sync_diff::{DiffRow, MergedSource, RowKind};
use tine_graph_features::{conflicts, pdf};
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

/// Markers exactly as `git merge` writes them in `diff3` style.
const DIFF3: &str = concat!(
    "- shared top\n",
    "<<<<<<< HEAD\n- mine wins\n",
    "||||||| merged common ancestors\n- original\n",
    "=======\n- theirs wins\n",
    ">>>>>>> feature\n",
);
const COPY: &str = "Notes.sync-conflict-20260817-101010-ABCDEFG.md";

fn scratch(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-f8c-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

fn open(root: &Path) -> Store {
    Store::open(root, Default::default()).unwrap().0
}

fn decidable(rows: &[DiffRow]) -> Vec<String> {
    let mut out = Vec::new();
    for row in rows {
        if row.kind != RowKind::Unchanged {
            out.push(row.id.clone());
        }
        out.extend(decidable(&row.children));
    }
    out
}

fn all(rows: &[DiffRow], choice: &str) -> HashMap<String, String> {
    decidable(rows)
        .into_iter()
        .map(|id| (id, choice.to_string()))
        .collect()
}

fn roots(text: &str) -> Vec<String> {
    tine_core::doc::parse(text)
        .roots
        .iter()
        .map(|b| b.raw().trim().to_string())
        .collect()
}

fn marker_copies(root: &Path, name: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root.join("logseq/.tine-trash/conflicts")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with(&format!("__markers__{name}"))
        })
        .map(|e| fs::read_to_string(e.path()).unwrap())
        .collect()
}

#[test]
fn conflict_queue_derives_both_artifact_sources_and_survives_a_restart() {
    let root = scratch("queue");
    fs::write(root.join("pages/Notes.md"), "- winner text\n").unwrap();
    fs::write(root.join("pages").join(COPY), "- copy text\n").unwrap();
    fs::write(root.join("pages/Merged.md"), DIFF3).unwrap();
    fs::write(root.join("pages/Calm.md"), "- nothing wrong here\n").unwrap();
    fs::write(
        root.join("pages/Docs about git.md"),
        "```\n<<<<<<< HEAD\n=======\n>>>>>>> feature\n```\n",
    )
    .unwrap();
    // A stray copy whose winner is gone stays out of the queue.
    fs::write(
        root.join("pages/Gone.sync-conflict-20260817-101010-ABCDEFG.md"),
        "- stray\n",
    )
    .unwrap();
    // A sync copy that itself carries markers belongs to the copy listing.
    fs::write(
        root.join("pages/Merged.sync-conflict-20260817-101010-ABCDEFG.md"),
        DIFF3,
    )
    .unwrap();

    let store = open(&root);
    let inventory = conflicts::conflict_inventory(&store);
    // The listings and the queue are one answer from one walk.
    assert_eq!(
        inventory
            .vcs_markers
            .iter()
            .map(|m| m.path.as_str())
            .collect::<Vec<_>>(),
        vec!["pages/Merged.md"]
    );
    for copy in inventory
        .queue
        .iter()
        .filter(|c| c.source == ConflictSource::SyncCopy)
    {
        let path = copy.sides[1].path.as_deref();
        assert!(inventory
            .sync_conflicts
            .iter()
            .any(|s| Some(s.path.as_str()) == path));
    }
    let queue = inventory.queue;
    assert_eq!(
        queue.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec![
            "copy:pages/Merged.sync-conflict-20260817-101010-ABCDEFG.md",
            "markers:pages/Merged.md",
            "copy:pages/Notes.sync-conflict-20260817-101010-ABCDEFG.md",
        ],
        "one object per artifact, ordered by page name: {queue:?}"
    );
    let markers = &queue[1];
    assert_eq!(markers.source, ConflictSource::VcsMarkers);
    assert_eq!(markers.page_path, "pages/Merged.md");
    assert_eq!(
        markers.markers,
        vec!["<<<<<<<", "|||||||", "=======", ">>>>>>>"]
    );
    assert_eq!(
        markers.sides.iter().map(|s| s.role).collect::<Vec<_>>(),
        vec![SideRole::Mine, SideRole::Theirs, SideRole::Base]
    );
    assert_eq!(markers.sides[0].label, "HEAD");
    assert_eq!(markers.sides[1].label, "feature");
    assert!(markers.block_conflicts.is_some_and(|n| n > 0));

    let copy = &queue[2];
    assert_eq!(copy.source, ConflictSource::SyncCopy);
    assert_eq!(copy.page_name, "Notes");
    assert_eq!(copy.page_path, "pages/Notes.md");
    assert_eq!(
        copy.sides
            .iter()
            .filter_map(|s| s.path.clone())
            .collect::<Vec<_>>(),
        vec!["pages/Notes.md".to_string(), format!("pages/{COPY}")]
    );
    assert!(copy.block_conflicts.is_some_and(|n| n > 0));

    // Derived: a second Store over the same disk state (a restart) reproduces
    // it, and nothing was written into the graph to make that work.
    store.close();
    let again = conflicts::conflict_inventory(&open(&root)).queue;
    assert_eq!(
        again
            .iter()
            .map(|c| (c.id.clone(), c.block_conflicts))
            .collect::<Vec<_>>(),
        queue
            .iter()
            .map(|c| (c.id.clone(), c.block_conflicts))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Merged.md")).unwrap(),
        DIFF3
    );
    assert!(!root.join("logseq/.tine-trash").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn marker_conflict_diff_reads_the_pages_own_sides_without_writing() {
    let root = scratch("marker-diff");
    fs::write(root.join("pages/Merged.md"), DIFF3).unwrap();
    fs::write(root.join("pages/Calm.md"), "- fine\n").unwrap();
    let store = open(&root);
    let parsed = conflicts::vcs_marker_conflict_diff(&store, "pages/Merged.md")
        .unwrap()
        .expect("a conflicted page");
    assert_eq!(parsed.mine_label, "HEAD");
    assert_eq!(parsed.theirs_label, "feature");
    assert_eq!(parsed.regions, 1);
    assert!(
        parsed.diff.three_way,
        "the ||||||| section is a real ancestor"
    );
    // Both staleness tokens address the ONE file the resolution will write.
    let rev: String = store
        .read(&PageId::from("pages/Merged.md").file(), None)
        .unwrap()
        .1
        .into();
    assert_eq!(parsed.diff.base_rev, rev);
    assert_eq!(parsed.diff.conflict_rev, rev);
    assert!(conflicts::vcs_marker_conflict_diff(&store, "pages/Calm.md")
        .unwrap()
        .is_none());
    assert_eq!(
        fs::read_to_string(root.join("pages/Merged.md")).unwrap(),
        DIFF3
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolving_markers_keep_both_writes_sibling_blocks_and_clears_the_quarantine() {
    let root = scratch("keep-both");
    let file = root.join("pages/Merged.md");
    fs::write(&file, DIFF3).unwrap();
    let store = open(&root);
    let id = PageId::from("pages/Merged.md");
    let read = store.page(&id).unwrap();
    assert!(
        matches!(
            store.save(
                EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &read.doc
            ),
            SaveOutcome::ReadOnly(_)
        ),
        "a marker-bearing page must refuse ordinary saves"
    );
    let diff = conflicts::vcs_marker_conflict_diff(&store, "pages/Merged.md")
        .unwrap()
        .unwrap()
        .diff;
    conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &all(&diff.rows, "both"),
        &diff.base_rev,
        "union",
    )
    .expect("resolution writes the merged result");
    let after = fs::read_to_string(&file).unwrap();
    assert!(
        vcs_conflict_markers(&after, Format::Md).is_empty(),
        "{after:?}"
    );
    assert_eq!(
        roots(&after),
        vec!["shared top", "mine wins", "theirs wins"]
    );
    let inventory = conflicts::conflict_inventory(&store);
    assert!(inventory.queue.is_empty() && inventory.vcs_markers.is_empty());
    assert_eq!(marker_copies(&root, "Merged.md"), vec![DIFF3.to_string()]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolving_markers_stages_the_preresolution_file_in_recoverable_trash() {
    let root = scratch("stage");
    let file = root.join("pages/Merged.md");
    fs::write(&file, DIFF3).unwrap();
    let store = open(&root);
    let diff = conflicts::vcs_marker_conflict_diff(&store, "pages/Merged.md")
        .unwrap()
        .unwrap()
        .diff;
    // Keep-mine everywhere — the LOSSY choice: "theirs wins" survives only in
    // the staged recovery copy.
    conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &all(&diff.rows, "mine"),
        &diff.base_rev,
        "union",
    )
    .unwrap();
    assert!(!fs::read_to_string(&file).unwrap().contains("theirs wins"));
    assert_eq!(marker_copies(&root, "Merged.md"), vec![DIFF3.to_string()]);
    // A stale resolve refuses BEFORE staging anything.
    let root2 = scratch("stage-stale");
    fs::write(root2.join("pages/Merged.md"), DIFF3).unwrap();
    let store2 = open(&root2);
    let err = conflicts::resolve_vcs_marker_conflict(
        &store2,
        "pages/Merged.md",
        &HashMap::new(),
        "not-the-current-rev",
        "union",
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(!root2.join("logseq/.tine-trash").exists());
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&root2);
}

#[test]
fn resolving_markers_can_apply_a_confirmed_merged_body() {
    const DISJOINT: &str = concat!(
        "- shared top\n",
        "<<<<<<< HEAD\n- the shared desktop machine label\n",
        "||||||| merged common ancestors\n- the shared desktop machine label 5\n",
        "=======\n- the shared desktop machine label 5 kk\n",
        ">>>>>>> feature\n",
    );
    let root = scratch("merged");
    let file = root.join("pages/Merged.md");
    fs::write(&file, DISJOINT).unwrap();
    let store = open(&root);
    let diff = conflicts::vcs_marker_conflict_diff(&store, "pages/Merged.md")
        .unwrap()
        .unwrap()
        .diff;
    let row = diff
        .rows
        .iter()
        .find(|r| r.merged.is_some())
        .expect("a merged proposal");
    assert_eq!(row.suggestion.as_deref(), Some("merged"));
    assert_eq!(
        row.merged.as_ref().unwrap().text,
        "the shared desktop machine label kk"
    );
    let decisions = HashMap::from([(row.id.clone(), "merged".to_string())]);
    conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &decisions,
        &diff.base_rev,
        "union",
    )
    .unwrap();
    let after = fs::read_to_string(&file).unwrap();
    assert_eq!(
        roots(&after),
        vec!["shared top", "the shared desktop machine label kk"]
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolving_fossil_markers_can_apply_the_suggested_resolution() {
    const FOSSIL: &str = concat!(
        "- shared top\n",
        "<<<<<<< BEGIN MERGE CONFLICT: local copy shown first <<<<<<<<<<<<<<<\n",
        "- the quick brown fox jumped over it\n",
        "####### SUGGESTED CONFLICT RESOLUTION follows ##################\n",
        "- the quick brown fox leapt over it\n",
        "||||||| COMMON ANCESTOR content follows |||||||||||||||||||||||||\n",
        "- the quick brown fox jumps over it\n",
        "======= MERGED IN content follows ==============================\n",
        "- the quick brown fox leaped over it\n",
        ">>>>>>> END MERGE CONFLICT >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>\n",
    );
    let root = scratch("fossil");
    let file = root.join("pages/Merged.md");
    fs::write(&file, FOSSIL).unwrap();
    let store = open(&root);
    let diff = conflicts::vcs_marker_conflict_diff(&store, "pages/Merged.md")
        .unwrap()
        .unwrap()
        .diff;
    let row = diff
        .rows
        .iter()
        .find(|r| r.merged.is_some())
        .expect("an artifact proposal");
    let proposal = row.merged.as_ref().unwrap();
    assert_eq!(proposal.source, MergedSource::Artifact);
    assert_eq!(proposal.text, "the quick brown fox leapt over it");
    let decisions = HashMap::from([(row.id.clone(), "merged".to_string())]);
    // The stale-rev guard fires against the ORIGINAL rev first.
    let err = conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &decisions,
        "not-the-current-rev",
        "union",
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&file).unwrap(), FOSSIL);
    conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &decisions,
        &diff.base_rev,
        "union",
    )
    .unwrap();
    let after = fs::read_to_string(&file).unwrap();
    assert_eq!(
        roots(&after),
        vec!["shared top", "the quick brown fox leapt over it"]
    );
    assert!(
        !after.contains("jumped") && !after.contains("leaped"),
        "{after:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_fossil_suggestion_equal_to_a_side_offers_nothing_and_writes_nothing() {
    const FOSSIL: &str = concat!(
        "- shared top\n",
        "<<<<<<< BEGIN MERGE CONFLICT: local copy shown first <<<<<<<<<<<<<<<\n",
        "- the quick brown fox jumped over it\n",
        "####### SUGGESTED CONFLICT RESOLUTION follows ##################\n",
        "- the quick brown fox leaped over it\n",
        "||||||| COMMON ANCESTOR content follows |||||||||||||||||||||||||\n",
        "- the quick brown fox jumps over it\n",
        "======= MERGED IN content follows ==============================\n",
        "- the quick brown fox leaped over it\n",
        ">>>>>>> END MERGE CONFLICT >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>\n",
    );
    let root = scratch("fossil-equal");
    let file = root.join("pages/Merged.md");
    fs::write(&file, FOSSIL).unwrap();
    let store = open(&root);
    let diff = conflicts::vcs_marker_conflict_diff(&store, "pages/Merged.md")
        .unwrap()
        .unwrap()
        .diff;
    assert!(diff.rows.iter().all(|r| r.merged.is_none()));
    let err = conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &all(&diff.rows, "merged"),
        &diff.base_rev,
        "union",
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{err}");
    assert_eq!(fs::read_to_string(&file).unwrap(), FOSSIL);
    assert!(!root.join("logseq/.tine-trash").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn markers_without_an_ancestor_refuse_a_forged_merged_decision() {
    const NO_BASE: &str = concat!(
        "- shared top\n",
        "<<<<<<< HEAD\n- the shared desktop machine label\n",
        "=======\n- the shared desktop machine label 5 kk\n",
        ">>>>>>> feature\n",
    );
    let root = scratch("nobase");
    let file = root.join("pages/Merged.md");
    fs::write(&file, NO_BASE).unwrap();
    let store = open(&root);
    let diff = conflicts::vcs_marker_conflict_diff(&store, "pages/Merged.md")
        .unwrap()
        .unwrap()
        .diff;
    assert!(!diff.three_way);
    assert!(diff.rows.iter().all(|r| r.merged.is_none()));
    let err = conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &all(&diff.rows, "merged"),
        &diff.base_rev,
        "union",
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{err}");
    assert_eq!(fs::read_to_string(&file).unwrap(), NO_BASE);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn marker_resolution_is_guarded_and_never_leaves_the_file_writable() {
    let root = scratch("guards");
    let file = root.join("pages/Merged.md");
    fs::write(&file, DIFF3).unwrap();
    fs::write(root.join("pages/Calm.md"), "- fine\n").unwrap();
    fs::write(root.join("pages/Other.md"), DIFF3).unwrap();
    let store = open(&root);
    let diff = conflicts::vcs_marker_conflict_diff(&store, "pages/Merged.md")
        .unwrap()
        .unwrap()
        .diff;
    let decisions = HashMap::new();

    // Stale base_rev → refuse without writing (the VCS moved under the UI).
    let err = conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &decisions,
        "not-the-current-rev",
        "union",
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&file).unwrap(), DIFF3);

    // A page with no markers is not a resolution target.
    let calm_rev: String = store
        .read(&PageId::from("pages/Calm.md").file(), None)
        .unwrap()
        .1
        .into();
    assert_eq!(
        conflicts::resolve_vcs_marker_conflict(
            &store,
            "pages/Calm.md",
            &decisions,
            &calm_rev,
            "union"
        )
        .unwrap_err()
        .kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Calm.md")).unwrap(),
        "- fine\n"
    );

    // The exemption is scoped to the one resolution: afterwards ordinary saves
    // to a still-marker-bearing page are refused again.
    conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &decisions,
        &diff.base_rev,
        "union",
    )
    .expect("the real resolution succeeds");
    let other = PageId::from("pages/Other.md");
    let read = store.page(&other).unwrap();
    assert!(
        matches!(
            store.save(
                EditKind::ReplacePage,
                &other,
                SaveBase::Existing(read.rev),
                &read.doc
            ),
            SaveOutcome::ReadOnly(_)
        ),
        "the other marker page stays quarantined"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Other.md")).unwrap(),
        DIFF3
    );
    // And the resolved page is now an ordinary, savable page.
    let id = PageId::from("pages/Merged.md");
    let read = store.page(&id).unwrap();
    let mut edited = read.doc.clone();
    edited.blocks[0].raw = "edited after resolving".into();
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &edited
        ),
        SaveOutcome::Saved(_)
    ));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_sync_copy_resolve_refuses_a_forged_merged_decision() {
    let root = scratch("copy-forged");
    fs::write(root.join("pages/Notes.md"), "- the quick brown fox\n").unwrap();
    fs::write(root.join("pages").join(COPY), "- the quick brown cat\n").unwrap();
    let store = open(&root);
    let copy = format!("pages/{COPY}");
    let diff = conflicts::sync_conflict_diff(&store, "pages/Notes.md", &copy, &[])
        .unwrap()
        .unwrap();
    assert!(!diff.three_way, "a conflict copy carries no ancestor");
    let err = conflicts::resolve_sync_conflict(
        &store,
        "pages/Notes.md",
        &copy,
        &all(&diff.rows, "merged"),
        &diff.base_rev,
        &diff.conflict_rev,
        None,
        &[],
        "union",
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{err}");
    assert_eq!(
        fs::read_to_string(root.join("pages/Notes.md")).unwrap(),
        "- the quick brown fox\n"
    );
    assert!(
        root.join("pages").join(COPY).exists(),
        "the copy is not trashed"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_sync_copy_resolve_against_a_marker_winner_writes_nothing() {
    let root = scratch("copy-marker-winner");
    fs::write(root.join("pages/Merged.md"), DIFF3).unwrap();
    let copy = "pages/Merged.sync-conflict-20260817-101010-ABCDEFG.md";
    fs::write(root.join(copy), "- copy\n").unwrap();
    let store = open(&root);
    let diff = conflicts::sync_conflict_diff(&store, "pages/Merged.md", copy, &[])
        .unwrap()
        .unwrap();
    let err = conflicts::resolve_sync_conflict(
        &store,
        "pages/Merged.md",
        copy,
        &HashMap::new(),
        &diff.base_rev,
        &diff.conflict_rev,
        None,
        &[],
        "union",
    )
    .unwrap_err();
    assert!(err.to_string().contains("conflict markers"), "{err}");
    assert_eq!(
        fs::read_to_string(root.join("pages/Merged.md")).unwrap(),
        DIFF3
    );
    assert!(root.join(copy).exists());
    let _ = fs::remove_dir_all(&root);
}

fn highlight(id: &str, page: i64) -> Highlight {
    Highlight {
        id: id.into(),
        page,
        position: Position {
            page,
            bounding: Rect {
                top: 0.0,
                left: 0.0,
                width: 1.0,
                height: 1.0,
                source_width: None,
                source_height: None,
            },
            rects: vec![],
        },
        color: "yellow".into(),
        text: Some(id.into()),
        image: None,
    }
}

fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn write_highlights_refuses_a_marker_bearing_hls_page() {
    let root = scratch("hls-markers");
    let store = open(&root);
    let h = highlight("11111111-1111-1111-1111-111111111111", 1);
    pdf::write_highlights(&store, "paper.pdf", "Paper", &[h.clone()], &[]).unwrap();
    let page = root.join("pages/hls__paper.md");
    let conflicted = format!(
        "<<<<<<< HEAD\n{}=======\n- the other merge side\n>>>>>>> feature\n",
        fs::read_to_string(&page).unwrap()
    );
    fs::write(&page, &conflicted).unwrap();
    store.close();
    let store = open(&root);
    assert!(conflicts::conflict_inventory(&store)
        .queue
        .iter()
        .any(|c| c.id == "markers:pages/hls__paper.md"));
    let before = tree(&root);
    let h2 = highlight("22222222-2222-2222-2222-222222222222", 4);
    let err = pdf::write_highlights(&store, "paper.pdf", "Paper", &[h.clone(), h2], &[h])
        .expect_err("a highlight write to a conflicted page must refuse");
    assert!(err.to_string().contains("conflict markers"), "{err}");
    assert_eq!(tree(&root), before, "page AND sidecar stay byte-identical");
    let _ = fs::remove_dir_all(&root);
}

/// The receipt fixture: one git `diff3` marker file and one Syncthing copy,
/// driven through the literal backend path the in-page resolver calls
/// (inventory → diff → guarded resolve → re-derived inventory). Before and
/// after trees are printed for the lane receipt; the after-bytes are pinned.
#[test]
fn end_to_end_marker_file_and_syncthing_copy_fixtures() {
    let root = scratch("e2e");
    fs::write(root.join("pages/Merged.md"), DIFF3).unwrap();
    fs::write(root.join("pages/Notes.md"), "- shared\n- winner edit\n").unwrap();
    fs::write(root.join("pages").join(COPY), "- shared\n- copy edit\n").unwrap();
    let show = |label: &str, t: &[(String, Vec<u8>)]| {
        for (path, bytes) in t {
            println!("{label} {path}: {:?}", String::from_utf8_lossy(bytes));
        }
    };
    let before = tree(&root);
    show("BEFORE", &before);

    let store = open(&root);
    let inventory = conflicts::conflict_inventory(&store);
    assert_eq!(
        inventory
            .queue
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        vec!["markers:pages/Merged.md", &format!("copy:pages/{COPY}")]
    );

    let marker = conflicts::vcs_marker_conflict_diff(&store, "pages/Merged.md")
        .unwrap()
        .unwrap()
        .diff;
    conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Merged.md",
        &all(&marker.rows, "both"),
        &marker.base_rev,
        "union",
    )
    .unwrap();

    let copy = format!("pages/{COPY}");
    let sync = conflicts::sync_conflict_diff(&store, "pages/Notes.md", &copy, &[])
        .unwrap()
        .unwrap();
    conflicts::resolve_sync_conflict(
        &store,
        "pages/Notes.md",
        &copy,
        &all(&sync.rows, "both"),
        &sync.base_rev,
        &sync.conflict_rev,
        None,
        &[],
        "union",
    )
    .unwrap();

    let after = tree(&root);
    show("AFTER", &after);
    let file = |rel: &str| {
        after
            .iter()
            .find(|(p, _)| p == rel)
            .map(|(_, b)| String::from_utf8(b.clone()).unwrap())
    };
    assert_eq!(
        file("pages/Merged.md").as_deref(),
        Some("- shared top\n- mine wins\n- theirs wins\n")
    );
    assert_eq!(
        file("pages/Notes.md").as_deref(),
        Some("- shared\n- winner edit\n- copy edit\n")
    );
    assert_eq!(file(&copy), None, "the merged copy left pages/");
    // Nothing is lost: both pre-resolution inputs sit in the recoverable trash.
    let trashed: Vec<String> = after
        .iter()
        .filter(|(p, _)| p.starts_with("logseq/.tine-trash/"))
        .map(|(_, b)| String::from_utf8(b.clone()).unwrap())
        .collect();
    assert!(trashed.contains(&DIFF3.to_string()), "{trashed:?}");
    assert!(
        trashed.contains(&"- shared\n- copy edit\n".to_string()),
        "{trashed:?}"
    );
    let inventory = conflicts::conflict_inventory(&store);
    assert!(inventory.queue.is_empty() && inventory.sync_conflicts.is_empty());
    let _ = fs::remove_dir_all(&root);
}

/// Master 3c18d0e (family 8): re-saving highlights must not churn the
/// annotation files. An identical highlight set rewrites neither the sidecar
/// nor the `hls__` page (bytes and mtimes unchanged, so Syncthing sees no
/// edit to conflict on), and a recolour keeps a hand-written child note under
/// the highlight byte-for-byte, with its indentation.
#[test]
fn resaving_highlights_leaves_unchanged_files_and_hand_notes_alone() {
    let root = scratch("hls-churn");
    let store = open(&root);
    let h = highlight("11111111-1111-1111-1111-111111111111", 1);
    pdf::write_highlights(&store, "paper.pdf", "Paper", &[h.clone()], &[]).unwrap();
    let page = root.join("pages/hls__paper.md");
    let sidecar = root.join("assets/paper.edn");
    let stamp = |path: &Path| {
        (
            fs::read(path).unwrap(),
            fs::metadata(path).unwrap().modified().unwrap(),
        )
    };
    let (page_before, sidecar_before) = (stamp(&page), stamp(&sidecar));
    std::thread::sleep(std::time::Duration::from_millis(30));
    pdf::write_highlights(&store, "paper.pdf", "Paper", &[h.clone()], &[h.clone()]).unwrap();
    assert_eq!(
        stamp(&page),
        page_before,
        "identical set: hls page untouched"
    );
    assert_eq!(
        stamp(&sidecar),
        sidecar_before,
        "identical set: sidecar untouched"
    );

    let written = String::from_utf8(page_before.0).unwrap();
    let with_note = format!(
        "{}\n  - my own note\n    second line of it\n",
        written.trim_end()
    );
    fs::write(&page, &with_note).unwrap();
    store.scan_refresh().unwrap();
    let mut recoloured = h.clone();
    recoloured.color = "green".into();
    pdf::write_highlights(&store, "paper.pdf", "Paper", &[recoloured], &[h]).unwrap();
    let after = fs::read_to_string(&page).unwrap();
    assert!(
        after.contains("\n  - my own note\n    second line of it\n"),
        "the hand-written child keeps its bytes and indentation:\n{after}"
    );
    let _ = fs::remove_dir_all(&root);
}

/// Master 3c18d0e `write_highlights_leaves_an_unchanged_hls_page_byte_identical`:
/// an `hls__` page restyled in the other house style (two-space indent, no
/// trailing newline) with a user note is not rewritten when a reopened graph
/// re-saves the same highlight set.
#[test]
fn resaving_highlights_leaves_a_restyled_hls_page_byte_identical() {
    let root = scratch("hls-restyled");
    let store = open(&root);
    let h = highlight("11111111-1111-1111-1111-111111111111", 1);
    pdf::write_highlights(&store, "paper.pdf", "Paper", &[h.clone()], &[]).unwrap();
    let page = root.join("pages/hls__paper.md");
    let generated = fs::read_to_string(&page).unwrap();
    let restyled = format!("{}\n  - my own note\n", generated.trim_end())
        .replace('\t', "  ")
        .trim_end()
        .to_string();
    fs::write(&page, &restyled).unwrap();
    drop(store);
    let reopened = open(&root);
    pdf::write_highlights(&reopened, "paper.pdf", "Paper", &[h], &[]).unwrap();
    assert_eq!(
        fs::read_to_string(&page).unwrap(),
        restyled,
        "re-saving the same highlights must not rewrite the page"
    );
    let _ = fs::remove_dir_all(&root);
}
