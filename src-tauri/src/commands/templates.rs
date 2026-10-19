//! Early answers for an empty today's daily template (BqbeQ): the configured
//! template and the absent day's file id, without waiting for the graph-wide
//! parse when the graph's files settle them.
use super::{GraphSlot, PageKind, Resolved, ResolvedWire, StoreError, WholeGraph};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tine_core::model::TemplateDto;
use tine_graph_features::templates::{named_template_from_files, NamedTemplate};

/// Whether this binding's launch warm already holds a graph view (a served
/// launch checkpoint or the loaded graph; `graph::warm_cache_async` sets it
/// once `whole_graph` returned). The early file answers stand down then: the
/// view answers at once. Before it flips, the named-template race below still
/// lets an arriving view stop the file lookup.
fn view_at_hand(slot: &GraphSlot) -> bool {
    slot.warm_done.load(Ordering::Acquire)
}

/// A journal day no file claims is absent at its configured id
/// (`absent_journal_id`); `None` leaves the answer to the graph.
pub(super) fn early_absent_journal(
    slot: &GraphSlot,
    name: &str,
    kind: PageKind,
) -> Option<Result<ResolvedWire, String>> {
    if kind != PageKind::Journal || view_at_hand(slot) {
        return None;
    }
    match tine_graph_features::journals::absent_journal_id(&slot.store, name) {
        Ok(Some(id)) => Some(Ok(Resolved::Absent { id }.into())),
        Err(StoreError::Closed) => Some(Err("graph load failed: Closed".to_owned())),
        // Claimed, not a day, or a read error: the graph-wide answer decides.
        Ok(None) | Err(_) => None,
    }
}

/// The template graph-wide discovery picks for `name` (at most one). The file
/// lookup and the graph wait run side by side on the blocking pool and the
/// first usable answer wins: a graph view (already loaded, or a served launch
/// checkpoint) answers at once and stops the file lookup between files; a
/// settled file answer returns without the graph, whose wait then ends with
/// the load. With a view already at hand only the graph answers. Cost: see
/// `named_template_from_files`.
pub(super) async fn named_template(
    slot: Arc<GraphSlot>,
    name: String,
) -> Result<Vec<TemplateDto>, String> {
    enum First {
        Graph(Result<WholeGraph, tine_store::LoadError>),
        Files(std::io::Result<NamedTemplate>),
    }
    let files = !view_at_hand(&slot);
    let arrived = Arc::new(AtomicBool::new(false));
    let (send, answers) = std::sync::mpsc::channel();
    {
        let (slot, arrived, send) = (Arc::clone(&slot), Arc::clone(&arrived), send.clone());
        tauri::async_runtime::spawn_blocking(move || {
            let view = slot.store.whole_graph();
            arrived.store(true, Ordering::Release);
            let _ = send.send(First::Graph(view));
        });
    }
    if files {
        let wanted = name.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let stop = || arrived.load(Ordering::Acquire);
            let _ = send.send(First::Files(named_template_from_files(
                &slot.store,
                &wanted,
                stop,
            )));
        });
    }
    tauri::async_runtime::spawn_blocking(move || loop {
        match answers.recv() {
            Ok(First::Graph(view)) => {
                let view = view.map_err(|e| format!("graph load failed: {e:?}"))?;
                let found = view.templates().into_iter().find(|t| t.name == name);
                return Ok(found.into_iter().collect());
            }
            Ok(First::Files(Ok(NamedTemplate::Settled(found)))) => {
                return Ok(found.into_iter().collect())
            }
            // Unsettled, or a read error (a closed store also ends the graph
            // wait): the graph answers.
            Ok(First::Files(_)) => continue,
            Err(_) => return Err("template lookup ended without an answer".to_owned()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BqbeQ: the early absent-day answer runs only until the launch warm
    /// holds a view (a served launch checkpoint answers at once).
    #[test]
    fn the_early_absent_day_answer_stands_down_once_a_view_is_at_hand() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for area in ["pages", "journals", "logseq"] {
            std::fs::create_dir_all(root.join(area)).unwrap();
        }
        std::fs::write(root.join("pages/A.md"), "- a\n").unwrap();
        std::fs::write(root.join(".tine-test-pause-load"), "").unwrap();
        let store = tine_store::Store::open(root, Default::default()).unwrap().0;
        let slot = GraphSlot::new(store, root.to_path_buf());
        let today = tine_core::date::JournalDate::today().title();
        assert!(matches!(
            early_absent_journal(&slot, &today, PageKind::Journal),
            Some(Ok(ResolvedWire::Absent { .. }))
        ));
        assert!(early_absent_journal(&slot, "A", PageKind::Page).is_none());
        slot.warm_done.store(true, Ordering::Release);
        assert!(
            early_absent_journal(&slot, &today, PageKind::Journal).is_none(),
            "with a view at hand the graph answers"
        );
        std::fs::remove_file(root.join(".tine-test-pause-load")).unwrap();
        slot.store.close();
    }
}
