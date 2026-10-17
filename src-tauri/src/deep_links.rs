//! External navigation (GH #181): launch URLs wait in a per-window queue until
//! that WebView subscribes and drains it. Resolution scans known graphs through
//! Store's existing identity/page/block doors; opening never writes graph data.
//! Closed graphs first get a read-only byte/name prefilter; only possible
//! matches start a full Store. Errors and ambiguous candidates are presented
//! by the frontend.
use crate::state::{slot_for_context, AppState, GraphContext};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tine_core::model::PageKind;
use tine_store::link_identity::LinkProbe;
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
    #[serde(default, rename = "displayRoot")]
    display_root: String,
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
fn page_holds_block(blocks: &[tine_core::model::BlockDto], wanted: &str) -> bool {
    blocks.iter().any(|block| {
        block.id == wanted
            || block
                .properties
                .iter()
                .any(|(key, value)| key == "id" && value == wanted)
            || page_holds_block(&block.children, wanted)
    })
}
fn target_in_store(
    store: &Store,
    root: &str,
    request: &LinkRequest,
) -> Result<Option<LinkTarget>, String> {
    // A store still reconciling serves its launch checkpoint (ADR 0070), which
    // lacks a target made while Tine was closed (a block that got its `id::`
    // after the checkpoint was written). A miss asks once more after the
    // launch diff; a ready store answers the same view again in O(1) (GH #181).
    let view = store.whole_graph().map_err(|e| format!("{e:?}"))?;
    match target_in_view(store, &view, root, request)? {
        Some(target) => Ok(Some(target)),
        None => {
            let view = store
                .whole_graph_reconciled()
                .map_err(|e| format!("{e:?}"))?;
            target_in_view(store, &view, root, request)
        }
    }
}

