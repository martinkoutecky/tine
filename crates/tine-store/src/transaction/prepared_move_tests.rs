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
    RenameMap(vec![("Old".into(), "New".into())])
}

/// The proposal the rename planner makes for `text`.
fn proposal(store: &Store, text: &str) -> tine_core::refs::RenameProposal {
    let keyed = std::collections::HashMap::from([(tine_core::refs::page_key("Old"), "New".into())]);
    tine_core::refs::rename_proposal(text, false, &keyed, store.config().file_name_format).unwrap()
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
                renames: std::sync::Arc::new(renames),
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
                renames: std::sync::Arc::new(renames),
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

/// PC-9 (PRINCIPLES 2): a rename step's certificate and the model's reading
/// of its old text are read by that step's preflight and check only, so the
/// plan keeps neither once its steps are prepared; a graph-wide rename does
/// not hold every referrer's (about 28 bytes per input byte) to the end.
/// Both the referrer rewrite (`PreparedRewrite::reuse`) and the moved page
/// (`reuse_proposal`) are covered; the bytes are the same as without them.
#[test]
fn a_prepared_step_keeps_neither_its_certificate_nor_its_reading() {
    let referrer = "- see [[Old]] and #Old\n";
    let (dir, store) = graph(PAGE);
    fs::write(dir.path().join("pages/Ref.md"), referrer).unwrap();
    let names = store.config().file_name_format;
    let plan = std::sync::Arc::new(crate::CheckPlan {
        old: Some("Old".into()),
        destination: "New".into(),
        days: store.config().config.journal_format(),
        names,
    });
    let reading = |text: &str| {
        std::sync::Arc::new(tine_core::cm_check::observe(
            text,
            tine_core::model::Format::Md,
            names,
        ))
    };
    let check = |page: &str, observed| crate::RenameCheck {
        plan: std::sync::Arc::clone(&plan),
        page: page.into(),
        fixed: false,
        observed: Some(observed),
        source: None,
    };
    let (from, to) = (
        store.file_id(Area::Pages, "Old.md").unwrap(),
        store.file_id(Area::Pages, "New.md").unwrap(),
    );
    let ref_id = crate::PageId::from("pages/Ref.md");
    let renames = std::sync::Arc::new(map());
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    let p = proposal(&store, referrer);
    let ref_reading = reading(referrer);
    let kept = [std::sync::Arc::downgrade(&ref_reading)];
    tx.rewrite_refs_prepared(
        &[EditKind::RenamePage],
        &ref_id,
        store.read(&ref_id.file(), None).unwrap().1,
        (referrer.to_owned(), p.text),
        Some(&renames),
        names,
        p.trace,
    );
    tx.check(&ref_id.file(), check("ref", ref_reading));
    let moved_reading = reading(PAGE);
    let kept = [kept[0].clone(), std::sync::Arc::downgrade(&moved_reading)];
    tx.move_file(
        &from,
        store.read(&from, None).unwrap().1,
        &to,
        MoveRewrite::Prepared {
            renames: std::sync::Arc::clone(&renames),
            old: PAGE.to_owned(),
            proposal: proposal(&store, PAGE),
            format: names,
        },
    );
    tx.check(&from, check("old", moved_reading));
    let _writer = store.writer.lock().unwrap();
    let prepared = tx.prepare_steps().map_err(|(_, why)| why).unwrap();
    assert_eq!(
        prepared[0].new.as_deref(),
        Some("- see [[New]] and #New\n".as_bytes())
    );
    assert_eq!(
        prepared[1].new.as_deref(),
        Some("title:: New\n\n- see [[New]] here\n".as_bytes())
    );
    for (step, reading) in tx.steps.iter().zip(&kept) {
        assert!(
            matches!(
                step,
                super::Step::Rewrite {
                    prepared: None,
                    check: None,
                    ..
                } | super::Step::Move {
                    prepared: None,
                    check: None,
                    ..
                }
            ),
            "PC-9: a prepared step still holds its certificate or check"
        );
        assert!(
            reading.upgrade().is_none(),
            "PC-9: the model's reading outlived its step's check"
        );
    }
    drop(_writer);
    store.close();
}
