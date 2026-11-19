//! `Store::move_occupant` probe counts (review c1909f09f, PC-3): the rename
//! planner asks it once per destination candidate, so a namespace rename pays
//! it per child. Every candidate first pays the move guard's own destination
//! check (`CHECK`, independent of graph size); then an absent destination and
//! a final symlink cost one `lstat` and no canonicalization, and a regular
//! occupant exactly two more (graph root and the destination), whatever the
//! volume folds.
use super::*;
use crate::path_identity::CANONICALIZATIONS;

fn canonicalizations(probe: impl FnOnce()) -> usize {
    CANONICALIZATIONS.with(|count| count.set(0));
    probe();
    CANONICALIZATIONS.with(|count| count.get())
}

#[test]
fn a_destination_check_costs_its_guard_check_plus_two_when_occupied() {
    // The destination check the move guard makes first (review da945bc19,
    // DS-8): a fixed number per candidate, whatever the graph's size.
    const CHECK: usize = 5;
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    fs::write(root.path().join("pages/b.md"), "title:: C\n\n- c\n").unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let file = |rel: &str| FileId::from(rel.to_owned());

    let mut found = None;
    assert_eq!(
        canonicalizations(|| found = Some(store.move_occupant(&file("pages/z.md")).unwrap())),
        CHECK
    );
    assert_eq!(found, Some(None), "an absent destination has no occupant");

    assert_eq!(
        canonicalizations(|| found = Some(store.move_occupant(&file("pages/b.md")).unwrap())),
        CHECK + 2
    );
    assert_eq!(found, Some(Some(file("pages/b.md"))));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("b.md", root.path().join("pages/L.md")).unwrap();
        assert_eq!(
            canonicalizations(|| found = Some(store.move_occupant(&file("pages/L.md")).unwrap())),
            CHECK
        );
        assert_eq!(
            found,
            Some(Some(file("pages/L.md"))),
            "a final symlink is its own entry, never its target"
        );
    }
    store.close();
}

/// Review da945bc19 (DS-8): the destination is checked the way the move
/// guard checks it before any probe, so the planner is never told that an
/// entry outside the graph occupies a destination. In-scope threats:
/// malformed input (a rename target built from an unsafe id) and an external
/// editor or sync delivery replacing a graph directory with a link out of the
/// graph. A graph with an `x.md` page outside it, and the probe's answer for
/// `to`.
fn outside_occupant(to: impl FnOnce(&Path, &Path) -> String) -> Result<Option<FileId>, StoreError> {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    fs::write(outside.path().join("x.md"), "title:: X\n\n- x\n").unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let to = to(root.path(), outside.path());
    assert!(
        fs::symlink_metadata(root.path().join(&to)).is_ok(),
        "{to} names a real file outside the graph"
    );
    let answer = store.move_occupant(&FileId::from(to));
    store.close();
    answer
}

#[test]
fn an_absolute_destination_is_refused_before_it_is_probed() {
    let answer = outside_occupant(|_, outside| outside.join("x.md").to_str().unwrap().to_owned());
    assert!(
        matches!(answer, Err(StoreError::InvalidTarget(_))),
        "{answer:?}"
    );
}

#[test]
fn a_parent_relative_destination_is_refused_before_it_is_probed() {
    // The two temp dirs are siblings, so this is relative on every platform
    // (review a55c0f03a: a Windows drive prefix must not end up inside it).
    let answer = outside_occupant(|root, outside| {
        assert_eq!(root.parent(), outside.parent());
        let sibling = outside.file_name().unwrap().to_str().unwrap();
        format!("pages/../../{sibling}/x.md")
    });
    assert!(
        matches!(answer, Err(StoreError::InvalidTarget(_))),
        "{answer:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_destination_under_a_link_out_of_the_graph_is_refused_before_it_is_probed() {
    let answer = outside_occupant(|root, outside| {
        std::os::unix::fs::symlink(outside, root.join("pages/ext")).unwrap();
        "pages/ext/x.md".to_owned()
    });
    assert!(answer.is_err(), "{answer:?}");
}
