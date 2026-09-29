//! Sheets (`tine.view` tables, boards and grids) in the static export.
//!
//! The app computes every sheet with its own evaluator (`src/sheet/*`: config,
//! filter, formulas, grouping, aggregates) and hands the publisher a pure DATA
//! fragment per sheet block (`SheetExport`). This module only LAYS THAT DATA OUT
//! and escapes every string of it: it never derives a row set, a group, a formula
//! value or an aggregate, so there is exactly one answer to what a sheet shows
//! (I-12, `tests/i12_single_definitions.rs`).
//!
//! An export with no frontend (the desktop CLI, `publish_static`) has no
//! `SheetIndex` and keeps the plain outline: a named divergence
//! (`Guide > Exporting`, `sheets_export_without_frontend_keeps_the_outline`).
//! A sheet the app could not compute, or whose subtree changed since the app
//! computed it, renders as the plain outline plus a visible note, never a failed
//! export (I-9).
//!
//! Unit cost: none persisted. Per sheet one subtree fingerprint (O(subtree
//! bytes)) at input time and again at render time; per export at most
//! `MAX_TOTAL_CELLS` cells of data are accepted (I-22).

use super::{
    ast_plain_text, body_blocks, decorate, esc, esc_attr, has_class, md_opts, publish_page_allowed,
    render_block, render_facets, tag_attr, unescape, Ctx, PageAnchors, PrintOpts, RenderGraph,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use tine_core::doc::DocBlock;
use tine_core::lsdoc::ast::Block;
use tine_core::model::BlockDto;
use tine_core::query::ir::{ViewKind, ViewSettings};
use tine_core::query::macro_text::{is_query_macro_name, query_macro_extent};
use tine_core::query::wire_parse::QueryTextDialect;
use tine_core::Corpus;

/// Sheets accepted per export.
const MAX_SHEETS: usize = 2_000;
/// Data cells (rows x columns, cards, grid cells) accepted for one sheet.
const MAX_SHEET_CELLS: usize = 100_000;
/// Data cells accepted across one export.
const MAX_TOTAL_CELLS: usize = 1_000_000;
/// Children of a sheet block sent to the app; the rest are counted in `omitted`.
const MAX_INPUT_ROWS: usize = 5_000;
/// Children of a sheet row sent to the app (the grid width bound).
const MAX_INPUT_COLS: usize = 256;

/// Query-backed sheets computed per phase (inputs, render). Each is one
/// whole-graph query, so the count is bounded (I-22); a block past the bound
/// keeps the flat result list.
const MAX_QUERY_SHEETS: usize = 64;

/// One candidate sheet block, as sent to the app (`SheetInput` in
/// `src/sheet/staticExport.ts`).
#[derive(Debug, Clone, Serialize)]
pub struct SheetInput {
    pub page: String,
    pub path: Vec<u32>,
    pub fp: String,
    pub owner: BlockDto,
    pub rows: Vec<BlockDto>,
    pub omitted: usize,
    /// Present when the block's whole body is one `{{query}}` macro: its result
    /// rows, for the app to present as the table or board the block asks for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<QueryRowsInput>,
}

/// The rows a query-backed sheet presents (`QuerySource` in
/// `src/sheet/staticExport.ts`): what the live query block hands its sheet.
#[derive(Debug, Clone, Serialize)]
pub struct QueryRowsInput {
    /// Echoed back so a changed result is refused; covers the query and its rows.
    pub fp: String,
    /// The query's own block presentation (`as table`), which beats `tine.view`.
    pub presentation: Option<ViewKind>,
    /// The parsed query's display settings (columns, sort), as the live sheet reads them.
    pub view: ViewSettings,
    /// The page each row belongs to, parallel to `rows`.
    pub pages: Vec<String>,
    pub rows: Vec<BlockDto>,
}

/// How one cell value is shown (`CellView` in `src/sheet/cellPresentation.ts`).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "k", rename_all = "lowercase")]
enum Cell {
    None,
    Marker { raw: String, text: String },
    Priority { raw: String, text: String },
    Date { cls: String, text: String },
    Chips { values: Vec<String> },
    Check { checked: bool },
    Inline { text: String },
    Error { message: String },
    Plain { text: String },
}

#[derive(Debug, Clone, Deserialize)]
struct Agg {
    label: String,
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Column {
    label: String,
    formula: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct TableRow {
    ix: usize,
    title: String,
    bg: Option<String>,
    cells: Vec<Cell>,
}

#[derive(Debug, Clone, Deserialize)]
struct Chips {
    priority: String,
    scheduled: String,
    deadline: String,
    tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Card {
    ix: usize,
    title: String,
    bg: Option<String>,
    chips: Chips,
}

#[derive(Debug, Clone, Deserialize)]
struct BoardColumn {
    label: String,
    cards: Vec<Card>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "view", rename_all = "lowercase")]
enum Body {
    Table {
        columns: Vec<Column>,
        rows: Vec<TableRow>,
        footer: Option<Vec<Option<Agg>>>,
        #[serde(rename = "filterError")]
        filter_error: Option<String>,
        omitted: usize,
    },
    Board {
        columns: Vec<BoardColumn>,
        #[serde(rename = "filterError")]
        filter_error: Option<String>,
        omitted: usize,
    },
    Grid {
        cols: usize,
        header: bool,
        footer: Option<Vec<Option<Agg>>>,
        omitted: usize,
    },
    Error {
        message: String,
    },
}

/// One computed sheet (`SheetExport` in `src/sheet/staticExport.ts`).
#[derive(Debug, Clone, Deserialize)]
pub struct SheetExport {
    page: String,
    path: Vec<u32>,
    fp: String,
    /// The answer was computed over a query's result rows, not the block's children.
    #[serde(default)]
    query: bool,
    #[serde(flatten)]
    body: Body,
}

impl Body {
    /// Data cells this body asks the renderer to lay out.
    fn cells(&self) -> usize {
        match self {
            Body::Table { columns, rows, .. } => rows.len().saturating_mul(columns.len().max(1)),
            Body::Board { columns, .. } => columns.iter().map(|c| c.cards.len()).sum(),
            Body::Grid { cols, .. } => *cols,
            Body::Error { .. } => 0,
        }
    }
}

/// The app's sheet answers for one export, keyed by (page name, block path).
#[derive(Debug, Default)]
pub struct SheetIndex(HashMap<(String, Vec<u32>), SheetExport>);

impl SheetIndex {
    /// Admit the app's answers within the export budget; an over-budget or
    /// duplicate sheet is dropped (its block keeps the plain outline).
    pub fn new(exports: Vec<SheetExport>) -> Self {
        let mut total = 0usize;
        let mut map = HashMap::new();
        for export in exports {
            let cells = export.body.cells();
            if map.len() >= MAX_SHEETS
                || cells > MAX_SHEET_CELLS
                || total.saturating_add(cells) > MAX_TOTAL_CELLS
            {
                tine_core::diag_line::diagnostic_line(
                    "tine export: sheet over budget, outline kept",
                );
                continue;
            }
            total += cells;
            map.entry((export.page.clone(), export.path.clone()))
                .or_insert(export);
        }
        Self(map)
    }
}

/// Child-index path of a block from its page root, as a borrowed chain (no
/// allocation per block).
#[derive(Clone, Copy)]
pub(super) struct SheetPath<'a> {
    parent: Option<&'a SheetPath<'a>>,
    ix: u32,
}

impl<'a> SheetPath<'a> {
    pub(super) fn root(ix: usize) -> Self {
        Self {
            parent: None,
            ix: ix as u32,
        }
    }
    pub(super) fn child(&'a self, ix: usize) -> SheetPath<'a> {
        Self {
            parent: Some(self),
            ix: ix as u32,
        }
    }
    fn to_vec(self) -> Vec<u32> {
        let mut out = vec![self.ix];
        let mut cursor = self.parent;
        while let Some(node) = cursor {
            out.push(node.ix);
            cursor = node.parent;
        }
        out.reverse();
        out
    }
}

/// FNV-1a over a block subtree (raw text plus shape), so the app's answer is
/// refused if the tree changed after the app computed it.
pub(super) fn fingerprint(root: &DocBlock) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let mut stack = vec![root];
    while let Some(block) = stack.pop() {
        feed(block.raw().as_bytes());
        feed(&[0x1f]);
        feed(&(block.children.len() as u32).to_le_bytes());
        stack.extend(block.children.iter().rev());
    }
    format!("{hash:016x}")
}

/// A block is a sheet CANDIDATE when it carries a `tine.view` property; whether
/// the value is a sheet view is the app's decision (`sheetConfig`).
fn is_candidate(block: &DocBlock) -> bool {
    block
        .properties()
        .iter()
        .any(|(key, _)| key.trim().eq_ignore_ascii_case("tine.view"))
}

/// Every candidate sheet block of the named pages (all pages when `pages` is
/// `None`), with the data the app needs to compute it. Bounded by `MAX_SHEETS`,
/// `MAX_INPUT_ROWS` and `MAX_INPUT_COLS`; cost O(blocks of the pages).
/// `published` holds the `page_key` of every page the export will publish
/// (`None` = no publication boundary, as in print): a query sheet's row on any
/// other page is left out of the app's input.
pub fn sheet_inputs(
    corpus: &Corpus,
    pages: Option<&[String]>,
    graph: Option<&RenderGraph<'_>>,
    published: Option<&HashSet<String>>,
) -> Vec<SheetInput> {
    let mut query_budget = MAX_QUERY_SHEETS;
    let wanted: Option<HashSet<&str>> = pages.map(|p| p.iter().map(String::as_str).collect());
    let mut out = Vec::new();
    for page in &corpus.pages {
        if wanted
            .as_ref()
            .is_some_and(|w| !w.contains(page.name.as_str()))
        {
            continue;
        }
        let mut stack: Vec<(&DocBlock, Vec<u32>)> = page
            .document
            .roots
            .iter()
            .enumerate()
            .rev()
            .map(|(i, b)| (b, vec![i as u32]))
            .collect();
        while let Some((block, path)) = stack.pop() {
            if out.len() >= MAX_SHEETS {
                return out;
            }
            if is_candidate(block) {
                let shallow = |b: &DocBlock| tine_core::projection::block_to_shallow_dto(b);
                let rows = block
                    .children
                    .iter()
                    .take(MAX_INPUT_ROWS)
                    .map(|row| {
                        let mut dto = shallow(row);
                        dto.children = row
                            .children
                            .iter()
                            .take(MAX_INPUT_COLS)
                            .map(shallow)
                            .collect();
                        dto
                    })
                    .collect();
                let query = graph.filter(|_| query_budget > 0).and_then(|graph| {
                    let found = sole_query_macro(block)?;
                    query_budget -= 1;
                    query_rows(graph, block, &found, &|name| {
                        published.is_none_or(|keys| keys.contains(&tine_core::refs::page_key(name)))
                    })
                });
                out.push(SheetInput {
                    page: page.name.clone(),
                    path: path.clone(),
                    fp: fingerprint(block),
                    owner: shallow(block),
                    rows,
                    omitted: block.children.len().saturating_sub(MAX_INPUT_ROWS),
                    query,
                });
            }
            for (i, child) in block.children.iter().enumerate().rev() {
                let mut child_path = path.clone();
                child_path.push(i as u32);
                stack.push((child, child_path));
            }
        }
    }
    out
}

/// The name and RAW argument of the query macro that is a sheet block's whole
/// body (`{{query …}}` or `{{tine-query …}}`), else `None`. The argument comes
/// from the block source, not from lsdoc's comma-split arguments, so an options
/// map or a literal comma survives (master §4.3.1).
pub(super) fn sole_query_macro(block: &DocBlock) -> Option<(String, String)> {
    if !is_candidate(block) {
        return None;
    }
    let blocks: Vec<Block> = body_blocks(block.raw());
    let html = tine_core::lsdoc::render_html(&blocks, &md_opts());
    let inner = html.trim().strip_prefix("<span ")?;
    let close = inner.find('>')?;
    let tag = &inner[..close];
    if !has_class(tag, "macro") || inner[close + 1..].trim() != "</span>" {
        return None;
    }
    let name = tag_attr(tag, "data-macro").map(unescape)?;
    is_query_macro_name(&name).then_some(())?;
    Some((name, query_macro_extent(block.raw())?.argument))
}

/// FNV-1a of a query sheet's identity: the owner's subtree, the macro and each
/// result row (page, raw text). Recomputed at render time from a fresh run.
fn query_fingerprint(
    owner: &DocBlock,
    found: &(String, String),
    pages: &[String],
    rows: &[BlockDto],
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0x1f;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    feed(fingerprint(owner).as_bytes());
    feed(found.0.as_bytes());
    feed(found.1.as_bytes());
    for (page, row) in pages.iter().zip(rows) {
        feed(page.as_bytes());
        // Not the row id: it is a per-load runtime id, and the input and the
        // render may each open the graph.
        feed(row.raw.as_bytes());
    }
    format!("{hash:016x}")
}

/// Run a sheet block's query the way the live block does and flatten the result
/// rows. `None` (the flat result list stays) when the query is refused, exceeds
/// its bounds, answers pages, matches nothing (after rows on unpublished pages
/// are dropped), or has more rows than one sheet takes.
fn query_rows(
    graph: &RenderGraph<'_>,
    owner: &DocBlock,
    found: &(String, String),
    published: &dyn Fn(&str) -> bool,
) -> Option<QueryRowsInput> {
    let (name, argument) = found;
    if !tine_core::query::query_source_within_limit(argument)
        || !tine_core::query::query_nesting_within_limit(argument)
    {
        return None;
    }
    let dialect = if name.eq_ignore_ascii_case("tine-query") {
        QueryTextDialect::MacroTql
    } else {
        QueryTextDialect::MacroQuery
    };
    let host = owner.properties();
    let (parsed, bounded) = graph.query_parsed(argument, dialect, &host)?;
    if bounded.exceeded || !bounded.pages.is_empty() {
        return None;
    }
    let mut pages = Vec::new();
    let mut rows = Vec::new();
    for group in bounded.groups {
        // A row on a page this export does not publish is left out here, before
        // the app computes cells, counts and aggregates from it.
        if !published(&group.page) {
            continue;
        }
        for block in group.blocks {
            pages.push(group.page.clone());
            rows.push(block);
        }
    }
    if rows.is_empty() || rows.len() > MAX_INPUT_ROWS {
        return None;
    }
    Some(QueryRowsInput {
        fp: query_fingerprint(owner, found, &pages, &rows),
        presentation: parsed.scoped.block_presentation,
        view: parsed.view,
        pages,
        rows,
    })
}

/// A CSS color the export will place in a `style` attribute: a hex, an
/// `rgb()/rgba()` list or a plain keyword; anything else is dropped (I-22).
fn safe_color(value: &str) -> Option<&str> {
    let v = value.trim();
    let hex = v.strip_prefix('#').is_some_and(|h| {
        matches!(h.len(), 3 | 4 | 6 | 8) && h.bytes().all(|b| b.is_ascii_hexdigit())
    });
    let func = (v.starts_with("rgb(") || v.starts_with("rgba("))
        && v.ends_with(')')
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" (),.%".contains(&b));
    let word = !v.is_empty() && v.len() <= 20 && v.bytes().all(|b| b.is_ascii_alphabetic());
    (hex || func || word).then_some(v)
}

fn style_attr(bg: &Option<String>) -> String {
    bg.as_deref()
        .and_then(safe_color)
        .map(|c| format!(" style=\"background:{}\"", esc_attr(c)))
        .unwrap_or_default()
}

fn inline(text: &str, ctx: &Ctx) -> String {
    decorate(
        &tine_core::lsdoc::render_html(&body_blocks(text), &md_opts()),
        ctx,
        0,
    )
}

fn alpha(s: &str) -> bool {
    !s.is_empty() && s.len() <= 16 && s.bytes().all(|b| b.is_ascii_alphabetic())
}

fn cell_html(cell: &Cell, ctx: &Ctx) -> String {
    match cell {
        Cell::None => String::new(),
        Cell::Marker { raw, .. } if alpha(raw) => {
            let mut out = String::new();
            render_facets::emit_header_facets(Some(raw), None, &mut out);
            out
        }
        Cell::Priority { raw, .. } if matches!(raw.as_str(), "A" | "B" | "C") => {
            let mut out = String::new();
            render_facets::emit_header_facets(None, Some(raw), &mut out);
            out
        }
        Cell::Marker { text, .. } | Cell::Priority { text, .. } | Cell::Plain { text } => esc(text),
        Cell::Date { cls, text } => format!(
            "<span class=\"sheet-date {}\">{}</span>",
            if cls == "deadline" {
                "deadline"
            } else {
                "scheduled"
            },
            esc(text)
        ),
        // The app shows tag and enum chips as inert chips, so the export does too.
        Cell::Chips { values } => values
            .iter()
            .map(|v| format!("<span class=\"sheet-chip\">{}</span>", esc(v)))
            .collect::<Vec<_>>()
            .join(" "),
        Cell::Check { checked } => format!(
            "<span class=\"task-checkbox{}\"></span>",
            if *checked { " checked" } else { "" }
        ),
        Cell::Inline { text } => inline(text, ctx),
        Cell::Error { message } => format!(
            "<span class=\"sheet-formula-error\" title=\"{}\">⚠</span>",
            esc_attr(message)
        ),
    }
}

fn note(class: &str, text: &str, detail: Option<&str>) -> String {
    let title = detail
        .map(|d| format!(" title=\"{}\"", esc_attr(d)))
        .unwrap_or_default();
    format!(
        "<div class=\"sheet-note {class}\"{title}>{}</div>",
        esc(text)
    )
}

fn omitted_note(omitted: usize) -> String {
    if omitted == 0 {
        String::new()
    } else {
        note("", &format!("{omitted} more rows are not shown."), None)
    }
}

/// The aggregate footer; `lead` adds the empty cell under a table's title column.
fn footer_row(footer: &Option<Vec<Option<Agg>>>, lead: bool) -> String {
    let Some(cells) = footer else {
        return String::new();
    };
    let mut out = String::from("<tfoot><tr>");
    if lead {
        out.push_str("<td></td>");
    }
    for cell in cells {
        match cell {
            Some(agg) => out.push_str(&format!(
                "<td><span class=\"sheet-agg-label\">{}</span> {}</td>",
                esc(&agg.label),
                esc(&agg.text)
            )),
            None => out.push_str("<td></td>"),
        }
    }
    out.push_str("</tr></tfoot>");
    out
}

/// Everything `render_block` threads through, so a sheet can anchor its rows in
/// the page's own numbering and re-enter `render_block` for a grid cell.
pub(super) struct Emit<'a, 'b> {
    pub ctx: &'a Ctx<'b>,
    pub slug: &'a str,
    pub title: &'a str,
    pub anchors: &'a PageAnchors,
    /// Rows already given their anchor: a card shown in two board columns
    /// carries the block's `id` once, so every page anchor stays unique.
    pub anchored: HashSet<*const DocBlock>,
    pub index: &'a mut Vec<serde_json::Value>,
    pub opts: PrintOpts,
    pub tree_depth: usize,
}

/// ` id="…"` (and search-index entry) for a row/card block's first emission, from
/// the page's one `PageAnchors` answer that `render_block` uses; empty for a repeat.
fn row_anchor(row: &DocBlock, e: &mut Emit) -> String {
    if !e.anchored.insert(row) {
        return String::new();
    }
    let anchor = e.anchors.get(row);
    let text = ast_plain_text(&body_blocks(row.raw()));
    if !text.is_empty() {
        e.index
            .push(json!({"slug": e.slug, "title": e.title, "anchor": anchor, "text": text}));
    }
    format!(" id=\"{}\"", esc_attr(&anchor))
}

fn render_table(owner: Option<&DocBlock>, body: &Body, e: &mut Emit, out: &mut String) {
    let Body::Table {
        columns,
        rows,
        footer,
        filter_error,
        omitted,
    } = body
    else {
        return;
    };
    if let Some(err) = filter_error {
        out.push_str(&note("sheet-filter-error", "Filter disabled", Some(err)));
    }
    out.push_str("<div class=\"sheet-scroll\"><table class=\"sheet-table\"><thead><tr>");
    for column in columns {
        let marker = if column.formula {
            "<span class=\"sheet-formula-marker\">ƒ</span>"
        } else {
            ""
        };
        out.push_str(&format!("<th>{marker}{}</th>", esc(&column.label)));
    }
    out.push_str("</tr></thead>");
    out.push_str(&footer_row(footer, true));
    out.push_str("<tbody>");
    for row in rows {
        let id = owner
            .and_then(|o| o.children.get(row.ix))
            .map(|b| row_anchor(b, e))
            .unwrap_or_default();
        out.push_str(&format!(
            "<tr{id}{}><td>{}</td>",
            style_attr(&row.bg),
            inline(&row.title, e.ctx)
        ));
        for cell in &row.cells {
            out.push_str(&format!("<td>{}</td>", cell_html(cell, e.ctx)));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
    out.push_str(&omitted_note(*omitted));
}

fn render_board(owner: Option<&DocBlock>, body: &Body, e: &mut Emit, out: &mut String) {
    let Body::Board {
        columns,
        filter_error,
        omitted,
    } = body
    else {
        return;
    };
    if let Some(err) = filter_error {
        out.push_str(&note("sheet-filter-error", "Filter disabled", Some(err)));
    }
    out.push_str("<div class=\"sheet-board\">");
    for column in columns {
        out.push_str(&format!(
            "<div class=\"sheet-board-col\"><h3>{} <span class=\"sheet-board-count\">{}</span></h3><ul>",
            esc(&column.label),
            column.cards.len()
        ));
        for card in &column.cards {
            let id = owner
                .and_then(|o| o.children.get(card.ix))
                .map(|b| row_anchor(b, e))
                .unwrap_or_default();
            let c = &card.chips;
            let mut chips = String::new();
            if !c.priority.is_empty() {
                chips.push_str(&cell_html(
                    &Cell::Priority {
                        raw: c.priority.clone(),
                        text: c.priority.clone(),
                    },
                    e.ctx,
                ));
            }
            for (cls, text) in [("scheduled", &c.scheduled), ("deadline", &c.deadline)] {
                if !text.is_empty() {
                    chips.push_str(&cell_html(
                        &Cell::Date {
                            cls: cls.into(),
                            text: text.clone(),
                        },
                        e.ctx,
                    ));
                }
            }
            if !c.tags.is_empty() {
                chips.push_str(&cell_html(
                    &Cell::Chips {
                        values: c.tags.clone(),
                    },
                    e.ctx,
                ));
            }
            out.push_str(&format!(
                "<li{id}{}><div class=\"sheet-card-title\">{}</div><div class=\"sheet-card-chips\">{chips}</div></li>",
                style_attr(&card.bg),
                inline(&card.title, e.ctx)
            ));
        }
        out.push_str("</ul></div>");
    }
    out.push_str("</div>");
    out.push_str(&omitted_note(*omitted));
}

fn render_grid(owner: &DocBlock, at: &SheetPath, body: &Body, e: &mut Emit, out: &mut String) {
    let Body::Grid {
        cols,
        header,
        footer,
        omitted,
    } = body
    else {
        return;
    };
    let (mut head, mut rows) = (String::new(), String::new());
    for (r, row) in owner.children.iter().enumerate() {
        let is_head = *header && r == 0;
        let tag = if is_head { "th" } else { "td" };
        let dst = if is_head { &mut head } else { &mut rows };
        dst.push_str("<tr>");
        let row_path = at.child(r);
        let ords = render_facets::siblings(&row.children, 0);
        for c in 0..*cols {
            match row.children.get(c) {
                Some(cell) => {
                    dst.push_str(&format!("<{tag}><ul class=\"sheet-cell-list\">"));
                    render_block(
                        cell,
                        dst,
                        e.ctx,
                        e.slug,
                        e.title,
                        e.anchors,
                        e.index,
                        e.opts,
                        ords[c],
                        e.tree_depth + 2,
                        &row_path.child(c),
                    );
                    dst.push_str(&format!("</ul></{tag}>"));
                }
                None => dst.push_str(&format!("<{tag} class=\"sheet-hole\"></{tag}>")),
            }
        }
        dst.push_str("</tr>");
    }
    out.push_str("<div class=\"sheet-scroll\"><table class=\"sheet-grid\">");
    if !head.is_empty() {
        out.push_str(&format!("<thead>{head}</thead>"));
    }
    out.push_str(&format!(
        "<tbody>{rows}</tbody>{}</table></div>",
        footer_row(footer, false)
    ));
    out.push_str(&omitted_note(*omitted));
}

/// Lay out the app's answer for `owner` in place of its plain child outline.
/// Returns true when a sheet was emitted; false leaves the caller to render the
/// outline (after a visible note when the answer cannot be used).
pub(super) fn emit(owner: &DocBlock, at: &SheetPath, e: &mut Emit, out: &mut String) -> bool {
    let Some(sheets) = e.ctx.graph.and_then(|g| g.sheets) else {
        return false;
    };
    let Some(export) = sheets.0.get(&(e.title.to_owned(), at.to_vec())) else {
        return false;
    };
    // A query-backed answer replaced the block's body (`emit_query`); the
    // children stay the plain outline below it.
    if export.query {
        return false;
    }
    if export.fp != fingerprint(owner) {
        out.push_str(&note(
            "",
            "This sheet changed while the export was prepared; showing its outline.",
            None,
        ));
        return false;
    }
    match &export.body {
        Body::Error { message } => {
            out.push_str(&note(
                "",
                "This sheet could not be computed; showing its outline.",
                Some(message),
            ));
            return false;
        }
        Body::Table { .. } => render_table(Some(owner), &export.body, e, out),
        Body::Board { .. } => render_board(Some(owner), &export.body, e, out),
        Body::Grid { .. } => render_grid(owner, at, &export.body, e, out),
    }
    true
}

/// The properties a sheet owner still prints as chips: its `tine.*` view
/// configuration is chrome (it drives the layout below), so it is dropped when
/// the app computed a sheet for the block. Any other block keeps every
/// property. O(properties); the sheet lookup is O(1).
pub(super) fn chip_props(
    props: Vec<(String, String)>,
    ctx: &Ctx,
    title: &str,
    at: &SheetPath,
) -> Vec<(String, String)> {
    let laid_out = ctx
        .graph
        .and_then(|g| g.sheets)
        .and_then(|sheets| sheets.0.get(&(title.to_owned(), at.to_vec())))
        .is_some_and(|export| !matches!(export.body, Body::Error { .. }));
    if !laid_out {
        return props;
    }
    props
        .into_iter()
        .filter(|(key, _)| !tine_core::doc::property_key_norm(key).starts_with("tine."))
        .collect()
}

/// Lay out the app's answer for a query-backed sheet block in place of its
/// `{{query}}` macro's flat result list. False (the caller renders the list,
/// which also states what it omits) when there is no answer, the query's
/// result is no longer the one the app computed from. Rows on pages this export
/// does not publish were left out before the app computed the sheet.
pub(super) fn emit_query(
    owner: &DocBlock,
    at: &SheetPath,
    found: &(String, String),
    e: &mut Emit,
    out: &mut String,
) -> bool {
    let Some(graph) = e.ctx.graph else {
        return false;
    };
    let Some(sheets) = graph.sheets else {
        return false;
    };
    let Some(export) = sheets.0.get(&(e.title.to_owned(), at.to_vec())) else {
        return false;
    };
    if !export.query {
        return false;
    }
    let Some(rows) = query_rows(graph, owner, found, &|page| {
        publish_page_allowed(e.ctx, page)
    }) else {
        return false;
    };
    if export.fp != rows.fp {
        out.push_str(&note(
            "",
            "This query changed while the export was prepared; showing its results.",
            None,
        ));
        return false;
    }
    match &export.body {
        Body::Error { message } => {
            out.push_str(&note(
                "",
                "This sheet could not be computed; showing the query results.",
                Some(message),
            ));
            return false;
        }
        Body::Table { .. } => render_table(None, &export.body, e, out),
        Body::Board { .. } => render_board(None, &export.body, e, out),
        Body::Grid { .. } => return false,
    }
    true
}
