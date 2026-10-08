//! GH #623 item 3: folding a block writes the file and nothing derived is
//! rebuilt for it — the alias list is inherited, unaffected answers survive,
//! and the launch checkpoint is not rewritten; the fold still survives a
//! warm relaunch through the launch diff.
use super::*;
use std::time::Duration;

fn graph() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    for (rel, text) in [
        ("logseq/config.edn", "{:preferred-format :markdown}\n"),
        ("pages/A.md", "- parent [[T]]\n  - child\n"),
        ("pages/B.md", "- other [[T]] [[U]]\n"),
        ("pages/C.md", "alias:: Cee\n\n- c\n"),
    ] {
        let path = root.path().join(rel);
        fs::write(&path, text).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
    }
    root
}

fn open(root: &Path, checkpoint: &Path) -> Store {
    Store::open(
        root,
        OpenOptions {
            watch: WatchMode::Poll,
            launch_checkpoint: Some(checkpoint.to_path_buf()),
            ..Default::default()
        },
    )
    .unwrap()
    .0
}

fn save_first_block(store: &Store, rel: &str, edit: impl FnOnce(&str) -> String) {
    let id = PageId::from(rel);
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks[0].raw = edit(&doc.blocks[0].raw);
    assert!(matches!(
        store.save(
            crate::EditKind::SaveBlock,
            &id,
            SaveBase::Existing(read.rev),
            &doc
        ),
        SaveOutcome::Saved(_)
    ));
}

fn dirty(store: &Store) -> bool {
    store.changes.checkpoint.get().unwrap().dirty()
}

fn backlink_raws(view: &WholeGraph, name: &str) -> Vec<String> {
    view.backlinks(name)
        .unwrap()
        .iter()
        .flat_map(|group| group.blocks.iter().map(|block| block.raw.clone()))
        .collect()
}

#[test]
fn a_fold_rebuilds_nothing_and_survives_a_warm_relaunch() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("graph.bin");
    let store = open(root.path(), &cp);
    let view = store.whole_graph_reconciled().unwrap();
    // A session's derived state: backlinks answers and the alias list.
    let unaffected = view.backlinks("U").unwrap();
    assert_eq!(backlink_raws(&view, "T").len(), 2);
    assert!(!view.graph.page_aliases().is_empty());
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(checkpoint::CheckpointWrite::Written { .. })
    ));
    assert!(!dirty(&store));

    save_first_block(&store, "pages/A.md", |raw| {
        format!("{raw}\ncollapsed:: true")
    });
    assert!(
        fs::read_to_string(root.path().join("pages/A.md"))
            .unwrap()
            .contains("collapsed:: true"),
        "the fold is on disk"
    );
    let folded = store.whole_graph().unwrap();
    assert!(
        folded.graph.alias_list_built(),
        "GH #623 item 3 / I-25: a fold inherits the alias list instead of walking every page \
         at the next read; exemplar ReadSnapshot::carry_alias_list_from"
    );
    assert!(
        !dirty(&store),
        "GH #623 item 3 / I-25: a fold does not rewrite the launch checkpoint; the next launch \
         diff rereads the one folded file (ADR 0070)"
    );
    assert!(
        Arc::ptr_eq(&unaffected, &folded.backlinks("U").unwrap()),
        "an answer that does not hold the folded page is carried"
    );
    assert!(
        backlink_raws(&folded, "T")
            .iter()
            .any(|raw| raw.contains("collapsed:: true")),
        "an answer holding the folded block shows its current text"
    );
    drop((view, folded));

    // Control: an ordinary edit still marks the checkpoint for rewriting.
    save_first_block(&store, "pages/B.md", |raw| format!("{raw} more"));
    assert!(dirty(&store), "a text edit is checkpointed");
    store.close();

    // Relaunch warm from the checkpoint written before the fold: the launch
    // diff rereads A, so the fold is not lost.
    fs::write(root.path().join("pages/B.md"), "- other [[T]] [[U]]\n").unwrap();
    let warm = open(root.path(), &cp);
    let view = warm.whole_graph_reconciled().unwrap();
    assert_eq!(
        warm.diagnostics()["checkpoint"]["load"]["outcome"].as_str(),
        Some("loaded")
    );
    assert!(
        warm.page(&PageId::from("pages/A.md")).unwrap().doc.blocks[0]
            .raw
            .contains("collapsed:: true")
    );
    assert!(backlink_raws(&view, "T")
        .iter()
        .any(|raw| raw.contains("collapsed:: true")));
    drop(view);
    warm.close();
}
