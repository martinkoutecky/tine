use super::*;
use tine_store::{FileId, IoError, Rollback, Store};

#[test]
fn planning_busy_keeps_its_message_and_retryable_kind() {
    let message = "Tine is still finishing another change to these pages. Try again in a moment.";
    let error = why_error(Why::Failed(IoError {
        kind: io::ErrorKind::WouldBlock,
        message: message.into(),
    }));
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(error.to_string(), message);
}

#[test]
fn rollback_failure_keeps_recovery_family_and_locations() {
    let root = crate::test_support::fresh_dir("tine-tx-error-");
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let graph_rev = store.whole_graph().unwrap().rev();
    let file = FileId::from("pages/a.md".to_owned());
    let recovery = FileId::from("logseq/.tine-trash/recovery/a.md".to_owned());
    let outcome = TxOutcome::NotCommitted {
        step: 1,
        why: Why::Failed(IoError {
            kind: io::ErrorKind::PermissionDenied,
            message: "write failed".into(),
        }),
        rollback: Rollback {
            kept_external: vec![(file.clone(), Some(recovery.clone()))],
            undo_failed: vec![(
                file,
                IoError {
                    kind: io::ErrorKind::PermissionDenied,
                    message: "undo failed".into(),
                },
            )],
        },
        publication_errors: Vec::new(),
        graph_rev,
    };
    let wire = tx_error(outcome).unwrap_err().to_string();
    assert!(
        wire.starts_with("rollback-incomplete:"),
        "I-9: rollback must retain a fixed family; exemplar tx_error: {wire}"
    );
    assert!(
        wire.contains(".tine-trash/recovery/a.md"),
        "I-9: rollback must name recovery location; exemplar tx_error: {wire}"
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rollback_and_publication_failures_keep_both_locations_and_original_reason() {
    let root = crate::test_support::fresh_dir("tine-tx-combined-error-");
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let outcome = TxOutcome::NotCommitted {
        step: 1,
        why: Why::Failed(IoError {
            kind: io::ErrorKind::PermissionDenied,
            message: "original write failed".into(),
        }),
        rollback: Rollback {
            kept_external: vec![(
                FileId::from("pages/a.md".to_owned()),
                Some(FileId::from("logseq/.tine-trash/recovery/a.md".to_owned())),
            )],
            undo_failed: vec![(
                FileId::from("pages/a.md".to_owned()),
                IoError {
                    kind: io::ErrorKind::PermissionDenied,
                    message: "undo failed".into(),
                },
            )],
        },
        publication_errors: vec![(
            FileId::from("pages/b.md".to_owned()),
            IoError {
                kind: io::ErrorKind::Other,
                message: "publish failed".into(),
            },
        )],
        graph_rev: store.whole_graph().unwrap().rev(),
    };
    let wire = tx_error(outcome).unwrap_err().to_string();
    assert!(wire.contains("pages/a.md"), "{wire}");
    assert!(wire.contains(".tine-trash/recovery/a.md"), "{wire}");
    assert!(wire.contains("undo failed"), "{wire}");
    assert!(wire.contains("pages/b.md"), "{wire}");
    assert!(wire.contains("publish failed"), "{wire}");
    assert!(wire.contains("original write failed"), "{wire}");
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn fail_read_conflict_retry_keeps_failed_undo_and_publication() {
    let root = crate::test_support::fresh_dir("tine-fail-retry-");
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    for undo in [true, false] {
        let file = FileId::from("pages/a.md".to_owned());
        let error = || IoError {
            kind: io::ErrorKind::PermissionDenied,
            message: "injected failure".into(),
        };
        let outcome = TxOutcome::NotCommitted {
            step: 1,
            why: Why::Conflict {
                file: file.clone(),
                disk: None,
            },
            rollback: Rollback {
                kept_external: vec![(
                    file.clone(),
                    Some(FileId::from("logseq/.tine-trash/recovery/a.md".to_owned())),
                )],
                undo_failed: if undo {
                    vec![(file.clone(), error())]
                } else {
                    vec![]
                },
            },
            publication_errors: if undo { vec![] } else { vec![(file, error())] },
            graph_rev: store.whole_graph().unwrap().rev(),
        };
        let error = commit_retry(outcome).expect_err(
            "I-2/I-9: incomplete rollback/publication must never be retried as a clean conflict",
        );
        assert!(error.to_string().contains("incomplete"));
    }
    store.close();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn fail_read_asset_inventory_cannot_be_an_empty_success() {
    let root = crate::test_support::fresh_dir("tine-fail-assets-");
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    std::fs::write(root.join("assets"), "not a directory").unwrap();
    assert!(
        format!("{:?}", assets::orphan_assets(&store)).contains("Err"),
        "I-2: an unreadable assets inventory cannot mean no orphans"
    );
    store.close();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_config_refusal_after_a_partial_save_keeps_its_family_and_what_was_saved() {
    // Review of HVgII: the PDF highlights sidecar is saved, then planning the
    // annotation page is refused because config.edn became unreadable.
    let refused = why_error(Why::Refused(Refusal::ConfigUnreadable(IoError {
        kind: io::ErrorKind::InvalidData,
        message: "invalid utf-8 sequence of 1 bytes from index 35".into(),
    })));
    let error = after_partial("The highlights were saved", refused);
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error
        .get_ref()
        .is_some_and(|cause| cause.is::<ConfigUnreadable>()));
    assert_eq!(
        error.to_string(),
        "config-unreadable-after: The highlights were saved\ninvalid utf-8 sequence of 1 bytes from index 35"
    );
    // Any other failure keeps the plain prefix.
    let other = after_partial("The merge was saved", io::Error::other("Disk full"));
    assert_eq!(other.to_string(), "The merge was saved, but disk full");
}
