use super::i01_write_protocol_census::{reference_exists, repo};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;

#[test]
fn assumption_anchors_guards_tests_and_model_hash_exist() {
    let _integration_guard = crate::integration_lock();
    let root = repo();
    let model = fs::read_to_string(root.join("docs/storage-model/quint/storage-s35.qnt")).unwrap();
    // G8: connect the actual model bytes to the hash already pinned by the
    // executable transcription and its replay fixtures (not just hash = hash).
    let hash = format!("{:x}", Sha256::digest(model.as_bytes()));
    let transcription =
        fs::read_to_string(root.join("crates/tine-store/src/page_state/mod.rs")).unwrap();
    assert!(
        transcription.contains(&format!("const MODEL_SHA: &str = \"{hash}\";")),
        "G8: model drift; regenerate and review the transcription/replay evidence"
    );
    let fixture: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("crates/tine-store/tests/fixtures/s2/scenarios.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(fixture["model_sha256"], hash);

    let source = fs::read_to_string(root.join("docs/storage-model/ASSUMPTIONS.md")).unwrap();
    let mut ids = BTreeSet::new();
    for line in source.lines().filter(|line| line.starts_with("| A-")) {
        let columns: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(columns.len(), 7, "malformed assumption row: {line}");
        let [id, _, anchor, guards, tests, _, status] = columns.as_slice() else {
            unreachable!()
        };
        assert!(ids.insert(*id), "duplicate assumption {id}");
        assert!(
            model.contains(anchor.trim_matches('`')),
            "{id}: missing model anchor {anchor}"
        );
        assert!(
            [
                "guarded+tested",
                "guarded-untested",
                "unguarded",
                "assumed-outside"
            ]
            .contains(status),
            "{id}: invalid status {status}"
        );
        for (cell, is_test) in [(guards, false), (tests, true)] {
            if *cell != "—" {
                for reference in cell.split("<br>") {
                    assert!(
                        reference_exists(reference.trim_matches('`'), is_test),
                        "{id}: missing {} {reference}",
                        if is_test { "test" } else { "guard" }
                    );
                }
            }
        }
        if *status == "guarded+tested" {
            assert!(
                *guards != "—" && *tests != "—",
                "{id}: guarded+tested needs both references"
            );
        }
        if *status == "guarded-untested" {
            assert_ne!(*guards, "—", "{id}: guarded-untested needs a guard");
        }
        if *status == "unguarded" {
            assert!(
                source.contains(&format!("[{id} follow-up](")),
                "{id}: unguarded row needs its queue link below the table"
            );
        }
    }
    let required: BTreeSet<_> = [
        "A-PATH-IDENTITY",
        "A-ROOT-BOUND",
        "A-DRAFT-NAMESPACE",
        "A-ONE-HOST",
        "A-DIRSYNC",
        "A-ATOMIC-READ",
        "A-DRAFT-DURABLE",
        "A-CASE-DISTINCT",
        "A-ATOMIC-REPLACE",
        "A-DELETE-ONE-MOVE",
        "A-TRASH-COUPLED",
        "A-NAME-SLOT",
        "A-EXT-WHOLE",
        "A-PROCESS-CUT",
        "A-REFS-GIVEN",
        "A-OPAQUE-TEXT",
        "A-PROGRESS",
    ]
    .into_iter()
    .collect();
    assert!(
        required.is_subset(&ids),
        "M0: missing design §4 assumption rows"
    );
}
