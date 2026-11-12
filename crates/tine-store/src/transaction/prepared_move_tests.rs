//! A moved page's reference rewrite carried from the caller's planning read
//! (`PagePlan::move_file` with `MoveRewrite::Prepared`): preflight reuses it only for the bytes
//! it was prepared from, and every guard runs against the file as it is at
//! write time. Threat scenarios: an external editor or sync delivery
//! changes the page between the rename's plan and its write (base-revision
//! guard), or a caller hands a proposal for bytes the file no longer holds.
use crate::{Area, EditKind, MoveRewrite, RenameMap, Store, Why};
use std::fs;

const PAGE: &str = "title:: Old\n\n- see [[Old]] here\n";

fn graph(text: &str) -> (tempfile::TempDir, std::sync::Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/Old.md"), text).unwrap();
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    (dir, store)
}

fn map() -> RenameMap {
    RenameMap(vec![("Old".into(), "New".into())], Vec::new())
}

/// The proposal the rename planner makes for `text`.
fn proposal(store: &Store, text: &str) -> tine_core::refs::RenameProposal {
    let keyed = std::collections::HashMap::from([(tine_core::refs::page_key("Old"), "New".into())]);
    tine_core::refs::rename_proposal(
        text,
        false,
        &keyed,
        &Default::default(),
        store.config().file_name_format,
    )
    .unwrap()
}

/// Move `pages/Old.md` to `pages/New.md`, carrying a proposal prepared from
/// `prepared_from` (or none), and return the published bytes.
fn moved(on_disk: &str, prepared_from: Option<&str>) -> String {
    let (dir, store) = graph(on_disk);
    let writer = crate::test_host_writer::HostWriter::start(&store);
    let from = store.file_id(Area::Pages, "Old.md").unwrap();
    let to = store.file_id(Area::Pages, "New.md").unwrap();
    let rev = store.read(&from, None).unwrap().1;
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    match prepared_from {
        Some(text) => {
            let renames = map();
            let prepared = MoveRewrite::Prepared {
                renames: &renames,
                old: text.to_owned(),
                proposal: proposal(&store, text),
                format: store.config().file_name_format,
            };
            tx.move_file(&from, rev, &to, prepared)
        }
        None => tx.move_file(&from, rev, &to, Some(&map())),
    };
    writer.apply(tx.plan().unwrap()).unwrap();
    assert!(!dir.path().join("pages/Old.md").exists());
    let out = fs::read_to_string(dir.path().join("pages/New.md")).unwrap();
    drop(writer);
    store.close();
    out
}

/// A carried proposal publishes exactly what the move computes itself: the
/// references and the own title rebound.
#[test]
fn a_carried_move_rewrite_publishes_the_bytes_the_move_computes() {
    let computed = moved(PAGE, None);
    assert_eq!(computed, "title:: New\n\n- see [[New]] here\n");
    assert_eq!(moved(PAGE, Some(PAGE)), computed);
}

/// A proposal for bytes the file no longer holds (same revision guard
/// passed) is not reused: preflight recomputes from the staged bytes.
#[test]
fn a_carried_move_rewrite_for_other_bytes_is_recomputed() {
    let stale = "title:: Old\n\n- an older [[Old]] text\n";
    assert_eq!(moved(PAGE, Some(stale)), moved(PAGE, None));
}

/// The file changed after the plan read it: the base-revision guard refuses
/// the carried move exactly as an uncarried one, and nothing is written.
#[test]
fn a_carried_move_rewrite_planned_before_an_external_edit_is_refused() {
    for carried in [true, false] {
        let (dir, store) = graph(PAGE);
        let from = store.file_id(Area::Pages, "Old.md").unwrap();
        let to = store.file_id(Area::Pages, "New.md").unwrap();
        let rev = store.read(&from, None).unwrap().1;
        let p = proposal(&store, PAGE);
        let edited = "title:: Old\n\n- see [[Old]] here, edited elsewhere\n";
        fs::write(dir.path().join("pages/Old.md"), edited).unwrap();
        let mut tx = store.page_plan(Some(EditKind::RenamePage));
        if carried {
            let renames = map();
            let prepared = MoveRewrite::Prepared {
                renames: &renames,
                old: PAGE.to_owned(),
                proposal: p,
                format: store.config().file_name_format,
            };
            tx.move_file(&from, rev, &to, prepared);
        } else {
            tx.move_file(&from, rev, &to, Some(&map()));
        }
        let refused = tx.plan().err();
        assert!(
            matches!(&refused, Some(Why::Conflict { file, .. }) if *file == from),
            "carried {carried}: {refused:?}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
            edited
        );
        assert!(!dir.path().join("pages/New.md").exists());
        store.close();
    }
}
