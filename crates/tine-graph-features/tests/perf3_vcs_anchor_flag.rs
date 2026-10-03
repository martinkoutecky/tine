//! GH #623 (Ellis build 4): `conflict_inventory` read every page file at launch
//! (14.6 s on 12,922 pages, Windows + Defender) to find VCS anchor lines. The
//! store already reads every file once to load the page cache and now records,
//! beside each page's disk revision, whether its bytes carried an anchor line
//! (`Store::vcs_anchor_state`). Unmarked cached pages are answered with no
//! read. This file proves the answer is the old read-based one (differential
//! over a corpus), stays so across external edits, and is the old cost class
//! no longer (a read-count test lives in `perf3_vcs_anchor_reads.rs`).
//!
//! Invariants: I-12 (one answerer: `has_vcs_anchor` is the single prefilter),
//! I-25 (unit cost).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::concord_queue::vcs_conflict_markers;
use tine_core::model::{sync_conflict_base, Format};
use tine_graph_features::conflicts::{conflict_inventory, ConflictQueue};
use tine_store::{FileId, Store};

fn scratch(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-perf3-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("logseq/config.edn"),
        "{:preferred-format :markdown}\n",
    )
    .unwrap();
    root
}

/// The marker corpus: (relative path, bytes). Every shape the dossier names
/// plus the near misses a prefilter could get wrong in either direction.
fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    let conflict_md = "- mine\n<<<<<<< HEAD\n- a\n=======\n- b\n>>>>>>> theirs\n- tail\n";
    vec![
        ("pages/Clean.md", b"- just text\n- more\n".to_vec()),
        ("pages/Marked LF.md", conflict_md.as_bytes().to_vec()),
        (
            "pages/Marked CRLF.md",
            conflict_md.replace('\n', "\r\n").into_bytes(),
        ),
        (
            "pages/Marked Org.org",
            b"* heading\n<<<<<<< HEAD\n** a\n=======\n** b\n>>>>>>> theirs\n".to_vec(),
        ),
        (
            "pages/ns___Nested.md",
            b"- parent\n  - child\n<<<<<<< x\n  - a\n>>>>>>> y\n".to_vec(),
        ),
        (
            "pages/Only theirs anchor.md",
            b"- a\n>>>>>>> branch\n- b\n".to_vec(),
        ),
        // Marker-like but not column 0 / not an anchor: never listed.
        (
            "pages/Indented.md",
            b"- a\n  <<<<<<< HEAD\n  >>>>>>> x\n".to_vec(),
        ),
        ("pages/Bullet.md", b"- <<<<<<< HEAD\n- >>>>>>> x\n".to_vec()),
        ("pages/No space.md", b"- a\n<<<<<<<\n>>>>>>>\n".to_vec()),
        ("pages/Divider only.md", b"- a\n=======\n- b\n".to_vec()),
        ("pages/Six.md", b"- a\n<<<<<< HEAD\n>>>>>> x\n".to_vec()),
        // Anchor inside a fence: the flag says "maybe", the scan says no.
        (
            "pages/Fenced.md",
            b"- a\n```\n<<<<<<< HEAD\n>>>>>>> x\n```\n".to_vec(),
        ),
        // Anchor on the very first byte of the file.
        (
            "pages/First line.md",
            b"<<<<<<< HEAD\n- a\n>>>>>>> x\n".to_vec(),
        ),
        // BOM before the anchor: not column 0 for the scan.
        (
            "pages/Bom.md",
            b"\xEF\xBB\xBF<<<<<<< HEAD\n- a\n>>>>>>> x\n".to_vec(),
        ),
        // Invalid UTF-8 beside an anchor: an error today, not a listing.
        // (Left out of the corpus; covered by `unreadable_*` in the store.)
        (
            "journals/2026_06_26.md",
            b"- today\n<<<<<<< HEAD\n- a\n>>>>>>> x\n".to_vec(),
        ),
        ("journals/2026_06_27.md", b"- clean day\n".to_vec()),
        // A sync copy is its own conflict kind; never a marker listing.
        (
            "pages/Marked LF.sync-conflict-20260929-101010-ABCDEFG.md",
            conflict_md.as_bytes().to_vec(),
        ),
    ]
}

fn write_corpus(root: &Path) {
    for (rel, bytes) in corpus() {
        fs::write(root.join(rel), bytes).unwrap();
    }
}

