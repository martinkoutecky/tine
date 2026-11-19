use super::{query_error_wire::reference_error, slot_for_context, GraphContext};
use serde::Serialize;
use std::sync::Arc;
use tine_core::model::RefGroup;
use tine_store::{FileRev, PageId, QueryError, WholeGraph};

type ExportSource = (PageId, Option<FileRev>, Vec<String>);

#[derive(Serialize)]
pub(crate) struct ReferenceGroupsWire {
    groups: Arc<Vec<RefGroup>>,
    sources: Vec<Option<ExportSource>>,
}

async fn reference_groups(
    name: String,
    state: GraphContext<'_>,
    read: fn(&WholeGraph, &str) -> Result<Arc<Vec<RefGroup>>, QueryError>,
) -> Result<ReferenceGroupsWire, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let view = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        let groups = read(&view, &name).map_err(reference_error)?;
        let sources = groups
            .iter()
            .map(|group| slot.store.reference_source(&view, group))
            .collect();
        Ok(ReferenceGroupsWire { groups, sources })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn get_backlinks(
    name: String,
    state: GraphContext<'_>,
) -> Result<ReferenceGroupsWire, String> {
    reference_groups(name, state, WholeGraph::backlinks).await
}

#[tauri::command]
pub(crate) async fn get_unlinked_refs(
    name: String,
    state: GraphContext<'_>,
) -> Result<ReferenceGroupsWire, String> {
    reference_groups(name, state, WholeGraph::unlinked_references).await
}
