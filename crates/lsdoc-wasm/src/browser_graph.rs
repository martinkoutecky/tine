//! Browser access to graph reads. Commands use the desktop wire adapters on
//! the memory graph's indexed publication. Bad inputs and command failures
//! cross wasm-bindgen as errors.

use serde::{Deserialize, Serialize};
use tine_core::model::PageKind;
use tine_core::query::ir::{FriendlyPageMatchScope, ViewSettings};
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::page_read_wire::{self, PageSource};
use tine_graph_features::{graph_read_wire as read, inventory_wire as inventory};
use tine_graph_features::{query_error_wire, query_wire, search};
use tine_store::{Cancel, PageId, WholeGraph};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct BrowserGraph {
    view: WholeGraph,
    root: String,
    query_home: Option<(PageId, query_wire::PublicationQuery)>,
}

#[wasm_bindgen]
impl BrowserGraph {
    /// `files[i]` holds the bytes of the graph-relative `paths[i]`,
    /// `logseq/config.edn` among them. Throws (a string) what the desktop open
    /// refuses, and for arrays of different lengths; a config read problem is
    /// `config_problem_json`. Cost: one read and parse of each file and the
    /// initial index capture, as a desktop open has.
    #[wasm_bindgen(constructor)]
    pub fn new(
        paths: Vec<String>,
        files: Vec<js_sys::Uint8Array>,
        options_json: Option<String>,
    ) -> Result<BrowserGraph, JsValue> {
        let options: OpenOptions = optional_input(options_json.as_deref())?;
        let files = read_sources(paths, files)?;
        if let Some(share) = options.share {
            tine_graph_features::share_admission::admit(
                &share.name,
                &share.exported_at,
                files.iter().map(|(path, bytes)| (path, bytes)),
            )
            .map_err(|error| js_error(format!("share-too-large: {error}")))?;
        }
        let mut graph = open(files)?;
        if let Some(reviewed) = options.query_home {
            let path = graph
                .view
                .config()
                .default_home
                .as_deref()
                .and_then(|name| graph.view.page_owner(name, PageKind::Page))
                .ok_or_else(|| js_error("Missing publication query home"))?;
            graph.query_home = Some((path, reviewed));
        }
        Ok(graph)
    }

    /// The effective metadata of the desktop `load_graph` answer.
    pub fn graph_meta_json(&self) -> Result<String, JsValue> {
        wire(read::graph_meta_wire(self.root.clone(), self.view.config()))
    }

    /// The config read problem of the desktop `load_graph` answer, or null.
    pub fn config_problem_json(&self) -> Result<String, JsValue> {
        wire(tine_graph_features::config_problem::config_problem(
            self.view.config(),
        ))
    }

    pub fn page_inventory_json(&self) -> Result<String, JsValue> {
        wire(inventory::page_inventory_wire(&self.view))
    }

    pub fn page_modified_json(&self) -> Result<String, JsValue> {
        wire(inventory::page_modified_wire(&self.view))
    }

    pub fn page_backlink_counts_json(&self) -> Result<String, JsValue> {
        wire(inventory::page_backlink_counts_wire(&self.view))
    }

    pub fn resolve_page_json(&self, name: &str, journal: bool) -> Result<String, JsValue> {
        wire(inventory::resolve_name(&self.view, name, journal))
    }

    pub fn page_alias_group_json(&self, name: &str) -> Result<String, JsValue> {
        wire(inventory::alias_group_wire(&self.view, name))
    }

    pub fn get_page_json(&self, name: &str, journal: bool) -> Result<String, JsValue> {
        let kind = if journal {
            PageKind::Journal
        } else {
            PageKind::Page
        };
        wire(
            page_read_wire::get_page(PageSource::View(&self.view), name, kind)
                .map_err(|error| js_error(format!("{error:?}")))?,
        )
    }

    pub fn get_page_by_path_json(&self, path: &str) -> Result<String, JsValue> {
        wire(
            page_read_wire::get_page_by_path(PageSource::View(&self.view), PageId::from(path))
                .map_err(|error| js_error(format!("{error:?}")))?,
        )
    }

