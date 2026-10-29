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

    // s3.6 is a separately hash-linked supplement, not a relabelled s3.5
    // transcription or Lean proof. Check the actual ITF endpoint fixture.
    let model = fs::read_to_string(root.join("docs/storage-model/quint/storage-s36.qnt")).unwrap();
    let supplement: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("crates/tine-store/tests/fixtures/s2/s36-policy.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        supplement["model_sha256"],
        format!("{:x}", Sha256::digest(model.as_bytes()))
    );
    let scenarios = fs::read(root.join("docs/storage-model/quint/scenarios-s36.inc")).unwrap();
    assert_eq!(
        supplement["scenarios_sha256"],
        format!("{:x}", Sha256::digest(scenarios))
    );
    assert_eq!(supplement["cases"].as_array().unwrap().len(), 36);

    // Every published model and scenario file matches its SHA256SUMS line
    // (SB1-GITSYNC: storage-s3.qnt had drifted unnoticed), so an anchor in
    // the s3.7 git-pull model names the reviewed bytes.
    let quint = root.join("docs/storage-model/quint");
    let sums = fs::read_to_string(quint.join("SHA256SUMS")).unwrap();
    for line in sums.lines().filter(|line| !line.trim().is_empty()) {
        let (sum, file) = line.split_once("  ").expect("sha256sum line");
        let bytes = fs::read(quint.join(file)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            sum,
            "{file}: SHA256SUMS drift"
        );
    }
    let git_model = fs::read_to_string(quint.join("storage-s37-gitpull.qnt")).unwrap();
    assert!(sums.contains("  storage-s37-gitpull.qnt"));

    let source = fs::read_to_string(root.join("docs/storage-model/ASSUMPTIONS.md")).unwrap();
    let mut ids = BTreeSet::new();
    for line in source.lines().filter(|line| line.starts_with("| A-")) {
        let columns: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(columns.len(), 7, "malformed assumption row: {line}");
        let [id, _, anchor, guards, tests, _, status] = columns.as_slice() else {
            unreachable!()
        };
        assert!(ids.insert(*id), "duplicate assumption {id}");
        // `s37: ...` anchors name the s3.7 git-pull model (ADR 0078 §4).
        let anchor = anchor.trim_matches('`');
        let found = match anchor.strip_prefix("s37: ") {
            Some(anchor) => git_model.contains(anchor),
            None => model.contains(anchor),
        };
        assert!(found, "{id}: missing model anchor {anchor}");
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
                source.contains(&format!("{id} follow-up")),
                "{id}: unguarded row needs its named follow-up below the table"
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
        "A-GIT-PULL",
    ]
    .into_iter()
    .collect();
    assert!(
        required.is_subset(&ids),
        "M0: missing design §4 assumption rows"
    );
}
