//! Test fixture: a page save as production runs it. It plans with the existing
//! `PagePlan` and hands the operation to a real page host, the only page
//! writer; it writes nothing itself. Included through `#[path]` in test targets
//! only, each naming its own build of tine-store as `store_crate`.
#![allow(dead_code)]

use super::store_crate::{ApplyRefusal, EditKind, FileRev, PageHost, PageId, SaveBase, Store, Why};
use std::sync::Arc;
use tine_core::model::PageDto;

/// Which owner refused the save. Nothing reached disk.
#[derive(Debug)]
pub(crate) enum SaveRefused {
    Planned(Why),
    Host(ApplyRefusal),
}

/// One operation's own changes as one value: the host publishes page by page
/// where a single transaction once published all at once. Lists and flags
/// accumulate; a later publication's scalars (revision, counts) win.
pub(crate) fn merged_changes(
    changes: impl IntoIterator<Item = serde_json::Value>,
) -> serde_json::Value {
    use serde_json::Value;
    fn merge(into: &mut Value, from: Value) {
        match (into, from) {
            (Value::Object(into), Value::Object(from)) => {
                for (key, value) in from {
                    match into.get_mut(&key) {
                        Some(slot) => merge(slot, value),
                        None => {
                            into.insert(key, value);
                        }
                    }
                }
            }
            (Value::Array(into), Value::Array(from)) => {
                for value in from {
                    if !into.contains(&value) {
                        into.push(value);
                    }
                }
            }
            (Value::Bool(into), Value::Bool(from)) => *into |= from,
            (into, from) => *into = from,
        }
    }
    let mut merged = Value::Object(Default::default());
    for change in changes {
        merge(&mut merged, change);
    }
    merged
}

/// Plan one page save and run it on `host`. Returns the revision of the bytes
/// now on disk, read back rather than assumed.
pub(crate) fn save_on(
    host: &PageHost,
    store: &Store,
    kind: EditKind,
    id: &PageId,
    base: SaveBase,
    doc: &PageDto,
) -> Result<FileRev, SaveRefused> {
    let mut plan = store.page_plan(Some(kind));
    plan.save_page(&[kind], id, base, doc);
    let op = plan.plan().map_err(SaveRefused::Planned)?;
    host.apply(op).map_err(SaveRefused::Host)?;
    Ok(store.read(&id.file(), None).unwrap().1)
}

struct Running {
    store: Arc<Store>,
    host: PageHost,
    _app_data: tempfile::TempDir,
}

/// A running host for one test, under disposable app data. Clones share it, so
/// a test's writer thread uses the same host. Drop it before removing the graph.
#[derive(Clone)]
pub(crate) struct HostWriter(Arc<Running>);

impl std::ops::Deref for HostWriter {
    type Target = PageHost;
    fn deref(&self) -> &PageHost {
        &self.0.host
    }
}

impl HostWriter {
    pub(crate) fn start(store: &Arc<Store>) -> Self {
        let app_data = tempfile::tempdir().unwrap();
        let host = PageHost::start_for_tests(store, app_data.path()).unwrap();
        Self(Arc::new(Running {
            store: store.clone(),
            host,
            _app_data: app_data,
        }))
    }

    /// This thread's host for `store`, kept across calls as a session keeps its
    /// one host: a helper that has no writer to hand takes it from here.
    pub(crate) fn for_store(store: &Arc<Store>) -> Self {
        thread_local! {
            static CURRENT: std::cell::RefCell<Option<HostWriter>> = const { std::cell::RefCell::new(None) };
        }
        CURRENT.with(|current| {
            let mut current = current.borrow_mut();
            match current.as_ref() {
                Some(writer) if Arc::ptr_eq(&writer.0.store, store) => writer.clone(),
                _ => current.insert(Self::start(store)).clone(),
            }
        })
    }

    /// The host has nothing held or pending, for a test whose watcher cannot
    /// drain (the load is paused, or the test stopped it).
    pub(crate) fn settle(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !self.quiescent_for_tests() {
            assert!(std::time::Instant::now() < deadline, "host did not settle");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Save one page and wait for its publication.
    pub(crate) fn save(
        &self,
        kind: EditKind,
        id: &PageId,
        base: SaveBase,
        doc: &PageDto,
    ) -> Result<FileRev, SaveRefused> {
        save_on(&self.0.host, &self.0.store, kind, id, base, doc)
    }
}
