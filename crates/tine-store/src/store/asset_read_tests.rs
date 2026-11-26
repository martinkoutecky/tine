//! Asset reads refuse a FIFO without waiting for a writer. Threat scenario
//! (sync delivery, external editors): a FIFO, socket or device node lands in
//! `assets/`; a blocking open would hang the read before it learns the path is
//! not a file.

use super::*;
use std::sync::mpsc;
use std::time::Duration;

fn graph_with_asset_fifo() -> (std::path::PathBuf, Store) {
    let root = crate::test_support::fresh_dir("tine-asset-fifo-");
    for dir in ["pages", "journals", "logseq", "assets"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("assets/real.png"), b"png").unwrap();
    let fifo = root.join("assets/pipe.png");
    let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let store = Store::open(
        &root,
        crate::OpenOptions {
            watch: crate::WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    (root, store)
}

/// Runs `read` on a worker; if it has not answered in 200 ms it is blocked in
/// `open`, so a writer is attached to release it and the test fails.
fn answers_without_a_writer<T: Send + 'static>(
    fifo: std::path::PathBuf,
    read: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || tx.send(read()).unwrap());
    let quick = rx.recv_timeout(Duration::from_millis(200));
    let was_quick = quick.is_ok();
    let result = quick.unwrap_or_else(|_| {
        let _writer = fs::OpenOptions::new().write(true).open(&fifo).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap()
    });
    worker.join().unwrap();
    assert!(was_quick, "the asset read waited for a FIFO writer");
    result
}

#[test]
fn a_fifo_in_assets_is_refused_without_blocking() {
    let (root, store) = graph_with_asset_fifo();
    let store = std::sync::Arc::new(store);
    let fifo = FileId::from("assets/pipe.png".to_owned());

    let (reader, file) = (store.clone(), fifo.clone());
    let read = answers_without_a_writer(root.join("assets/pipe.png"), move || {
        reader.read(&file, Some(1024)).map(|_| ())
    });
    assert!(
        matches!(read, Err(StoreError::InvalidTarget(_))),
        "{read:?}"
    );

    let (reader, file) = (store.clone(), fifo.clone());
    let stream = answers_without_a_writer(root.join("assets/pipe.png"), move || {
        reader.open_read(&file).map(|_| ())
    });
    assert!(
        matches!(stream, Err(StoreError::InvalidTarget(_))),
        "{stream:?}"
    );

    // A regular asset still reads as before.
    let real = FileId::from("assets/real.png".to_owned());
    assert_eq!(store.read(&real, None).unwrap().0, b"png");
    let (_, len) = store.open_read(&real).unwrap();
    assert_eq!(len, 3);
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_symlinked_asset_is_read_but_not_streamed() {
    let (root, store) = graph_with_asset_fifo();
    std::os::unix::fs::symlink(root.join("assets/real.png"), root.join("assets/link.png")).unwrap();
    let link = FileId::from("assets/link.png".to_owned());
    assert_eq!(store.read(&link, None).unwrap().0, b"png");
    assert!(matches!(
        store.open_read(&link),
        Err(StoreError::StreamSymlink(_))
    ));
    store.close();
    let _ = fs::remove_dir_all(root);
}
