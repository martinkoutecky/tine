//! Browser access to graph reads. Commands use the desktop wire adapters on
//! the memory graph's indexed publication. Bad inputs and command failures
//! cross wasm-bindgen as errors.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tine_core::model::PageKind;
use tine_core::query::ir::{ExecutionContext, FriendlyPageMatchScope, ViewSettings};
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::graph_change_wire::GraphChange;
use tine_graph_features::page_read_wire::{self, PageSource};
use tine_graph_features::{graph_read_wire as read, inventory_wire as inventory};
use tine_graph_features::{query_error_wire, query_wire, search};
use tine_store::{Cancel, Change, PageId, WholeGraph};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct BrowserGraph {
    view: WholeGraph,
    root: String,
    /// Each key's version past the first (1): every write of the key moves
    /// it, so a submit typed on an earlier one is stale.
    versions: HashMap<String, u64>,
}

#[wasm_bindgen]
impl BrowserGraph {
    /// `files[i]` holds the bytes of the graph-relative `paths[i]`,
    /// `logseq/config.edn` among them. Options: `share` (a quick share's
    /// admission), `times`, a modification time in Unix seconds per path
    /// (absent: no time), which `page_modified_json` and recency answer from,
    /// and `editable` (default false). This constructor alone decides whether
    /// the graph takes edits: a share never does, whatever `editable` says.
    /// Only an editable graph keeps its files for `replace_files` and
    /// `submit_page_json`; a read-only one (a published export, a quick
    /// share) keeps its read view alone, and `replace_files` throws and
    /// `submit_page_json` refuses (`failed`) there.
    /// Throws (a string) what the desktop open refuses, for arrays of
    /// different lengths and for a time of no given path (whatever the time;
    /// a pre-epoch time is then no recency row, as on disk); a config read
    /// problem is `config_problem_json`. Cost: one read and parse of each
    /// file and the initial index capture, as a desktop open has.
    #[wasm_bindgen(constructor)]
    pub fn new(
        paths: Vec<String>,
        files: Vec<js_sys::Uint8Array>,
        options_json: Option<String>,
    ) -> Result<BrowserGraph, JsValue> {
        let options: OpenOptions = optional_input(options_json.as_deref())?;
        let files = read_sources(paths, files)?;
        if let Some(share) = &options.share {
            tine_graph_features::share_admission::check_sizes(
                &share.name,
                &share.exported_at,
                files.iter().map(|(path, bytes)| (path.len(), bytes.len())),
            )
            .map_err(|error| js_error(format!("share-too-large: {error}")))?;
        }
        let editable = options.editable && options.share.is_none();
        let graph = open(files, options.times, editable)?;
        if options.share.is_some() {
            if !graph.view.unreadable_files().is_empty() {
                return Err(js_error(
                    "share-too-large: a shared source could not be read as an outline",
                ));
            }
            let corpus = graph.view.corpus(None, false);
            tine_graph_features::share_admission::check_blocks(
                corpus.pages.iter().map(|p| p.document.as_ref()),
            )
            .map_err(|error| js_error(format!("share-too-large: {error}")))?;
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
        let context: ExecutionContext = optional_input(context_json.as_deref())?;
        let result = query_wire::run_wire(&self.view, &query, &view, &context, &Cancel::default());
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

    /// Replace files: `files[i]` (bytes, or null to delete) at the
    /// graph-relative `paths[i]`, under the constructor's path rules, then
    /// recapture the changed pages (`WholeGraph::replace_files`). Answers
    /// `{answers, pages}`: the publication's answer signal (the `answers`
    /// the desktop's publication events carry, the new `rev` among it) and
    /// the desktop `graph-changed` payloads of the pages that moved. Throws (a
    /// string) a refused change, which leaves the graph unchanged. Cost: one
    /// parse per changed page and the incremental capture of those paths.
    pub fn replace_files(
        &mut self,
        paths: Vec<String>,
        files: Vec<JsValue>,
    ) -> Result<String, JsValue> {
        if paths.len() != files.len() {
            return Err(js_error(format!(
                "BrowserGraph: {} paths, {} files",
                paths.len(),
                files.len()
            )));
        }
        let changes = (paths.into_iter().zip(files))
            .map(|(path, file)| {
                // Only `null` deletes: `undefined` (and a sparse slot) is no
                // file's bytes, so the byte check below refuses it.
                if file.is_null() {
                    return Ok((path, None));
                }
                let bytes = file.dyn_into::<js_sys::Uint8Array>().map_err(|_| {
                    js_error(format!("BrowserGraph: {path:?} is not bytes or null"))
                })?;
                Ok((path, Some(bytes.to_vec())))
            })
            .collect::<Result<Vec<_>, JsValue>>()?;
        let change = self.view.replace_files(changes).map_err(js_error)?;
        wire(self.published(&change))
    }

    /// Submit a page DTO (the desktop `PageDto` JSON) for the page file
    /// `key`, typed on `version` (a key starts at 1; each write moves it).
    /// Serializes over the file's current text with the store's one page
    /// serializer (the desktop page host's bytes), then replaces the file
    /// (`WholeGraph::submit_page`). Answers `{"written": {path, text,
    /// version, answers, pages}}` (the key's new version, and the payloads
    /// `replace_files` answers), or `{"refused": PageRefusal}` (the desktop
    /// host's refusal shape; a stale version is `not-admitted`). Writes
    /// nothing on a refusal.
    pub fn submit_page_json(
        &mut self,
        key: &str,
        dto_json: &str,
        version: u64,
    ) -> Result<String, JsValue> {
        let dto: tine_core::model::PageDto = input(dto_json)?;
        let current = version == self.version(key);
        let (bytes, change) = match self.view.submit_page(&PageId::from(key), &dto, current) {
            Ok(written) => written,
            Err(refusal) => return wire(serde_json::json!({ "refused": refusal })),
        };
        let published = self.published(&change);
        wire(serde_json::json!({
            "written": {
                "path": key,
                "text": String::from_utf8(bytes).map_err(js_error)?,
                "version": self.version(key),
                "answers": published.answers,
                "pages": published.pages,
            }
        }))
    }

    /// The desktop `list_template_names` answer.
    pub fn list_template_names_json(&self) -> Result<String, JsValue> {
        wire(self.view.template_names())
    }

    /// The desktop `list_templates` answer: every template, or the first
    /// named `name`.
    pub fn list_templates_json(&self, name: Option<String>) -> Result<String, JsValue> {
        wire(tine_graph_features::templates::view_templates(
            &self.view,
            name.as_deref(),
        ))
    }

    /// The desktop `export_query_subtrees` answer for its `specs` JSON.
    pub fn export_query_subtrees_json(&self, specs_json: &str) -> Result<String, JsValue> {
        let specs: Vec<tine_core::query::QueryExportSpec> = input(specs_json)?;
        wire(
            self.view
                .export_query_subtrees(&specs)
                .map_err(search_error)?,
        )
    }

    /// The desktop `tagged_pages` answer.
    pub fn tagged_pages_json(&self, name: &str) -> Result<String, JsValue> {
        let answer = query_wire::tagged_pages(&self.view, name, &Cancel::default());
        wire(answer.map_err(js_error)?)
    }

    /// The desktop `query_explain_empty` answer; the view is accepted for
    /// its command shape and ignored, as there.
    pub fn query_explain_empty_json(
        &self,
        query_json: &str,
        view_json: &str,
        context_json: Option<String>,
    ) -> Result<String, JsValue> {
        let query = input(query_json)?;
        let _: ViewSettings = input(view_json)?;
        let context: ExecutionContext = optional_input(context_json.as_deref())?;
        let answer = query_wire::explain_empty(&self.view, &query, &context, &Cancel::default());
        wire(answer.map_err(js_error)?)
    }
}

impl BrowserGraph {
    fn version(&self, key: &str) -> u64 {
        1 + self.versions.get(key).copied().unwrap_or(0)
    }

    /// A publication's window payloads; moves the version of every file it
    /// changed.
    fn published<'a>(&mut self, change: &'a Change) -> Published<'a> {
        for (id, _, _) in &change.files {
            *self.versions.entry(id.as_str().to_owned()).or_default() += 1;
        }
        let pages = (change.pages())
            .map(|(id, kind, page, name)| GraphChange::of(id, kind, page, name))
            .collect();
        Published {
            answers: change,
            pages,
        }
    }
}