/// The old answer, computed from the files themselves exactly as the read
/// path did: every page or journal text file, sync copies excluded, with its
/// scan result for its own format.
fn oracle(root: &Path) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for dir in ["journals", "pages"] {
        for entry in fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            let rel = format!("{dir}/{}", path.file_name().unwrap().to_string_lossy());
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            if sync_conflict_base(&stem).is_some() {
                continue;
            }
            let text = String::from_utf8(fs::read(&path).unwrap()).unwrap();
            let markers = vcs_conflict_markers(&text, Format::from_path(&path));
            if !markers.is_empty() {
                out.insert(rel, markers.iter().map(|m| m.to_string()).collect());
            }
        }
    }
    out
}

fn answer(store: &Store) -> BTreeMap<String, Vec<String>> {
    conflict_inventory(store)
        .unwrap()
        .vcs_markers
        .into_iter()
        .map(|m| (m.path, m.markers))
        .collect()
}

#[test]
fn the_flag_answer_equals_the_read_answer_over_the_marker_corpus() {
    let root = scratch("diff");
    write_corpus(&root);
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let expected = oracle(&root);
    // The oracle is not vacuous: LF, CRLF, org, nested, theirs-only, first-byte,
    // BOM (its closing anchor), journal.
    assert_eq!(expected.len(), 8, "{expected:?}");
    assert_eq!(answer(&store), expected);
    // Indented / bullet / no-space / divider / six / fenced stay out.
    for clean in [
        "Indented",
        "Bullet",
        "No space",
        "Divider only",
        "Six",
        "Fenced",
    ] {
        assert!(
            !expected.contains_key(&format!("pages/{clean}.md")),
            "{clean}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_flag_follows_external_edits_added_removed_and_moved() {
    let root = scratch("follow");
    write_corpus(&root);
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let queue = ConflictQueue::default();
    let changes = store.subscribe();
    assert_eq!(queue.inventory(&store).unwrap().vcs_markers.len(), 8);

    let edit = |rel: &str, text: &str| {
        let path = root.join(rel);
        let temp = path.with_extension("ext-tmp");
        fs::write(&temp, text).unwrap();
        fs::rename(temp, path).unwrap();
    };
    let settle = |store: &Store| {
        // Poll-free: an external edit is observed by an explicit rescan.
        store.scan_refresh().unwrap();
        while let Ok(Some(change)) = changes.try_recv() {
            queue.refresh_change(store, &change).unwrap();
        }
    };
    // A clean page gains a marker; a marked page is resolved; a fenced anchor
    // (flag true, scan false) loses its fence and becomes a real conflict.
    edit("pages/Clean.md", "- a\n<<<<<<< HEAD\n- b\n>>>>>>> x\n");
    edit("pages/Marked LF.md", "- resolved\n");
    edit("pages/Fenced.md", "- a\n<<<<<<< HEAD\n>>>>>>> x\n");
    settle(&store);
    let expected = oracle(&root);
    assert!(expected.contains_key("pages/Clean.md"));
    assert!(!expected.contains_key("pages/Marked LF.md"));
    assert!(expected.contains_key("pages/Fenced.md"));
    assert_eq!(
        queue.inventory(&store).unwrap().vcs_markers.len(),
        expected.len()
    );
    assert_eq!(answer(&store), expected);
    assert_eq!(
        store.vcs_anchor_state(&FileId::from("pages/Clean.md".to_owned())),
        Some(true)
    );
    assert_eq!(
        store.vcs_anchor_state(&FileId::from("pages/Marked LF.md".to_owned())),
        Some(false),
        "a resolved page drops its flag"
    );

    // A near miss (six angle brackets) is typed into a real anchor, and a
    // marked org page is edited in place while staying marked.
    edit("pages/Six.md", "- a\n<<<<<<< HEAD\n>>>>>>> x\n");
    edit(
        "pages/Marked Org.org",
        "* heading\n<<<<<<< HEAD\n** c\n>>>>>>> theirs\n",
    );
    settle(&store);
    let expected = oracle(&root);
    assert!(expected.contains_key("pages/Six.md"));
    assert_eq!(answer(&store), expected);
    assert_eq!(answer(&store), {
        // A cold store over the same bytes agrees with the incremental one.
        let cold = Store::open(&root, Default::default()).unwrap().0;
        cold.whole_graph().unwrap();
        answer(&cold)
    });
    let _ = fs::remove_dir_all(&root);
}
