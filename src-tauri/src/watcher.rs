//! Device watch preference and the transport adapter for one graph window.

use crate::settings::{settings_path, update_settings};
use crate::state::{AppState, GraphSlot};
use std::path::Path;
use std::sync::{Arc, Weak};
use tauri::{Emitter, Manager, State};
use tine_core::model::PageKind;
use tine_store::{Change, ChangeKind, Origin, WatchMode};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct GraphChange {
    name: String,
    kind: PageKind,
    created: bool,
    removed: bool,
}

pub(crate) fn watch_mode(app: &tauri::AppHandle) -> WatchMode {
    let selected = settings_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| serde_json::from_str::<serde_json::Value>(&source).ok())
        .and_then(|settings| settings.get("watch_mode")?.as_str().map(str::to_owned));
    match selected.as_deref() {
        Some("poll") => WatchMode::Poll,
        Some("inotify") => WatchMode::Notify,
        _ if cfg!(target_os = "android") => WatchMode::Poll,
        _ => WatchMode::Notify,
    }
}

#[tauri::command]
pub(crate) fn get_watch_mode(app: tauri::AppHandle) -> String {
    match watch_mode(&app) {
        WatchMode::Notify => "inotify",
        WatchMode::Poll => "poll",
    }
    .into()
}

#[tauri::command]
pub(crate) fn set_watch_mode(
    mode: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mode = if mode == "poll" {
        WatchMode::Poll
    } else {
        WatchMode::Notify
    };
    update_settings(&app, |settings| {
        settings["watch_mode"] = serde_json::json!(match mode {
            WatchMode::Notify => "inotify",
            WatchMode::Poll => "poll",
        });
    })?;
    for (_, slot) in state.graphs.read().unwrap().entries() {
        slot.store.set_watch_mode(mode);
    }
    Ok(())
}

/// The v0.6.5 window events for one publication: `graph-changed` payloads in
/// file order, and whether `conflicts-changed` is due. Own changes emit none.
fn window_events(change: &Change) -> (Vec<GraphChange>, bool) {
    let mut events = Vec::new();
    let mut conflicts_dirty = false;
    if change.origin == Origin::Own {
        return (events, conflicts_dirty);
    }
    for (id, kind, _) in &change.files {
        if tine_core::model::path_is_sync_conflict(Path::new(id.as_str())) {
            conflicts_dirty = true;
        } else if let Some((page_kind, name)) = change.page(id) {
            events.push(GraphChange {
                name: name.to_owned(),
                kind: page_kind,
                created: matches!(kind, ChangeKind::Created),
                removed: matches!(kind, ChangeKind::Removed),
            });
        }
    }
    (events, conflicts_dirty)
}

fn dispatch(app: &tauri::AppHandle, label: &str, binding_generation: u64, change: Change) {
    let (events, conflicts_dirty) = window_events(&change);
    for event in events {
        let _ = app.emit_to(
            label,
            "graph-changed",
            serde_json::json!({
                "name": event.name,
                "kind": event.kind,
                "created": event.created,
                "removed": event.removed,
                "binding_generation": binding_generation,
            }),
        );
    }
    if conflicts_dirty {
        let _ = app.emit_to(label, "conflicts-changed", ());
    }
}

