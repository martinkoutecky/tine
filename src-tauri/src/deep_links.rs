//! External navigation (GH #181): launch URLs wait in a per-window queue until
//! that WebView subscribes and drains it. Resolution scans known graphs through
//! Store's existing identity/page/block doors; opening never writes graph data.
//! Cost O(known graph paths) for graph URLs, O(all known graph blocks) for a
//! block URL. Errors and ambiguous candidates are presented by the frontend.
use crate::state::{slot_for_context, AppState, GraphContext};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tine_core::model::PageKind;
use tine_store::{Resolved, Store};

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct LinkRequest {
    graph: Option<String>,
    page: Option<String>,
    block: Option<String>,
}
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct LinkTarget {
    pub(crate) root: String,
    #[serde(rename = "graphId")]
    graph_id: Option<String>,
    name: Option<String>,
    #[serde(rename = "pageKind")]
    page_kind: Option<PageKind>,
    path: Option<String>,
    block: Option<String>,
    error: Option<String>,
}
#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum Delivery {
    Url { url: String },
    Target { target: LinkTarget },
}
#[derive(Default)]
pub(crate) struct PendingLinks(Mutex<HashMap<String, Vec<Delivery>>>);

fn queue(app: &tauri::AppHandle, label: &str, delivery: Delivery) {
    app.state::<PendingLinks>()
        .0
        .lock()
        .unwrap()
        .entry(label.into())
        .or_default()
        .push(delivery);
    let _ = app.emit_to(label, "tine-link-pending", ());
}
pub(crate) fn receive_url(app: &tauri::AppHandle, url: String) {
    if !url.starts_with("tine:") {
        return;
    }
    let state = app.state::<AppState>();
    let label = state
        .last_focused
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| "main".into());
    queue(app, &label, Delivery::Url { url });
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        #[cfg(desktop)]
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
#[tauri::command]
pub(crate) fn take_tine_links(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Vec<Delivery> {
    app.state::<PendingLinks>()
        .0
        .lock()
        .unwrap()
        .remove(window.label())
        .unwrap_or_default()
}
#[tauri::command]
pub(crate) async fn graph_link_identity(
    path: Option<String>,
    context: GraphContext<'_>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if let Some(path) = path {
        if !crate::settings::list_known_graphs(app.clone())
            .iter()
            .any(|known| known.path == path)
        {
            return Err("Graph is not in the known-graph list".into());
        }
        return tauri::async_runtime::spawn_blocking(move || {
            let root =
                Store::canonical_root(std::path::Path::new(&path)).map_err(|e| e.to_string())?;
            let state = app.state::<AppState>();
            let owner = state.graphs.read().unwrap().owner(&root);
            if let Some(slot) =
                owner.and_then(|label| crate::state::slot_for_window(&state, &label).ok())
            {
                return slot.store.ensure_link_identity();
            }
            let (store, _, _) = Store::open(
                &root,
                tine_store::OpenOptions {
                    approved_external_assets: crate::settings::approved_external_assets(
                        &app, &root,
                    ),
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            store.ensure_link_identity()
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    let slot = slot_for_context(&context)?;
    tauri::async_runtime::spawn_blocking(move || slot.store.ensure_link_identity())
        .await
        .map_err(|e| e.to_string())?
}
fn valid_id(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn validate(request: &LinkRequest) -> Result<(), String> {
    if request.graph.as_deref().is_some_and(|id| !valid_id(id))
        || request.block.as_deref().is_some_and(|id| !valid_id(id))
        || request
            .page
            .as_ref()
            .is_some_and(|name| name.is_empty() || name.len() > 8192)
        || !matches!(
            (&request.graph, &request.page, &request.block),
            (Some(_), _, None) | (None, None, Some(_))
        )
    {
        return Err("Invalid Tine navigation target".into());
    }
    Ok(())
}
fn target_in_store(
    store: &Store,
    root: &str,
    request: &LinkRequest,
) -> Result<Option<LinkTarget>, String> {
    let view = store.whole_graph().map_err(|e| format!("{e:?}"))?;
    let mut target = LinkTarget {
        root: root.into(),
        graph_id: None,
        name: None,
        page_kind: None,
        path: None,
        block: None,
        error: None,
    };
    let (name, kind, block) = if let Some(id) = &request.block {
        let Some(group) = view
            .blocks(&[id.clone()])
            .map_err(|e| format!("{e:?}"))?
            .into_iter()
            .next()
            .flatten()
        else {
            return Ok(None);
        };
        (group.page, group.kind, Some(id.clone()))
    } else if let Some(name) = &request.page {
        // One existing physical page; no alias guessing or virtual creation.
        let kind = [PageKind::Page, PageKind::Journal]
            .into_iter()
            .find(|kind| {
                matches!(
                    view.resolve(name, *kind == PageKind::Journal),
                    Resolved::Existing { .. }
                )
            });
        let Some(kind) = kind else {
            return Ok(None);
        };
        (name.clone(), kind, None)
    } else {
        return Ok(Some(target));
    };
    let Resolved::Existing { id, .. } = view.resolve(&name, kind == PageKind::Journal) else {
        return Ok(None);
    };
    let page = store.page(&id).map_err(|e| format!("{e:?}"))?;
    target.name = Some(page.doc.name);
    target.page_kind = Some(kind);
    target.path = Some(page.id.as_str().into());
    target.block = block;
    Ok(Some(target))
}
#[tauri::command]
pub(crate) async fn scan_known_graphs_for_link(
    request: LinkRequest,
    app: tauri::AppHandle,
) -> Result<Vec<LinkTarget>, String> {
    validate(&request)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut candidates = Vec::new();
        let mut missing_copies = Vec::new();
        let mut errors = Vec::new();
        let mut roots = std::collections::HashSet::new();
        for known in crate::settings::list_known_graphs(app.clone()) {
            let Ok(root) = Store::canonical_root(std::path::Path::new(&known.path)) else { continue; };
            if !roots.insert(root.clone()) { continue; }
            let identity = Store::read_link_identity_at(&root);
            if let Some(id) = &request.graph {
                match &identity {
                    Ok(Some(found)) if found.eq_ignore_ascii_case(id) => {}
                    Ok(_) => continue,
                    Err(e) => { errors.push(e.clone()); continue; }
                }
            }
            let root_text = root.display().to_string();
            if request.page.is_none() && request.block.is_none() {
                candidates.push(LinkTarget { root: root_text, graph_id: identity.ok().flatten(), name: None, page_kind: None, path: None, block: None, error: None });
                continue;
            }
            let owner = state.graphs.read().unwrap().owner(&root);
            let result = if let Some(slot) = owner.and_then(|label| crate::state::slot_for_window(&state, &label).ok()) {
                target_in_store(&slot.store, &root_text, &request)
            } else {
                Store::open(&root, tine_store::OpenOptions {
                    approved_external_assets: crate::settings::approved_external_assets(&app, &root),
                    ..Default::default()
                }).map_err(|e| e.to_string()).and_then(|(store, _, _)| target_in_store(&store, &root_text, &request))
            };
            match result {
                Ok(Some(mut target)) => { target.graph_id = identity.ok().flatten(); candidates.push(target); },
                Ok(None) => {
                    if let Ok(Some(id)) = identity {
                        // A remembered copy remains authoritative even if its
                        // target was deleted. Never silently use another copy.
                        let missing = LinkTarget { root: root_text, graph_id: Some(id), name: request.page.clone(), page_kind: None, path: None, block: request.block.clone(), error: None };
                        if request.graph.is_some() { candidates.push(missing); }
                        else { missing_copies.push(missing); }
                    }
                },
                Err(e) => {
                    errors.push(e.clone());
                    if let Ok(Some(id)) = identity {
                        let unavailable = LinkTarget { root: root_text, graph_id: Some(id), name: request.page.clone(), page_kind: None, path: None, block: request.block.clone(), error: Some(e) };
                        if request.graph.is_some() { candidates.push(unavailable); }
                        else { missing_copies.push(unavailable); }
                    }
                }
            }
        }
        let ids: std::collections::HashSet<_> = candidates.iter().filter_map(|target| target.graph_id.clone()).collect();
        candidates.extend(missing_copies.into_iter().filter(|target| target.graph_id.as_ref().is_some_and(|id| ids.contains(id))));
        if candidates.is_empty() {
            let detail = errors.first().map(|e| format!(" ({e})")).unwrap_or_default();
            return Err(format!("Tine link target not found in any known graph. Open its graph in Tine first, then try the link again.{detail}"));
        }
        Ok(candidates)
    }).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) fn handoff_tine_link(
    target: LinkTarget,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<bool, String> {
    let root =
        Store::canonical_root(std::path::Path::new(&target.root)).map_err(|e| e.to_string())?;
    let owner = app.state::<AppState>().graphs.read().unwrap().owner(&root);
    if let Some(label) = owner.filter(|label| label != window.label()) {
        queue(&app, &label, Delivery::Target { target });
        if let Some(existing) = app.get_webview_window(&label) {
            let _ = existing.show();
            #[cfg(desktop)]
            let _ = existing.unminimize();
            let _ = existing.set_focus();
        }
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_targets_are_read_only_and_block_ids_survive_page_moves() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("pages")).unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        let source = format!("- linked\n  id:: {id}\n");
        std::fs::write(temp.path().join("pages/A.md"), &source).unwrap();
        let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
        let block = LinkRequest {
            graph: None,
            page: None,
            block: Some(id.into()),
        };
        assert_eq!(
            target_in_store(&store, "fixture", &block)
                .unwrap()
                .unwrap()
                .name
                .as_deref(),
            Some("A")
        );
        drop(store);
        std::fs::rename(
            temp.path().join("pages/A.md"),
            temp.path().join("pages/B.md"),
        )
        .unwrap();
        let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
        let target = target_in_store(&store, "fixture", &block).unwrap().unwrap();
        assert_eq!(target.name.as_deref(), Some("B"));
        assert_eq!(target.block.as_deref(), Some(id));
        let missing = LinkRequest {
            graph: Some(id.into()),
            page: Some("Missing".into()),
            block: None,
        };
        assert!(target_in_store(&store, "fixture", &missing)
            .unwrap()
            .is_none());
        assert!(!temp.path().join("pages/Missing.md").exists());
        assert!(!temp.path().join("logseq/tine-graph-id").exists());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("pages/B.md")).unwrap(),
            source
        );
    }
}
