//! Where a native capture (Android photo or voice memo) lands (og H1b).
//!
//! The cache file handed back by the native recorder or camera is the capture's
//! only copy, and the user cannot reach it. A capture started in graph A that
//! finishes after the window switched to graph B must therefore be kept, and
//! kept in A: the frontend names the graph the capture was STARTED in, and it is
//! written there through the ordinary audited asset transaction, never into B.
//! This is the asset counterpart of og-T's `keepAtSwitch`, which writes a draft
//! for an explicit root after the binding moved (`drafts::store_draft`).
//!
//! The caller is still a live bound window (`slot_for_context` in the command).
//! Resolution, cheapest first: the window's own graph; another window that has
//! the graph open (its live store); otherwise a known graph opened for this one
//! write and closed again.
//!
//! Refusal: a root that is not a known graph on this device. Threat scenario
//! (in scope, AGENTS.md storage trust boundary): plugins or web content running
//! in the WebView naming an arbitrary directory as a write target. The recorded
//! graph came from `graphMeta().root`, which is always a graph this device has
//! opened, so an honest capture never meets the refusal.
//!
//! Unit cost: no persisted record beyond the asset file itself (one file per
//! capture, as before). The rare cross-graph case adds one store open of the
//! target graph, O(page directory listing), with its parse worker closed after
//! the write.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tine_store::{OpenOptions, Store, WatchMode};

use crate::state::{GraphContext, GraphSlot};

pub(crate) enum CaptureTarget {
    Bound(Arc<GraphSlot>),
    Opened(Store),
}

impl CaptureTarget {
    pub(crate) fn store(&self) -> &Store {
        match self {
            Self::Bound(slot) => &slot.store,
            Self::Opened(store) => store,
        }
    }

    pub(crate) fn asset_error(&self, error: std::io::Error) -> String {
        tine_graph_features::assets::error_for_user(self.store(), error)
    }
}

impl Drop for CaptureTarget {
    fn drop(&mut self) {
        if let Self::Opened(store) = self {
            store.close();
        }
    }
}

/// The command's entry: the device's known graphs and asset approvals from app
/// settings, the live stores from the window registry.
pub(crate) fn pick(
    window_slot: Arc<GraphSlot>,
    graph_root: Option<String>,
    app: &tauri::AppHandle,
    ctx: &GraphContext<'_>,
) -> Result<CaptureTarget, String> {
    let known: Vec<String> = crate::settings::list_known_graphs(app.clone())
        .into_iter()
        .map(|graph| graph.path)
        .collect();
    resolve(
        window_slot,
        graph_root.as_deref(),
        |root| {
            let graphs = ctx.state.graphs.read().unwrap();
            graphs.owner(root).and_then(|window| graphs.slot(&window))
        },
        &known,
        |root| crate::settings::approved_external_assets(app, root),
    )
}

/// Resolve the graph a capture belongs to. `graph_root: None` is the window's
/// own graph (the pre-H1b behaviour). `open_elsewhere` finds another window's
/// live slot for a root; `known` lists the device's known graph paths;
/// `approved_assets` returns a graph's approved external `assets/` link.
pub(crate) fn resolve(
    window_slot: Arc<GraphSlot>,
    graph_root: Option<&str>,
    open_elsewhere: impl Fn(&Path) -> Option<Arc<GraphSlot>>,
    known: &[String],
    approved_assets: impl Fn(&Path) -> Option<PathBuf>,
) -> Result<CaptureTarget, String> {
    let Some(root) = graph_root else {
        return Ok(CaptureTarget::Bound(window_slot));
    };
    let root = crate::state::canonical_graph_root(root)?;
    if root == window_slot.root_key {
        return Ok(CaptureTarget::Bound(window_slot));
    }
    if let Some(slot) = open_elsewhere(&root) {
        return Ok(CaptureTarget::Bound(slot));
    }
    let is_known = known
        .iter()
        .any(|path| crate::state::canonical_graph_root(path).is_ok_and(|known| known == root));
    if !is_known {
        // Scenario: plugin/web content naming an arbitrary directory (module doc).
        return Err(format!(
            "capture target {} is not a known graph",
            root.display()
        ));
    }
    let (store, _, _) = Store::open(
        &root,
        OpenOptions {
            approved_external_assets: approved_assets(&root),
            watch: WatchMode::Poll,
            launch_checkpoint: None,
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(CaptureTarget::Opened(store))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn graph(dir: &Path, name: &str) -> PathBuf {
        let root = dir.join(name);
        for area in ["pages", "journals", "assets"] {
            fs::create_dir_all(root.join(area)).unwrap();
        }
        fs::canonicalize(root).unwrap()
    }

    fn slot(root: &Path) -> Arc<GraphSlot> {
        let store = Store::open(root, OpenOptions::default()).unwrap().0;
        Arc::new(GraphSlot::new(store, root.to_path_buf()))
    }

    fn write(target: &CaptureTarget, name: &str) -> String {
        tine_graph_features::assets::import_asset_file(
            target.store(),
            name,
            tine_store::Content::Bytes(b"memo".to_vec()),
        )
        .unwrap()
    }

    #[test]
    fn a_capture_finished_after_a_switch_lands_in_the_graph_it_started_in() {
        let dir = tempfile::tempdir().unwrap();
        let (started, current) = (graph(dir.path(), "A"), graph(dir.path(), "B"));
        let known = vec![started.display().to_string(), current.display().to_string()];
        let target = resolve(
            slot(&current),
            Some(started.to_str().unwrap()),
            |_| None,
            &known,
            |_| None,
        )
        .unwrap();
        assert!(matches!(target, CaptureTarget::Opened(_)));
        let stored = write(&target, "memo.m4a");
        drop(target);
        assert_eq!(
            fs::read(started.join("assets").join(&stored)).unwrap(),
            b"memo"
        );
        assert_eq!(fs::read_dir(current.join("assets")).unwrap().count(), 0);
    }

    #[test]
    fn the_window_graph_and_a_graph_open_elsewhere_use_their_live_store() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (graph(dir.path(), "A"), graph(dir.path(), "B"));
        let own = slot(&a);
        let target = resolve(
            own.clone(),
            Some(a.to_str().unwrap()),
            |_| None,
            &[],
            |_| None,
        );
        assert!(matches!(&target, Ok(CaptureTarget::Bound(s)) if Arc::ptr_eq(s, &own)));
        let mut registry = crate::state::GraphRegistry::default();
        let other = slot(&b);
        registry.bind("other".into(), other.clone()).unwrap();
        let elsewhere = |root: &Path| registry.owner(root).and_then(|w| registry.slot(&w));
        let target = resolve(own, Some(b.to_str().unwrap()), elsewhere, &[], |_| None);
        assert!(matches!(&target, Ok(CaptureTarget::Bound(s)) if Arc::ptr_eq(s, &other)));
    }

    #[test]
    fn a_root_that_is_not_a_known_graph_is_refused_and_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let (current, stranger) = (graph(dir.path(), "B"), graph(dir.path(), "elsewhere"));
        let known = vec![current.display().to_string()];
        let refused = resolve(
            slot(&current),
            Some(stranger.to_str().unwrap()),
            |_| None,
            &known,
            |_| None,
        );
        assert!(matches!(refused, Err(message) if message.contains("not a known graph")));
        assert_eq!(fs::read_dir(stranger.join("assets")).unwrap().count(), 0);
    }
}
