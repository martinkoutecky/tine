//! `Store::move_occupant` probe counts (review c1909f09f, PC-3): the rename
//! planner asks it once per destination candidate, so a namespace rename pays
//! it per child. An absent destination and a final symlink cost one `lstat`
//! and no canonicalization; a regular occupant costs exactly two (graph root
//! and the destination), whatever the volume folds.
use super::*;
use crate::path_identity::CANONICALIZATIONS;

fn canonicalizations(probe: impl FnOnce()) -> usize {
    CANONICALIZATIONS.with(|count| count.set(0));
    probe();
    CANONICALIZATIONS.with(|count| count.get())
}

#[test]
fn a_destination_check_costs_two_canonicalizations_only_when_occupied() {
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
        0
    );
    assert_eq!(found, Some(None), "an absent destination has no occupant");

    assert_eq!(
        canonicalizations(|| found = Some(store.move_occupant(&file("pages/b.md")).unwrap())),
        2
    );
    assert_eq!(found, Some(Some(file("pages/b.md"))));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("b.md", root.path().join("pages/L.md")).unwrap();
        assert_eq!(
            canonicalizations(|| found = Some(store.move_occupant(&file("pages/L.md")).unwrap())),
            0
        );
        assert_eq!(
            found,
            Some(Some(file("pages/L.md"))),
            "a final symlink is its own entry, never its target"
        );
    }
    store.close();
}