    pub fn journal_feed_page_json(
        &self,
        limit: usize,
        before_day: Option<i64>,
        wait_for_references: bool,
        through_day: Option<i64>,
    ) -> Result<String, JsValue> {
        wire(
            page_read_wire::journal_feed_page(
                PageSource::View(&self.view),
                limit,
                before_day,
                through_day,
                wait_for_references,
            )
            .map_err(js_error)?,
        )
    }

    pub fn journal_content_days_json(&self) -> Result<String, JsValue> {
        wire(read::journal_content_days(&self.view))
    }

    pub fn page_icons_json(&self, names_json: &str) -> Result<String, JsValue> {
        let names: Vec<String> = input(names_json)?;
        wire(read::page_icons(&self.view, &names))
    }

    pub fn get_backlinks_json(&self, name: &str) -> Result<String, JsValue> {
        wire(read::backlinks(&self.view, name).map_err(reference_error)?)
    }

    pub fn get_backlink_filter_context_json(
        &self,
        name: &str,
        targets_json: &str,
    ) -> Result<String, JsValue> {
        let targets: Vec<_> = input(targets_json)?;
        wire(read::backlink_filter_context(&self.view, name, &targets).map_err(js_error)?)
    }

    pub fn get_unlinked_refs_json(&self, name: &str) -> Result<String, JsValue> {
        wire(read::unlinked_refs(&self.view, name).map_err(reference_error)?)
    }

    pub fn get_block_ref_counts_json(&self) -> Result<String, JsValue> {
        wire(read::block_ref_counts(&self.view))
    }

    pub fn get_block_referrers_json(&self, uuid: &str) -> Result<String, JsValue> {
        wire(read::block_referrers(&self.view, uuid).map_err(js_error)?)
    }

    pub fn resolve_blocks_json(&self, uuids_json: &str) -> Result<String, JsValue> {
        let uuids: Vec<String> = input(uuids_json)?;
        wire(read::resolve_blocks(&self.view, &uuids).map_err(js_error)?)
    }

    pub fn preview_block_json(&self, uuid: &str, max_nodes: usize) -> Result<String, JsValue> {
        wire(read::preview_block(&self.view, uuid, max_nodes).map_err(js_error)?)
    }

    /// Literal block search. Synchronous browser calls share no search lane.
    pub fn search_json(&self, query: &str, limit: usize) -> Result<String, JsValue> {
        let answer = search::find_blocks_sequenced(
            &self.view,
            &search::SearchLanes::default(),
            query,
            limit,
            None,
            None,
        )
        .map_err(search_error)?;
        wire(answer)
    }

    pub fn capture_quick_switch_json(
        &self,
        query: &str,
        limit: usize,
        transliteration: bool,
    ) -> Result<String, JsValue> {
        wire(inventory::capture_quick_switch(
            &self.view,
            query,
            limit,
            transliteration,
        ))
    }

    /// `scope_json` is `{"keys":true}` or `{"key":"property name"}`.
    pub fn query_facets_json(&self, scope_json: &str) -> Result<String, JsValue> {
        let scope: read::AutocompleteFacetScope = input(scope_json)?;
        wire(read::query_facets(
            &self.view,
            scope.into_policy().map_err(js_error)?,
        ))
    }

    /// `dialect` uses the desktop wire names. Properties are a JSON array of pairs.
    pub fn parse_query_json(
        &self,
        text: &str,
        dialect: &str,
        block_properties_json: Option<String>,
    ) -> Result<String, JsValue> {
        use serde::de::value::{Error, StrDeserializer};
        let dialect = QueryTextDialect::deserialize(StrDeserializer::<Error>::new(dialect))
            .map_err(js_error)?;
        let properties: Vec<(String, String)> = optional_input(block_properties_json.as_deref())?;
        wire(query_wire::parse(&self.view, text, dialect, &properties).map_err(js_error)?)
    }

