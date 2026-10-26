use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tine_core::model::PageKind;
use tine_store::{PageHost, PageOperation, Store};

pub(crate) fn atomic_write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = path.as_ref();
    let root = path.parent().and_then(Path::parent).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "fixture path has no graph root",
        )
    })?;
    let temp = root.join(format!(
        ".fixture-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(temp, path)
}

/// Renames and deletes are the page host's (STEP3 §7): each runs through a
/// host started for it under its own app data, as the app's graph binding
/// runs one.
pub(crate) fn hosted<T>(store: &Arc<Store>, run: impl FnOnce(&Store, &PageHost) -> T) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(store, &host)
}

/// A page's deletion as the window runs it (STEP3 §7, §4.4, Q-P2b-5):
/// `page_delete` by name, kind and displayed path through the GH #620
/// identity checks, then `page_owed` and `page_wait` until it is published.
/// Returns the operation and whether the deletion was published (`None`
/// when it was not `Pending`); an identity refusal is the error.
pub(crate) fn delete(
    store: &Arc<Store>,
    name: &str,
    kind: PageKind,
    expected_path: Option<&str>,
) -> io::Result<(PageOperation, Option<bool>)> {
    hosted(store, |store, host| {
        let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
            .as_u64()
            .unwrap();
        let file = match store
            .whole_graph()
            .unwrap()
            .resolve(name, kind == PageKind::Journal)
        {
            tine_store::Resolved::Existing { id, .. } => {
                Some(store.path_for_os_handoff(&id.file(), false).unwrap())
            }
            _ => None,
        };
        let operation = tine_graph_features::pages::delete_page_expected(
            store,
            host,
            session,
            name,
            kind,
            expected_path,
        )?;
        // Finding B: `Applied` once the deletion's own save published it;
        // `Pending` when it cannot publish without the user. The index
        // publication is the window's barrier, waited for here.
        if !matches!(operation, PageOperation::Applied | PageOperation::Pending) {
            return Ok((operation, None));
        }
        let needs: Vec<_> = host
            .owed(session, None)
            .unwrap()
            .into_iter()
            .map(|(key, version)| (key, version, None))
            .collect();
        let bound = std::time::Duration::from_secs(20);
        let published = host.wait_published(session, &needs, bound) == PageOperation::Applied;
        // Q-P2b-4: `Applied` vouches for the disk, at once.
        if let Some(file) = file.filter(|_| published) {
            assert!(
                !file.exists(),
                "page_wait vouched for {name} before its file moved"
            );
        }
        Ok((operation, Some(published)))
    })
}