fn target_in_view(
    store: &Store,
    view: &tine_store::WholeGraph,
    root: &str,
    request: &LinkRequest,
) -> Result<Option<LinkTarget>, String> {
    let mut target = LinkTarget {
        root: root.into(),
        display_root: tine_core::graph_paths::normalize_graph_path(root),
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
    let Resolved::Existing { id, others } = view.resolve(&name, kind == PageKind::Journal) else {
        return Ok(None);
    };
    // `view.blocks` names only the logical page. Same-name pages (and duplicate
    // journal days) are separate physical files, and the block belongs to ONE of
    // them: keep that file, not whichever the page resolver prefers.
    let page = match &block {
        None => store.page(&id).map_err(|e| format!("{e:?}"))?,
        Some(wanted) => {
            let mut owner = None;
            for candidate in std::iter::once(&id).chain(others.iter()) {
                let read = store.page(candidate).map_err(|e| format!("{e:?}"))?;
                if page_holds_block(&read.doc.blocks, wanted) {
                    owner = Some(read);
                    break;
                }
            }
            let Some(owner) = owner else {
                return Ok(None);
            };
            owner
        }
    };
    target.name = Some(page.doc.name);
    target.page_kind = Some(kind);
    target.path = Some(page.id.as_str().into());
    target.block = block;
    Ok(Some(target))
}

/// A closed graph's lookup: the read-only prefilter ruled the target out, or a
/// full Store open answered (`None` = not there).
enum ClosedLookup {
    RuledOut,
    Searched(Option<LinkTarget>),
}

fn target_in_closed_graph(
    root: &std::path::Path,
    request: &LinkRequest,
    approved_external_assets: Option<&std::path::Path>,
    use_probe: bool,
    open: impl FnOnce() -> Result<Store, String>,
) -> Result<ClosedLookup, String> {
    let probe = match (&request.block, &request.page) {
        (Some(id), _) => LinkProbe::Block {
            id,
            approved_external_assets,
        },
        (_, Some(name)) => LinkProbe::Page {
            name,
            approved_external_assets,
        },
        _ => LinkProbe::Identity,
    };
    if use_probe && !Store::read_link_identity_at(root, probe).target_possible {
        return Ok(ClosedLookup::RuledOut);
    }
    let store = open()?;
    target_in_store(&store, &root.display().to_string(), request).map(ClosedLookup::Searched)
}

/// One known graph, as the scan sees it before looking for the target.
struct KnownRoot {
    root: std::path::PathBuf,
    identity: Result<Option<String>, String>,
}

/// The candidate list for one link. `lookup(root, use_probe)` answers for one
/// graph (an open graph ignores `use_probe`). GH #181: the prefilter is only an
/// optimisation, so when no graph resolved the target, every graph it ruled out
/// gets a full open before "not found" (reported on Windows, beta.2). A found
/// target keeps the fast path. O(known graphs) lookups, plus one full open per
/// ruled-out graph only on a miss.
fn resolve_link_candidates(
    request: &LinkRequest,
    known: Vec<KnownRoot>,
    mut lookup: impl FnMut(&std::path::Path, bool) -> Result<ClosedLookup, String>,
) -> Result<Vec<LinkTarget>, String> {
    let mut candidates = Vec::new();
    let mut missing_copies = Vec::new();
    let mut errors = Vec::new();
    let mut ruled_out = Vec::new();
    let marker = |root_text: &str, id: String, error: Option<String>| LinkTarget {
        display_root: tine_core::graph_paths::normalize_graph_path(root_text),
        root: root_text.into(),
        graph_id: Some(id),
        name: request.page.clone(),
        page_kind: None,
        path: None,
        block: request.block.clone(),
        error,
    };
    for known in known {
        let KnownRoot { root, identity } = known;
        if let Some(id) = &request.graph {
            match &identity {
                Ok(Some(found)) if found.eq_ignore_ascii_case(id) => {}
                Ok(_) => continue,
                Err(e) => {
                    errors.push(e.clone());
                    continue;
                }
            }
        }
        let root_text = root.display().to_string();
        if request.page.is_none() && request.block.is_none() {
            candidates.push(LinkTarget {
                display_root: tine_core::graph_paths::normalize_graph_path(&root_text),
                root: root_text,
                graph_id: identity.ok().flatten(),
                name: None,
                page_kind: None,
                path: None,
                block: None,
                error: None,
            });
            continue;
        }
        let result = lookup(&root, true);
        let result = match result {
            Ok(ClosedLookup::RuledOut) => {
                ruled_out.push((root.clone(), identity.clone()));
                Ok(None)
            }
            Ok(ClosedLookup::Searched(found)) => Ok(found),
            Err(e) => Err(e),
        };
        match result {
            Ok(Some(mut target)) => {
                target.graph_id = identity.ok().flatten();
                candidates.push(target);
            }
            Ok(None) => {
                if let Ok(Some(id)) = identity {
                    // A remembered copy remains authoritative even if its
                    // target was deleted. Never silently use another copy.
                    let missing = marker(&root_text, id, None);
                    if request.graph.is_some() {
                        candidates.push(missing);
                    } else {
                        missing_copies.push(missing);
                    }
                }
            }
            Err(e) => {
                errors.push(e.clone());
                if let Ok(Some(id)) = identity {
                    let unavailable = marker(&root_text, id, Some(e));
                    if request.graph.is_some() {
                        candidates.push(unavailable);
                    } else {
                        missing_copies.push(unavailable);
                    }
                }
            }
        }
    }
    let resolved =
        |candidates: &[LinkTarget]| candidates.iter().any(|target| target.path.is_some());
    if !ruled_out.is_empty() && !resolved(&candidates) {
        let mut rescued = 0u64;
        for (root, identity) in &ruled_out {
            let root_text = root.display().to_string();
            match lookup(root, false) {
                Ok(ClosedLookup::Searched(Some(mut target))) => {
                    rescued += 1;
                    candidates.retain(|known| known.root != root_text);
                    missing_copies.retain(|known| known.root != root_text);
                    target.graph_id = identity.clone().ok().flatten();
                    candidates.push(target);
                }
                Ok(_) => {}
                Err(e) => errors.push(e),
            }
        }
        crate::flight::link_probe_rescue(ruled_out.len() as u64, rescued);
    }
    let ids: std::collections::HashSet<_> = candidates
        .iter()
        .filter_map(|target| target.graph_id.clone())
        .collect();
    candidates.extend(
        missing_copies
            .into_iter()
            .filter(|target| target.graph_id.as_ref().is_some_and(|id| ids.contains(id))),
    );
    if candidates.is_empty() {
        let detail = errors
            .first()
            .map(|e| format!(" ({e})"))
            .unwrap_or_default();
        return Err(format!("Tine link target not found in any known graph. Open its graph in Tine first, then try the link again.{detail}"));
    }
    Ok(candidates)
}

#[tauri::command]
pub(crate) async fn scan_known_graphs_for_link(
    request: LinkRequest,
    app: tauri::AppHandle,
) -> Result<Vec<LinkTarget>, String> {
    validate(&request)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut roots = std::collections::HashSet::new();
        let mut known = Vec::new();
        for graph in crate::settings::list_known_graphs(app.clone()) {
            let Ok(root) = Store::canonical_root(std::path::Path::new(&graph.path)) else {
                continue;
            };
            if !roots.insert(root.clone()) {
                continue;
            }
            let identity = Store::read_link_identity_at(&root, LinkProbe::Identity).identity;
            known.push(KnownRoot { root, identity });
        }
        resolve_link_candidates(&request, known, |root, use_probe| {
            let root_text = root.display().to_string();
            let owner = state.graphs.read().unwrap().owner(root);
            if let Some(slot) =
                owner.and_then(|label| crate::state::slot_for_window(&state, &label).ok())
            {
                return target_in_store(&slot.store, &root_text, &request)
                    .map(ClosedLookup::Searched);
            }
            let approved = crate::settings::approved_external_assets(&app, root);
            target_in_closed_graph(root, &request, approved.as_deref(), use_probe, || {
                Store::open(
                    root,
                    tine_store::OpenOptions {
                        approved_external_assets: approved.clone(),
                        ..Default::default()
                    },
                )
                .map_err(|e| e.to_string())
                .map(|(store, _, _)| store)
            })
        })
    })
    .await
    .map_err(|e| e.to_string())?
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

    /// The pre-rescue answer of one closed graph: ruled out reads as absent.
    fn closed_lookup(
        root: &std::path::Path,
        request: &LinkRequest,
        approved: Option<&std::path::Path>,
        open: impl FnOnce() -> Result<Store, String>,
    ) -> Result<Option<LinkTarget>, String> {
        target_in_closed_graph(root, request, approved, true, open).map(|lookup| match lookup {
            ClosedLookup::RuledOut => None,
            ClosedLookup::Searched(target) => target,
        })
    }

    #[test]
    fn a_probe_miss_everywhere_falls_back_to_full_opens() {
        // GH #181 (Windows, beta.2): the quick check ruled out the graph that
        // held the block, so the link reported "not found in any known graph".
        let temp = tempfile::tempdir().unwrap();
        let id = "abcdefab-1111-4111-8111-111111111111";
        let roots: Vec<_> = ["holder", "other"]
            .map(|name| temp.path().join(name))
            .into();
        for root in &roots {
            std::fs::create_dir_all(root.join("pages")).unwrap();
        }
        std::fs::write(
            roots[0].join("pages/Note.md"),
            format!("- target\n  id:: {id}\n"),
        )
        .unwrap();
        std::fs::write(roots[1].join("pages/Other.md"), "- unrelated\n").unwrap();
        let request = LinkRequest {
            graph: None,
            page: None,
            block: Some(id.into()),
        };
        let known = || {
            roots
                .iter()
                .map(|root| KnownRoot {
                    root: root.clone(),
                    identity: Ok(None),
                })
                .collect::<Vec<_>>()
        };
        let full_opens = std::cell::Cell::new(0);
        // A prefilter that wrongly rules out every graph.
        let lookup = |root: &std::path::Path, use_probe: bool| {
            if use_probe {
                return Ok(ClosedLookup::RuledOut);
            }
            full_opens.set(full_opens.get() + 1);
            target_in_closed_graph(root, &request, None, false, || {
                Store::open(root, Default::default())
                    .map(|x| x.0)
                    .map_err(|e| e.to_string())
            })
        };
        let found = resolve_link_candidates(&request, known(), lookup).expect("the block is found");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].root, roots[0].display().to_string());
        assert_eq!(found[0].block.as_deref(), Some(id));
        assert_eq!(
            full_opens.get(),
            2,
            "only a miss everywhere opens the ruled-out graphs"
        );

        // A target the quick check finds never opens the graphs it ruled out.
        let opens = std::cell::Cell::new(0);
        let found = resolve_link_candidates(&request, known(), |root, use_probe| {
            opens.set(opens.get() + 1);
            target_in_closed_graph(root, &request, None, use_probe, || {
                Store::open(root, Default::default())
                    .map(|x| x.0)
                    .map_err(|e| e.to_string())
            })
        })
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(opens.get(), 2, "one lookup per graph, no rescue pass");

        // A real miss still reports "not found" after the rescue pass.
        let missing = LinkRequest {
            block: Some("11111111-1111-4111-8111-111111111111".into()),
            ..request.clone()
        };
        let error = resolve_link_candidates(&missing, known(), |root, use_probe| {
            target_in_closed_graph(root, &missing, None, use_probe, || {
                Store::open(root, Default::default())
                    .map(|x| x.0)
                    .map_err(|e| e.to_string())
            })
        })
        .err()
        .unwrap();
        assert!(
            error.starts_with("Tine link target not found in any known graph"),
            "{error}"
        );
    }
    #[test]
    fn closed_graph_without_target_never_opens_a_store() {
        // Copy link persists v4 UUIDs; these must still skip unrelated graphs.
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("pages")).unwrap();
        std::fs::write(temp.path().join("pages/Other.md"), "- unrelated\n").unwrap();
        let opens = std::cell::Cell::new(0);
        let request = LinkRequest {
            graph: None,
            page: None,
            block: Some("11111111-1111-4111-8111-111111111111".into()),
        };
        assert!(closed_lookup(temp.path(), &request, None, || {
            opens.set(opens.get() + 1);
            Store::open(temp.path(), Default::default())
                .map(|x| x.0)
                .map_err(|e| e.to_string())
        })
        .unwrap()
        .is_none());
        assert_eq!(
            opens.get(),
            0,
            "unrelated graphs must not start a graph load"
        );
        let page = LinkRequest {
            graph: Some("11111111-1111-4111-8111-111111111111".into()),
            page: Some("Missing".into()),
            block: None,
        };
        assert!(closed_lookup(temp.path(), &page, None, || {
            panic!("a missing page must not start a graph load")
        })
        .unwrap()
        .is_none());
    }

    #[test]
    fn uncertain_probe_and_substring_hits_keep_the_full_open_authority() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("pages")).unwrap();
        let id = "abcdefab-1111-4111-8111-111111111111";
        let request = LinkRequest {
            graph: None,
            page: None,
            block: Some(id.into()),
        };
        let path = temp.path().join("pages/Other.md");
        std::fs::write(&path, format!("- prose mentions {}\n", id.to_uppercase())).unwrap();
        let opens = std::cell::Cell::new(0);
        let search = || {
            closed_lookup(temp.path(), &request, None, || {
                opens.set(opens.get() + 1);
                Store::open(temp.path(), Default::default())
                    .map(|x| x.0)
                    .map_err(|e| e.to_string())
            })
        };
        assert!(
            search().unwrap().is_none(),
            "a substring alone is not a block identity"
        );
        assert_eq!(opens.get(), 1);
        std::fs::File::create(&path)
            .unwrap()
            .set_len(tine_store::PARSE_INPUT_MAX_BYTES + 1)
            .unwrap();
        assert!(search().unwrap().is_none());
        assert_eq!(
            opens.get(),
            2,
            "uncertain oversized reads fall back to Store"
        );
        std::fs::create_dir_all(temp.path().join("logseq")).unwrap();
        std::fs::write(
            temp.path().join("logseq/config.edn"),
            "{:pages-directory \"../outside\"}",
        )
        .unwrap();
        assert!(search().is_err(), "unsafe layouts keep the full-open error");
        assert_eq!(opens.get(), 3);
    }

    #[test]
    #[ignore = "lane measurement: GH181_FIXTURES names disposable graph fixtures"]
    fn measure_closed_graph_link_search() {
        let roots = std::env::var("GH181_FIXTURES").unwrap();
        let request = LinkRequest {
            graph: None,
            page: None,
            block: Some("11111111-1111-4111-8111-111111111111".into()),
        };
        for root in roots.split(':') {
            let root = std::path::Path::new(root);
            let began = std::time::Instant::now();
            let mut open_elapsed = std::time::Duration::ZERO;
            let mut opens = 0;
            let result = closed_lookup(root, &request, None, || {
                opens += 1;
                let began = std::time::Instant::now();
                let store = Store::open(root, Default::default())
                    .map(|x| x.0)
                    .map_err(|e| e.to_string());
                open_elapsed = began.elapsed();
                store
            })
            .unwrap();
            eprintln!(
                "fixture={} total_ms={:.3} open_return_ms={:.3} opens={} found={}",
                root.file_name().unwrap().to_string_lossy(),
                began.elapsed().as_secs_f64() * 1000.0,
                open_elapsed.as_secs_f64() * 1000.0,
                opens,
                result.is_some()
            );
        }
    }

    #[test]
    fn cold_runtime_block_link_matches_full_open_without_persisted_id() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("pages")).unwrap();
        let path = temp.path().join("pages/Note.md");
        let source = "- target without persisted id\n";
        std::fs::write(&path, source).unwrap();
        let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
        store.whole_graph().unwrap();
        let page = store.page_named("Note", PageKind::Page).unwrap().unwrap();
        let runtime = page.doc.blocks[0].id.clone();
        assert_eq!(runtime, "fdfc8779-86de-8fcd-9a9e-6e878f60e594");
        let request = LinkRequest {
            graph: None,
            page: None,
            block: Some(runtime),
        };
        let display_root = temp.path().display().to_string();
        let full = target_in_store(&store, &display_root, &request).unwrap();
        assert!(full.is_some());
        store.close();
        drop(store);
        let opens = std::cell::Cell::new(0);
        let cold = closed_lookup(temp.path(), &request, None, || {
            opens.set(opens.get() + 1);
            Store::open(temp.path(), Default::default())
                .map(|x| x.0)
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(
            serde_json::to_value(cold).unwrap(),
            serde_json::to_value(full).unwrap()
        );
        assert_eq!(opens.get(), 1);
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }

    #[test]
    fn closed_search_matches_full_open() {
        let temp = tempfile::tempdir().unwrap();
        let id = "abcdefab-1111-4111-8111-111111111111";
        let roots: Vec<_> = ["one", "copy", "other"]
            .map(|name| temp.path().join(name))
            .into();
        for (index, root) in roots.iter().enumerate() {
            std::fs::create_dir_all(root.join("pages/nested")).unwrap();
            std::fs::create_dir_all(root.join("journals")).unwrap();
            std::fs::write(
                root.join("pages/Name.markdown"),
                "title:: Named page\n- body\n",
            )
            .unwrap();
            std::fs::write(root.join("journals/2026_10_14.md"), "- journal\n").unwrap();
            if index < 2 {
                std::fs::write(
                    root.join("pages/nested/Target.ORG"),
                    format!("* linked\n:PROPERTIES:\n:id: {id}\n:END:\n"),
                )
                .unwrap();
            }
        }
        for (block, page, expected) in [
            (Some(id), None, 2),
            (Some("22222222-2222-4222-8222-222222222222"), None, 0),
            (None, Some("Named page"), 3),
            (None, Some("Missing"), 0),
            (None, Some("Oct 14th, 2026"), 3),
        ] {
            let request = LinkRequest {
                graph: page.map(|_| id.into()),
                page: page.map(str::to_owned),
                block: block.map(str::to_owned),
            };
            let mut found = 0;
            for root in &roots {
                let open = || {
                    Store::open(root, Default::default())
                        .map(|x| x.0)
                        .map_err(|e| e.to_string())
                };
                let full = target_in_store(&open().unwrap(), &root.display().to_string(), &request)
                    .unwrap();
                let filtered = closed_lookup(root, &request, None, open).unwrap();
                assert_eq!(
                    serde_json::to_value(&filtered).unwrap(),
                    serde_json::to_value(full).unwrap()
                );
                found += usize::from(filtered.is_some());
            }
            assert_eq!(found, expected, "block={block:?} page={page:?}");
        }
        std::fs::remove_file(roots[1].join("pages/nested/Target.ORG")).unwrap();
        let request = LinkRequest {
            graph: None,
            page: None,
            block: Some(id.into()),
        };
        let count = roots
            .iter()
            .filter(|root| {
                closed_lookup(root, &request, None, || {
                    Store::open(root, Default::default())
                        .map(|x| x.0)
                        .map_err(|e| e.to_string())
                })
                .unwrap()
                .is_some()
            })
            .count();
        assert_eq!(count, 1);
    }
    /// GH #181: a block link opened while Tine was closed, to a block that got
    /// its `id::` after the launch checkpoint was written. The store serves
    /// that checkpoint until the launch diff ends; the lookup must not report
    /// the block missing from it.
    #[test]
    fn a_cold_start_link_finds_a_block_the_launch_checkpoint_lacks() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("pages")).unwrap();
        let cp = tempfile::tempdir().unwrap();
        let cp = cp.path().join("graph.bin");
        let open = || {
            Store::open(
                temp.path(),
                tine_store::OpenOptions {
                    watch: tine_store::WatchMode::Poll,
                    launch_checkpoint: Some(cp.clone()),
                    ..Default::default()
                },
            )
            .unwrap()
            .0
        };
        std::fs::write(temp.path().join("pages/A.md"), "- linked\n").unwrap();
        let store = open();
        store.whole_graph_reconciled().unwrap();
        assert!(matches!(
            store.write_checkpoint_now(),
            Some(tine_store::CheckpointWrite::Written { .. })
        ));
        store.close();
        // While closed: the block gets its id:: (Copy block link elsewhere).
        let id = "22222222-2222-4222-8222-222222222222";
        std::fs::write(
            temp.path().join("pages/A.md"),
            format!("- linked\n  id:: {id}\n"),
        )
        .unwrap();
        let pause = temp.path().join(".tine-test-pause-launch-diff");
        std::fs::write(&pause, b"").unwrap();
        let store = open();
        assert!(
            matches!(store.is_graph_ready(), Ok(false)),
            "the store serves its checkpoint"
        );
        let resume = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            std::fs::remove_file(&pause).unwrap();
        });
        let block = LinkRequest {
            graph: None,
            page: None,
            block: Some(id.into()),
        };
        let target = target_in_store(&store, "fixture", &block).unwrap();
        resume.join().unwrap();
        let target = target.expect("the block made while closed is found");
        assert_eq!(target.name.as_deref(), Some("A"));
        assert_eq!(target.block.as_deref(), Some(id));
    }

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

    #[test]
    fn a_block_link_resolves_to_the_physical_page_that_holds_the_block() {
        // Two files whose names differ only in case are one logical page whose
        // physical members are separate files: the block link must name the
        // file that holds the block, not the page resolver's preferred member.
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("pages")).unwrap();
        let id = "abcdefab-1111-4111-8111-111111111111";
        std::fs::write(temp.path().join("pages/Foo.md"), "- canonical only\n").unwrap();
        std::fs::write(
            temp.path().join("pages/foo.md"),
            format!("- linked\n  id:: {id}\n"),
        )
        .unwrap();
        let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
        let request = LinkRequest {
            graph: None,
            page: None,
            block: Some(id.into()),
        };
        let target = target_in_store(&store, "fixture", &request)
            .unwrap()
            .unwrap();
        assert_eq!(target.path.as_deref(), Some("pages/foo.md"));
        assert_eq!(target.block.as_deref(), Some(id));
        // The upper-case spelling of an authored id is a different block id:
        // lookup is exact, so it must not be silently folded.
        let upper = LinkRequest {
            graph: None,
            page: None,
            block: Some(id.to_uppercase()),
        };
        assert!(target_in_store(&store, "fixture", &upper)
            .unwrap()
            .is_none());
    }

    #[test]
    fn id_shape_matches_the_shared_typescript_fixture() {
        // `src/deepLinks.test.ts` asserts the same file: both languages accept
        // exactly these spellings and neither folds a block id's case.
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/deep-link-ids.json")).unwrap();
        for id in cases["valid"].as_array().unwrap() {
            assert!(valid_id(id.as_str().unwrap()), "{id}");
        }
        for id in cases["invalid"].as_array().unwrap() {
            assert!(!valid_id(id.as_str().unwrap()), "{id}");
        }
    }
}