    /// Query, view and optional execution context use the desktop JSON shapes.
    pub fn query_run_json(
        &self,
        query_json: &str,
        view_json: &str,
        context_json: Option<String>,
    ) -> Result<String, JsValue> {
        let query = input(query_json)?;
        let view = input(view_json)?;
        let context: QueryContext = optional_input(context_json.as_deref())?;
        let result = match &self.query_home {
            Some((path, reviewed))
                if context.source_page.is_some()
                    && context.source_page == self.view.config().default_home =>
            {
                query_wire::run_publication(&self.view, &query, &view, path.as_str(), reviewed)
            }
            _ => query_wire::run_wire(
                &self.view,
                &query,
                &view,
                &context.execution,
                &Cancel::default(),
            ),
        };
        wire(result.map_err(js_error)?)
    }

    /// Options are the desktop search fields `explain`, `scope`,
    /// `pageMatchScope`, `pageView`, `blockView`, and `transliteration`.
    pub fn run_graph_search_json(
        &self,
        source: String,
        page_limit: usize,
        block_limit: usize,
        options_json: Option<String>,
    ) -> Result<String, JsValue> {
        let options: SearchOptions = optional_input(options_json.as_deref())?;
        let answer = search::run_graph_search_sequenced(
            &self.view,
            &search::SearchLanes::default(),
            source,
            page_limit,
            block_limit,
            None,
            None,
            options.explain,
            options.scope,
            options.page_match_scope,
            options.page_view,
            options.block_view,
            options.transliteration.unwrap_or(false),
        )
        .map_err(search_error)?;
        wire(answer)
    }
}

#[derive(Default, Deserialize)]
struct OpenOptions {
    share: Option<ShareMeta>,
    query_home: Option<query_wire::PublicationQuery>,
}

#[derive(Deserialize)]
struct ShareMeta {
    name: String,
    exported_at: String,
}

#[derive(Default, Deserialize)]
struct QueryContext {
    #[serde(flatten)]
    execution: tine_core::query::ir::ExecutionContext,
    source_page: Option<String>,
}

fn read_sources(
    paths: Vec<String>,
    files: Vec<js_sys::Uint8Array>,
) -> Result<Vec<(String, Vec<u8>)>, JsValue> {
    if paths.len() != files.len() {
        return Err(js_error(format!(
            "BrowserGraph: {} paths, {} files",
            paths.len(),
            files.len()
        )));
    }
    Ok(paths
        .into_iter()
        .zip(files)
        .map(|(path, bytes)| (path, bytes.to_vec()))
        .collect())
}

fn open(files: Vec<(String, Vec<u8>)>) -> Result<BrowserGraph, JsValue> {
    let (view, root) = WholeGraph::from_files(files)
        .map_err(|error| js_error(format!("BrowserGraph: {error}")))?;
    Ok(BrowserGraph {
        view,
        root,
        query_home: None,
    })
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SearchOptions {
    explain: bool,
    scope: Option<search::Scope>,
    page_match_scope: Option<FriendlyPageMatchScope>,
    page_view: Option<ViewSettings>,
    block_view: Option<ViewSettings>,
    transliteration: Option<bool>,
}

fn reference_error(error: tine_store::QueryError) -> JsValue {
    js_error(query_error_wire::reference_error(error, |_| {}))
}

fn search_error(error: tine_store::QueryError) -> JsValue {
    js_error(query_error_wire::query_error(error))
}

fn input<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, JsValue> {
    serde_json::from_str(json).map_err(js_error)
}

/// An optional desktop parameter: absent or JSON `null` is its default, as a
/// Tauri `Option` parameter reads it.
fn optional_input<T: serde::de::DeserializeOwned + Default>(
    json: Option<&str>,
) -> Result<T, JsValue> {
    let given: Option<T> = json.map_or(Ok(None), input)?;
    Ok(given.unwrap_or_default())
}

fn wire(answer: impl Serialize) -> Result<String, JsValue> {
    serde_json::to_string(&answer).map_err(js_error)
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
