//! Family 8a (Concord): the sync-tool conflict-copy name recognizer accepts
//! only the GENERATED provider shapes. Ported from master
//! `model_tests.rs::sync_conflict_base_matches_real_provider_formats_only`
//! (master `4280da0e6`): a real page named `Foo.sync-conflict-notes.md` must
//! stay a real page, and Seafile copies must be recognized.

use tine_core::model::sync_conflict_base;

#[test]
fn sync_conflict_base_matches_real_provider_formats_only() {
    for (stem, base) in [
        // Syncthing: `<stem>.sync-conflict-YYYYMMDD-HHMMSS-<short device id>`
        // (syncthing lib/model/folder_sendrecv.go `conflictName`; the device
        // id is up to 7 base32 chars [A-Z2-7], empty when the modifying
        // device is unknown; pre-1.1.0 versions omitted `-<device>`).
        ("Foo.sync-conflict-20260705-141233-A2B3C4D", Some("Foo")),
        ("Foo.sync-conflict-20260705-141233-", Some("Foo")),
        ("Foo.sync-conflict-20190201-124559", Some("Foo")),
        (
            "Foo.bar.sync-conflict-20260705-141233-ABCDEFG",
            Some("Foo.bar"),
        ),
        // Nested copy: the deepest tag wins, the base keeps the outer tag.
        (
            "Foo.sync-conflict-20260101-010101-AAAAAAA.sync-conflict-20260202-020202-BBBBBBB",
            Some("Foo.sync-conflict-20260101-010101-AAAAAAA"),
        ),
        // False positives the loose substring match used to deindex:
        ("Foo.sync-conflict-notes", None),
        ("Foo.sync-conflict-", None),
        ("Foo.sync-conflict-2026-08-01", None),
        ("Foo.sync-conflict-20260705", None),
        ("Foo.sync-conflict-20260705-141233x", None),
        ("Foo.sync-conflict-20260705-141233-abcdefg", None),
        ("Foo.sync-conflict-20260705-141233-ABCDEFGH", None),
        // Seafile: `<stem> (SFConflict [modifier ]YYYY-MM-DD-HH-MM-SS)`
        // (seafile/common/vc-common.c `gen_conflict_path`).
        (
            "Note (SFConflict me@example.com 2026-08-01-10-00-00)",
            Some("Note"),
        ),
        ("Note (SFConflict 2026-08-01-10-00-00)", Some("Note")),
        ("Note (SFConflict discussion)", None),
        ("Note (SFConflict 2026-08-01)", None),
        (
            "Note (SFConflict me@example.com 2026-08-01-10-00-00) extra",
            None,
        ),
        // Dropbox (behavior unchanged):
        ("Report (conflicted copy 2026-08-01)", Some("Report")),
        (
            "Report (Alice's conflicted copy 2026-08-01)",
            Some("Report"),
        ),
    ] {
        assert_eq!(sync_conflict_base(stem), base, "stem: {stem:?}");
    }
}
