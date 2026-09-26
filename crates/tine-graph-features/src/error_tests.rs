use super::*;
use tine_store::{FileId, IoError, Rollback, Store};

#[test]
fn rollback_failure_keeps_recovery_family_and_locations() {
    let root = std::env::temp_dir().join(format!("tine-tx-error-{}", std::process::id()));
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
