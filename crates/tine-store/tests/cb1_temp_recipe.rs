//! The one write-temp, fsync, rename recipe, counted where it runs: a created
//! file costs one temp payload and two fsyncs (the temp, its directory),
//! whichever helper writes it. A streamed create writes twice (its stage, then
//! the file), and both copies are counted; before the recipe was shared, the
//! copy helpers' four fsyncs and two payloads were invisible to these counters.
use std::fs;
use tine_store::{cost_counters, Area, Content, Store, TxOutcome};

#[test]
fn a_streamed_create_counts_both_of_its_writes() {
    let _integration_guard = crate::exclusive_integration_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("assets")).unwrap();
    let source = dir.path().join("source.bin");
    fs::write(&source, b"streamed bytes").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let mut measured = vec![];
    for (name, streamed) in [("bytes.bin", false), ("stream.bin", true)] {
        let id = store.file_id(Area::Assets, name).unwrap();
        let content = if streamed {
            Content::Stream {
                source: fs::File::open(&source).unwrap(),
                max_bytes: 1024,
            }
        } else {
            Content::Bytes(b"streamed bytes".to_vec())
        };
        cost_counters::reset();
        let mut tx = store.transaction(None);
        tx.create(&id, content);
        let outcome = tx.commit();
        let counts = cost_counters::snapshot();
        assert!(
            matches!(outcome, TxOutcome::Committed { .. }),
            "{outcome:?}"
        );
        assert_eq!(store.read(&id, None).unwrap().0, b"streamed bytes");
        measured.push((
            name,
            counts.fsyncs,
            counts.files_written,
            counts.bytes_written,
        ));
    }
    let (_, bytes_syncs, bytes_files, bytes_written) = measured[0];
    assert_eq!((bytes_files, bytes_written), (1, 14), "{measured:?}");
    assert_eq!(
        measured[1],
        ("stream.bin", bytes_syncs + 2, 2, 28),
        "a streamed create: the stage's write and the copy's, each one payload and two fsyncs"
    );
    let names: Vec<String> = fs::read_dir(dir.path().join("assets"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "no stage or temp is left: {names:?}");
    store.close();
}