pub(crate) fn start_slot_events(app: tauri::AppHandle, label: String, slot: &Arc<GraphSlot>) {
    let subscription = slot.store.subscribe();
    let weak: Weak<GraphSlot> = Arc::downgrade(slot);
    std::thread::spawn(move || {
        while let Ok(change) = subscription.recv() {
            let Some(slot) = weak.upgrade() else {
                break;
            };
            let current = app.state::<AppState>().graphs.read().unwrap().slot(&label);
            if current
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &slot))
            {
                dispatch(&app, &label, slot.binding_generation, change);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scan-refresh an external edit and return the window events it yields.
    fn events_after(slot: &GraphSlot, edit: impl FnOnce()) -> Vec<GraphChange> {
        slot.store.whole_graph().unwrap();
        let subscription = slot.store.subscribe();
        edit();
        slot.store.scan_refresh().unwrap();
        let mut events = Vec::new();
        while let Some(change) = subscription.try_recv().unwrap() {
            events.extend(window_events(&change).0);
        }
        events
    }

    fn atomic_write(root: &Path, rel: &str, text: &str) {
        let temp = root.join(".adapter-write");
        std::fs::write(&temp, text).unwrap();
        std::fs::rename(temp, root.join(rel)).unwrap();
    }

    #[test]
    fn graph_changed_uses_the_store_page_name() {
        let root = std::env::temp_dir().join(format!(
            "tine-watch-names-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::write(root.join("pages/foo.md"), "title:: Bar\n\n- one\n").unwrap();
        std::fs::write(root.join("journals/2026_07_10.md"), "- day\n").unwrap();
        std::fs::write(root.join("journals/Jul 10th, 2026.md"), "- shadow\n").unwrap();
        let store = tine_store::Store::open(
            &root,
            tine_store::OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Poll,
            },
        )
        .unwrap()
        .0;
        let slot = GraphSlot::new(store, root.clone());
        let modified = |name: &str, kind| GraphChange {
            name: name.into(),
            kind,
            created: false,
            removed: false,
        };

        let events = events_after(&slot, || {
            atomic_write(&root, "pages/foo.md", "title:: Bar\n\n- two, longer\n")
        });
        // v0.6.5 names a page by its file even with `title::` (title identity
        // arrived in 0.6.90); the adapter must use the store's name, not its own.
        assert_eq!(events, vec![modified("foo", PageKind::Page)]);

        let events = events_after(&slot, || {
            atomic_write(&root, "journals/Jul 10th, 2026.md", "- shadow edited\n")
        });
        assert_eq!(events, vec![], "a shadow journal is not a graph page");

        let events = events_after(&slot, || {
            atomic_write(&root, "journals/2026_07_10.md", "- day two\n")
        });
        assert_eq!(events, vec![modified("Jul 10th, 2026", PageKind::Journal)]);

        let events = events_after(&slot, || {
            std::fs::remove_file(root.join("pages/foo.md")).unwrap()
        });
        assert_eq!(
            events,
            vec![GraphChange {
                name: "foo".into(),
                kind: PageKind::Page,
                created: false,
                removed: true,
            }]
        );
        drop(slot);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_page_changes_keep_the_v065_payload_shape() {
        let root = std::env::temp_dir().join(format!(
            "tine-watch-adapter-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        let store = tine_store::Store::open(
            &root,
            tine_store::OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Poll,
            },
        )
        .unwrap()
        .0;
        let slot = GraphSlot::new(store, root.clone());
        let event = |created, removed| GraphChange {
            name: "New".into(),
            kind: PageKind::Page,
            created,
            removed,
        };
        assert_eq!(
            events_after(&slot, || atomic_write(&root, "pages/New.md", "- new\n")),
            vec![event(true, false)]
        );
        let file = std::fs::File::options()
            .write(true)
            .open(root.join("pages/New.md"))
            .unwrap();
        assert_eq!(
            events_after(&slot, || file
                .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(5))
                .unwrap()),
            vec![],
            "a touch publishes a generation but no window event"
        );
        assert_eq!(
            events_after(&slot, || std::fs::remove_file(root.join("pages/New.md"))
                .unwrap()),
            vec![event(false, true)]
        );
        slot.store.whole_graph().unwrap();
        let subscription = slot.store.subscribe();
        atomic_write(&root, "pages/New.sync-conflict-2026.md", "- theirs\n");
        slot.store.scan_refresh().unwrap();
        let change = subscription
            .try_recv()
            .unwrap()
            .expect("conflict copy publishes");
        assert_eq!(window_events(&change), (vec![], true));
        drop(slot);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollback_external_bytes_dispatch_graph_changed() {
        use tine_store::{FaultPoint, PageId, SaveBase, TxOutcome};

        let root = std::env::temp_dir().join(format!(
            "tine-watch-rollback-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/A.md"), "- old A\n").unwrap();
        std::fs::write(root.join("pages/B.md"), "- old B\n").unwrap();
        let store = tine_store::Store::open(
            &root,
            tine_store::OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Poll,
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        let subscription = store.subscribe();
        let a = PageId::from("pages/A.md");
        let b = PageId::from("pages/B.md");
        let read_a = store.page(&a).unwrap();
        let read_b = store.page(&b).unwrap();
        let mut doc_a = read_a.doc;
        let mut doc_b = read_b.doc;
        doc_a.blocks[0].raw = "new A".into();
        doc_b.blocks[0].raw = "new B".into();
        let mut tx = store.transaction();
        tx.save_page(&a, SaveBase::Existing(read_a.rev), &doc_a);
        tx.save_page(&b, SaveBase::Existing(read_b.rev), &doc_b);
        store.inject_fault(FaultPoint::MidStepIoAt(1));
        store.inject_fault(FaultPoint::UndoLiveWrite);
        assert!(matches!(tx.commit(), TxOutcome::NotCommitted { .. }));
        let events: Vec<_> = std::iter::from_fn(|| subscription.try_recv().unwrap())
            .flat_map(|change| window_events(&change).0)
            .collect();
        assert_eq!(
            events,
            vec![GraphChange {
                name: "B".into(),
                kind: PageKind::Page,
                created: false,
                removed: false,
            }]
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
