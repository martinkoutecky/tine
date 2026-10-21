//! An off page host refuses every page writer (host-only-writer DESIGN
//! §3.6). The slot is `PageHostSlot::Off` before a binding's host starts and
//! after a restore whose fresh host could not start (in-scope scenario: a
//! disk error at the host's start). Each command passes the statement
//! temporary `slot.host_slot()?.running()?` to its writer (the A-K3 shape
//! `host_slot_guard_tests` pins), so with the slot off the command answers
//! `NO_PAGE_HOST` and writes nothing. A Tauri command cannot run without a
//! webview, so each test runs that statement over a real slot, one writer
//! family each: merge and rename-file, journal trash and migration, PDF
//! highlights, the Concord resolves, and the Guide copy.

use crate::state::{GraphSlot, PageHostSlot, NO_PAGE_HOST};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use tine_graph_features::{conflicts, guide, journals, pages, pdf};
use tine_store::{OpenOptions, Store, WatchMode};

const COPY: &str = "pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md";
const MARKED: &str = "pages/Marked.md";
const KEEPER: &str = "journals/2026_06_26.md";
const STRAY: &str = "journals/Friday, 26-06-2026.md";

/// A bound slot over a graph with something for every writer to change,
/// its host never started (`PageHostSlot::Off`).
fn off_slot(label: &str) -> (tempfile::TempDir, PathBuf, GraphSlot) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(label);
    for sub in ["pages", "journals", "logseq", "assets"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    for (rel, text) in [
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        ),
        ("pages/A.md", "- a body\n"),
        ("pages/B.md", "- b body\n"),
        ("pages/stray name.md", "- stray\n"),
        ("pages/Foo.md", "- shared\n- mine\n"),
        (COPY, "- shared\n- theirs\n"),
        (
            MARKED,
            "- before\n<<<<<<< ours\n- mine\n=======\n- theirs\n>>>>>>> theirs\n",
        ),
        (KEEPER, "- shared line\n- only in canonical\n"),
        (STRAY, "- shared line\n- only in stray\n"),
        ("journals/Jun 19th, 2026.md", "- day\n"),
    ] {
        std::fs::write(root.join(rel), text).unwrap();
    }
    let store = Store::open(
        &root,
        OpenOptions {
            approved_external_assets: None,
            watch: WatchMode::Poll,
            launch_checkpoint: None,
        },
    )
    .unwrap()
    .0;
    store.whole_graph().unwrap();
    let slot = GraphSlot::new(store, root.clone());
    assert!(matches!(*slot.host_slot().unwrap(), PageHostSlot::Off));
    (dir, root, slot)
}

/// Every file under the graph, by path.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                out.insert(path.strip_prefix(root).unwrap().to_owned(), bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// `command` (a command's writer statement) refuses with `NO_PAGE_HOST` and
/// leaves every file of the graph as it was.
fn refuses<T: std::fmt::Debug>(label: &str, command: impl FnOnce(&GraphSlot) -> Result<T, String>) {
    let (_dir, root, slot) = off_slot(label);
    let before = tree(&root);
    let result = command(&slot);
    assert_eq!(
        result.unwrap_err(),
        NO_PAGE_HOST,
        "{label}: an off host must refuse"
    );
    assert_eq!(tree(&root), before, "{label}: the refusal wrote");
}

#[test]
fn an_off_host_refuses_the_page_merge_and_rescue() {
    refuses("merge", |slot| {
        pages::merge_pages(
            &slot.store,
            slot.host_slot()?.running()?,
            "pages/A.md",
            "pages/B.md",
        )
        .map_err(|error| error.to_string())
    });
    refuses("rescue", |slot| {
        pages::rename_file_to_page(
            &slot.store,
            slot.host_slot()?.running()?,
            "pages/stray name.md",
            "Rescued",
        )
        .map_err(|error| error.to_string())
    });
}

#[test]
fn an_off_host_refuses_the_journal_trash_and_migration() {
    refuses("trash-journal", |slot| {
        journals::trash_journal_file(
            &slot.store,
            slot.host_slot()?.running()?,
            "Jun 19th, 2026.md",
        )
        .map_err(|error| error.to_string())
    });
    refuses("migrate", |slot| {
        let listed = journals::journal_filename_migrations(&slot.store).unwrap();
        assert!(!listed.is_empty(), "{listed:?}");
        journals::migrate_journal_filenames(&slot.store, slot.host_slot()?.running()?, &listed)
            .map_err(|error| error.to_string())
    });
}

#[test]
fn an_off_host_refuses_a_highlight_save() {
    let rect = tine_core::pdf::Rect {
        top: 1.0,
        left: 1.0,
        width: 2.0,
        height: 2.0,
        source_width: None,
        source_height: None,
    };
    let highlight = tine_core::pdf::Highlight {
        id: "h1".into(),
        page: 1,
        position: tine_core::pdf::Position {
            page: 1,
            bounding: rect.clone(),
            rects: vec![rect],
        },
        color: "yellow".into(),
        text: Some("quoted".into()),
        image: None,
    };
    refuses("highlights", |slot| {
        pdf::write_highlights(
            &slot.store,
            slot.host_slot()?.running()?,
            "paper.pdf",
            "Paper",
            &[highlight],
            &[],
        )
        .map_err(|error| error.to_string())
    });
}

#[test]
fn an_off_host_refuses_the_concord_resolves() {
    refuses("sync-copy", |slot| {
        let diff = conflicts::sync_conflict_diff(&slot.store, "pages/Foo.md", COPY, &[])
            .unwrap()
            .expect("a sync copy");
        conflicts::resolve_sync_conflict(
            &slot.store,
            slot.host_slot()?.running()?,
            "pages/Foo.md",
            COPY,
            &HashMap::new(),
            &diff.base_rev,
            &diff.conflict_rev,
            None,
            &[],
            "union",
        )
        .map_err(|error| error.to_string())
    });
    refuses("duplicate-day", |slot| {
        let diff = conflicts::duplicate_journal_diff(&slot.store, KEEPER, STRAY)
            .unwrap()
            .expect("a duplicate day");
        conflicts::resolve_duplicate_journal_day(
            &slot.store,
            slot.host_slot()?.running()?,
            KEEPER,
            STRAY,
            &HashMap::new(),
            &diff.base_rev,
            &diff.conflict_rev,
            "union",
        )
        .map_err(|error| error.to_string())
    });
    refuses("markers", |slot| {
        let review = conflicts::vcs_marker_conflict_diff(&slot.store, MARKED)
            .unwrap()
            .expect("markers");
        conflicts::resolve_vcs_marker_conflict(
            &slot.store,
            slot.host_slot()?.running()?,
            MARKED,
            &HashMap::new(),
            &review.diff.base_rev,
            "union",
        )
        .map_err(|error| error.to_string())
    });
}

#[test]
fn an_off_host_refuses_the_guide_copy() {
    refuses("guide", |slot| {
        guide::copy_guide_into_graph(&slot.store, slot.host_slot()?.running()?, "Tine Guide")
            .map_err(|error| error.to_string())
    });
}
