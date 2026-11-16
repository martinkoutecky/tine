//! C5's actual app census/adoption boundary. The page-host M4 tests separately
//! cold-recover valid durable records through ProductionIo after this rename.
use super::*;

#[test]
fn m4_c5_native_moved_graph_surfaces_every_old_namespace_vehicle() {
    for known_old_root in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        let data = dir.path().join("app");
        fs::create_dir(&old).unwrap();
        let old = fs::canonicalize(old).unwrap();
        let id = crate::settings::host_graph_id(&old);
        let source = data.join("drafts-v2").join(&id);
        fs::create_dir_all(source.join("unreadable")).unwrap();
        fs::create_dir(source.join("trash-custody")).unwrap();
        // Census deliberately treats record bytes as opaque. Even a vehicle
        // unreadable to launch must be surfaced rather than silently dropped.
        for (name, contents) in [
            ("p-one.draft", "first input"),
            ("op-two.draft", "second and third input"),
            ("unreadable/bad.draft", "unreadable admitted bytes"),
            ("trash-custody/marker", "pending trash custody"),
        ] {
            fs::write(source.join(name), contents).unwrap();
        }
        let known = if known_old_root {
            vec![old.to_string_lossy().into_owned()]
        } else {
            vec![]
        };
        assert!(find_orphans(&data, &old, &known).is_empty());
        // A process crash is represented by retaining only physical files;
        // no host/cached census or root identity carries across this rename.
        fs::rename(&old, &new).unwrap();
        let new = fs::canonicalize(new).unwrap();
        let found = find_orphans(&data, &new, &known);
        assert_eq!(
            found,
            vec![Orphan {
                id: id.clone(),
                old_root: known.first().cloned()
            }]
        );
        // Relaunch alone keeps every old vehicle and reports the same orphan
        // again; it cannot automatically mix it with another graph's edits.
        assert_eq!(find_orphans(&data, &new, &known), found);
        let target = data
            .join("drafts-v2")
            .join(crate::settings::host_graph_id(&new));
        fs::create_dir_all(target.join("unreadable")).unwrap();
        fs::write(target.join("p-current.draft"), "current input").unwrap();
        assert_eq!(
            adopt_namespace(&data, &id, &new, || {}).unwrap_err(),
            "draft-recovery-current-unsaved"
        );
        assert_eq!(
            fs::read_to_string(source.join("p-one.draft")).unwrap(),
            "first input"
        );
        fs::remove_file(target.join("p-current.draft")).unwrap();
        adopt_namespace(&data, &id, &new, || {}).unwrap();
        assert!(!source.exists());
        for (name, contents) in [
            ("p-one.draft", "first input"),
            ("op-two.draft", "second and third input"),
            ("unreadable/bad.draft", "unreadable admitted bytes"),
            ("trash-custody/marker", "pending trash custody"),
        ] {
            assert_eq!(fs::read_to_string(target.join(name)).unwrap(), contents);
        }
        assert!(find_orphans(&data, &new, &known).is_empty());
        assert!(!old.exists());
    }
}