#[derive(Serialize)]
struct Published<'a> {
    answers: &'a Change,
    pages: Vec<GraphChange>,
}

#[derive(Default, Deserialize)]
struct OpenOptions {
    share: Option<ShareMeta>,
    /// Modification time per path, in Unix seconds.
    times: Option<HashMap<String, i64>>,
    /// Whether the graph takes edits.
    #[serde(default)]
    editable: bool,
}

#[derive(Deserialize)]
struct ShareMeta {
    name: String,
    exported_at: String,
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

fn open(
    files: Vec<(String, Vec<u8>)>,
    times: Option<HashMap<String, i64>>,
    editable: bool,
) -> Result<BrowserGraph, JsValue> {
    let times = (times.unwrap_or_default().into_iter())
        .map(|(path, secs)| (path, unix_time(secs)))
        .collect();
    let (view, root) = WholeGraph::from_files(files, times, editable)
        .map_err(|error| js_error(format!("BrowserGraph: {error}")))?;
    Ok(BrowserGraph {
        view,
        root,
        versions: HashMap::new(),
    })
}

/// The time `secs` Unix seconds name, signed: a pre-epoch time stays before
/// the epoch, so the shared answers treat it as they treat a disk file's
/// (`page_modified` omits it). `None` where the target's clock cannot hold
/// it (wasm32 holds no pre-epoch time): `WholeGraph::from_files` admits it
/// as any given time, and the file then has no time, which `page_modified`
/// answers alike.
fn unix_time(secs: i64) -> Option<std::time::SystemTime> {
    let magnitude = std::time::Duration::from_secs(secs.unsigned_abs());
    if secs < 0 {
        std::time::UNIX_EPOCH.checked_sub(magnitude)
    } else {
        std::time::UNIX_EPOCH.checked_add(magnitude)
    }
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
