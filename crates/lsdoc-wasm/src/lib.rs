//! WASM entry point for Tine's in-browser block parser.
//!
//! The frontend renders block bodies from lsdoc's AST. To parse synchronously
//! (no Tauri IPC, no fallback flash), `lsdoc` is compiled to WebAssembly and the
//! AST is shipped to JS as a JSON string — the SAME `serde_json` encoding the IPC
//! path used, which `src/render/ast.ts` already mirrors 1:1.

use wasm_bindgen::prelude::*;

// Panic = abort, so a Rust panic is a bare wasm `unreachable` trap whose message is
// otherwise lost (the og "Unreachable code should not be executed (evaluating
// 'page_header_json')" report carried no cause). The hook runs BEFORE the abort: it
// logs the message to the console and keeps the latest one for `last_panic`, which the
// glue (scripts/build-wasm.mjs) reads when it recovers from a trap. Only the location and a
// literal message are kept (I-5: no page text in logs). The hook itself must never panic
// (that would be a second abort inside the abort): it uses `try_borrow_mut` and formats
// into a fresh String.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(message: &str);
}

thread_local! {
    static LAST_PANIC: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

#[wasm_bindgen(start)]
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // I-5: a formatted panic message can embed page text (str slicing panics quote the
        // string), and the thrown Error reaches the diagnostics log. Keep the location, which
        // names the site, and the message only when it is a fixed literal.
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let literal = info.payload().downcast_ref::<&str>().copied().unwrap_or("<formatted message withheld>");
        let message = format!("lsdoc-wasm panic at {place}: {literal}");
        LAST_PANIC.with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut() {
                *slot = message.clone();
            }
        });
        console_error(&message);
    }));
}

/// The message of the most recent panic in this instance ("" if none). A trapped
/// instance still answers small calls; the glue reads this before reinstantiating.
#[wasm_bindgen]
pub fn last_panic() -> String {
    LAST_PANIC.with(|slot| slot.try_borrow().map(|s| s.clone()).unwrap_or_default())
}

#[path = "../../tine-core/src/page_identity.rs"]
mod page_identity;
#[path = "../../tine-core/src/query/group_field.rs"]
mod group_field;

/// Native page-name key: O(name bytes), no I/O; Rust trim/lowercase/slashes/NFC.
#[wasm_bindgen]
pub fn page_identity_key(name: &str) -> String {
    page_identity::page_key(name)
}

/// Native group-field grammar: O(value bytes), no I/O; invalid tokens return null.
#[wasm_bindgen]
pub fn canonical_group_field(value: &str) -> Option<String> {
    group_field::canonical_group_token(value).map(str::to_owned)
}

#[path = "../../tine-core/src/page_filename.rs"]
mod page_filename;

/// Encode a Windows-safe Logseq page stem; legacy selects percent-encoded
/// namespaces, false selects triple-lowbar. O(title bytes), no I/O or failure.
#[wasm_bindgen]
pub fn encode_page_name(name: &str, legacy: bool) -> String {
    page_filename::encode_page_name(name, legacy)
}

/// Decode a Logseq page stem in legacy or triple-lowbar (legacy = false) format.
/// O(stem bytes), no I/O; malformed percent escapes are preserved.
#[wasm_bindgen]
pub fn decode_page_name(stem: &str, legacy: bool) -> String {
    page_filename::decode_page_name(stem, legacy)
}

/// Split already-parsed linkable property values with the native separator.
/// O(value bytes), without parsing or I/O. Empty members retain their position.
#[wasm_bindgen]
pub fn split_linkable_property(value: &str) -> Vec<String> {
    value.split(block_regions::is_linkable_property_separator).map(str::to_owned).collect()
}

#[path = "../../tine-core/src/block_regions.rs"]
mod block_regions;
mod render {
    pub(crate) use crate::lsdoc_block_parse::parse_block;
    pub(crate) fn parse_text_bounded(text: &str, is_org: bool) -> Option<lsdoc::ast::Projection> {
        crate::lsdoc_block_parse::parse_text_bounded(text, if is_org { "org" } else { "md" })
    }
}

#[path = "../../tine-core/src/logbook.rs"]
mod logbook;
#[path = "../../tine-core/src/media_mime.rs"]
mod media_mime;
#[path = "../../lsdoc-block-parse.rs"]
mod lsdoc_block_parse;
#[path = "../../tine-core/src/property_line.rs"]
mod property_line;

/// Accepted single Markdown property line, using the native borrowed grammar.
/// O(line bytes), no parse or I/O; malformed input serializes as null.
#[wasm_bindgen]
pub fn property_line_json(line: &str) -> String {
    serde_json::to_string(&property_line::parse_property_line(line)).unwrap()
}

/// Whole-preamble property ownership, with parser-owned literals excluded.
/// O(preamble bytes); no block wrapper or I/O.
#[wasm_bindgen]
pub fn page_regions_json(raw: &str, is_org: bool) -> String {
    let regions = block_regions::parse_document(raw, is_org);
    serde_json::to_string(&regions.page_properties().collect::<Vec<_>>()).unwrap()
}

/// Accepted marker/priority of one block with their byte spans (`block_regions::header_tokens`).
/// O(block bytes); no regions walk.
#[wasm_bindgen]
pub fn header_tokens_json(raw: &str, is_org: bool) -> String {
    serde_json::to_string(&block_regions::header_tokens(raw, is_org)).unwrap()
}

/// Markdown page header (leading accepted properties, see `block_regions::page_header`).
/// O(preamble bytes); no I/O.
#[wasm_bindgen]
pub fn page_header_json(raw: &str) -> String {
    serde_json::to_string(&block_regions::page_header(raw)).unwrap()
}

/// Shared native target classification on an accepted AST link. O(target).
#[wasm_bindgen]
pub fn reference_target_name(kind: &str, value: &str, label: &str, org: bool, filename_candidates: bool) -> Option<String> {
    block_regions::reference_target_name(kind, value, label, org, filename_candidates)
}

/// Names inside an accepted NestedLink node; O(node bytes).
#[wasm_bindgen]
pub fn nested_reference_names(content: &str) -> Vec<String> {
    block_regions::nested_reference_names(content)
}

/// MIME from the final case-insensitive path extension; O(path bytes), no I/O.
/// Unknown extensions return application/octet-stream. Shared with native media.
#[wasm_bindgen]
pub fn mime_from_path(path: &str) -> String {
    media_mime::from_path(path).to_string()
}

/// Parse one de-bulleted block body into lsdoc's render AST, serialized to JSON.
///
/// Mirrors `tine_core::render::parse_block` exactly. Both bridges compile the same
/// shared boundary helper: OG-compatible re-bullet parsing plus Tine's deliberate
/// correction for line-leading Markdown inline code containing `::`.
#[wasm_bindgen]
pub fn parse_block_json(raw: &str, is_org: bool) -> String {
    let ast = lsdoc_block_parse::parse_block(raw, is_org);
    lsdoc::blocks_to_json(&ast).unwrap_or_else(|_| "[]".to_string())
}

/// Parse a WHOLE FILE (raw graph file text, NOT re-bulleted) into lsdoc's observable
/// projection `{blocks, refs}`, serialized to JSON — the same thing the `lsdoc-parse`
/// CLI emits. Unlike `parse_block_json` (one de-bulleted block), this is document-level,
/// for the "Help improve Tine" diff panel, which compares whole files against mldoc
/// exactly as `lsdoc/tools/graph-check.mjs` does. Not on the render path.
#[wasm_bindgen]
pub fn parse_document_json(text: &str, is_org: bool) -> String {
    let fmt = if is_org { "org" } else { "md" };
    // Too deep for the bounded door (I-22) ⇒ the same "{}" as a failed encode.
    lsdoc_block_parse::parse_text_bounded(text, fmt)
        .and_then(|projection| lsdoc::projection_to_json(&projection).ok())
        .unwrap_or_else(|| "{}".to_string())
}

/// Render one de-bulleted block body to lsdoc's CANONICAL HTML skeleton (M3 render
/// contract — `lsdoc::render_html`): structural tags + classes + `data-*` hooks, no
/// ref/asset/math/macro resolution. Re-bullets EXACTLY like `parse_block_json` so the
/// rendered AST is identical, then renders it.
///
/// NOT on the app's render path — the frontend renders the AST reactively (interactive
/// DOM, resolved refs/assets), never lsdoc's HTML string. This exists ONLY so the
/// anti-drift gate (`src/render/skeleton-drift.test.tsx`) can compare lsdoc's canonical
/// skeleton against the frontend's reactive skeleton, from the SAME wasm the app ships —
/// catching drift between the two renderers (Option C2: both conform to one skeleton).
#[wasm_bindgen]
pub fn render_block_html(raw: &str, is_org: bool) -> String {
    let rfmt = if is_org {
        lsdoc::Format::Org
    } else {
        lsdoc::Format::Md
    };
    let blocks = lsdoc_block_parse::parse_block(raw, is_org);
    lsdoc::render_html(&blocks, &lsdoc::RenderOpts { format: rfmt })
}

#[wasm_bindgen]
pub fn logbook_clock_in(raw: &str, is_org: bool, with_seconds: bool) -> String {
    logbook::clock_in_at(raw, logbook_format(is_org), with_seconds, now_parts())
}

#[wasm_bindgen]
pub fn logbook_clock_out(raw: &str, is_org: bool, with_seconds: bool) -> String {
    logbook::clock_out_at(raw, logbook_format(is_org), with_seconds, now_parts())
}

#[wasm_bindgen]
pub fn logbook_apply_marker_transition(
    raw: &str,
    is_org: bool,
    old_marker: &str,
    new_marker: &str,
    enabled: bool,
    with_seconds: bool,
) -> String {
    let old = (!old_marker.is_empty()).then_some(old_marker);
    let new = (!new_marker.is_empty()).then_some(new_marker);
    logbook::apply_marker_transition_at(
        raw,
        logbook_format(is_org),
        old,
        new,
        enabled,
        with_seconds,
        now_parts(),
    )
}

#[wasm_bindgen]
pub fn logbook_info_json(raw: &str, is_org: bool) -> String {
    let (rows, seconds) = logbook::clock_info(raw, is_org);
    let rows = rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "type": r.kind,
                "start": r.start,
                "end": r.end,
                "span": r.span,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "seconds": seconds,
        "summary": logbook::format_compact_duration(seconds),
        "rows": rows,
    })
    .to_string()
}

fn logbook_format(is_org: bool) -> logbook::LogbookFormat {
    if is_org {
        logbook::LogbookFormat::Org
    } else {
        logbook::LogbookFormat::Markdown
    }
}

fn now_parts() -> logbook::TimestampParts {
    let d = js_sys::Date::new_0();
    logbook::TimestampParts {
        year: d.get_full_year() as i32,
        month: d.get_month() + 1,
        day: d.get_date(),
        weekday: d.get_day(),
        hour: d.get_hours(),
        minute: d.get_minutes(),
        second: d.get_seconds(),
    }
}

/// The lsdoc git tag this wasm was built against (set by `build:wasm` via the
/// `LSDOC_TAG` env, read from tine-core's Cargo.toml — the single source of truth).
/// Surfaced to the frontend for diagnostics; the hard stale-wasm guard lives in the
/// build:wasm script (it refuses to build if this crate's pin ≠ tine-core's pin).
/// See docs/wasm-parse-plan.md §7D.
#[wasm_bindgen]
pub fn lsdoc_tag() -> String {
    option_env!("LSDOC_TAG").unwrap_or("unknown").to_string()
}

#[wasm_bindgen]
pub fn parse_block_bundle_json(raw: &str, is_org: bool) -> String {
    let blocks = lsdoc_block_parse::parse_block(raw, is_org);
    let regions = block_regions::from_blocks(raw, is_org, &blocks);
    format!(
        "{{\"blocks\":{},\"regions\":{},\"sole_macro\":{}}}",
        lsdoc::blocks_to_json(&blocks).unwrap(),
        serde_json::to_string(&regions).unwrap(),
        serde_json::to_string(&standalone_macro::sole_macro(&blocks)).unwrap()
    )
}
#[wasm_bindgen]
pub fn edit_block_regions_json(
    raw: &str,
    is_org: bool,
    regions: &JsValue,
    request: &JsValue,
) -> Result<String, JsValue> {
    use block_regions::{BlockRegions, Drawer, Edit, Planning, Property, Range};
    fn get(v: &JsValue, key: &str) -> JsValue {
        js_sys::Reflect::get(v, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
    }
    fn text(v: &JsValue, key: &str) -> String {
        get(v, key).as_string().unwrap_or_default()
    }
    fn optional(v: &JsValue, key: &str) -> Option<String> {
        get(v, key).as_string()
    }
    fn array(v: &JsValue, key: &str) -> js_sys::Array {
        js_sys::Array::from(&get(v, key))
    }
    let range = |v: &JsValue| -> Result<Range, JsValue> {
        let a = js_sys::Array::from(v);
        let start = a
            .get(0)
            .as_f64()
            .ok_or_else(|| JsValue::from_str("Invalid region"))? as usize;
        let end = a
            .get(1)
            .as_f64()
            .ok_or_else(|| JsValue::from_str("Invalid region"))? as usize;
        if start > end
            || end > raw.len()
            || !raw.is_char_boundary(start)
            || !raw.is_char_boundary(end)
        {
            return Err(JsValue::from_str("Invalid region"));
        }
        Ok(Range(start, end))
    };
    let mut r = BlockRegions::default();
    r.quarantined = get(regions, "quarantined").as_bool().unwrap_or(true);
    for v in array(regions, "literals").iter() {
        r.literals.push(range(&v)?);
    }
    for v in array(regions, "property_regions").iter() {
        r.property_regions.push(range(&v)?);
    }
    for p in array(regions, "properties").iter() {
        r.properties.push(Property {
            key: text(&p, "key"),
            value: text(&p, "value"),
            line: range(&get(&p, "line"))?,
            key_range: range(&get(&p, "key_range"))?,
            value_range: range(&get(&p, "value_range"))?,
            region: get(&p, "region").as_f64().unwrap_or(0.0) as usize,
            primary: get(&p, "primary").as_bool().unwrap_or(false),
            directive: get(&p, "directive").as_bool().unwrap_or(false),
            applicable: get(&p, "applicable").as_bool().unwrap_or(false),
        });
    }
    for p in array(regions, "planning").iter() {
        r.planning.push(Planning {
            kind: text(&p, "kind"),
            line: range(&get(&p, "line"))?,
            timestamp: range(&get(&p, "timestamp"))?,
            date: serde_json::Value::Null,
        });
    }
    for d in array(regions, "drawers").iter() {
        let clocks = array(&d, "clocks")
            .iter()
            .map(|v| range(&v))
            .collect::<Result<Vec<_>, _>>()?;
        r.drawers.push(Drawer {
            name: text(&d, "name"),
            range: range(&get(&d, "range"))?,
            close: get(&d, "close").as_f64().unwrap_or(0.0) as usize,
            clocks,
        });
    }
    if text(request, "kind") == "reattach_properties" {
        return r.reattach_org_properties(raw, &text(request, "hidden"))
            .map_err(|e| JsValue::from_str(&e));
    }
    let edit = match text(request, "kind").as_str() {
        "property" => Edit::Property {
            key: text(request, "key"),
            value: optional(request, "value"),
        },
        "planning" => Edit::Planning {
            which: text(request, "which"),
            value: optional(request, "value"),
        },
        "strip_copy" => Edit::StripCopy {
            template: get(request, "template").as_bool().unwrap_or(false),
        },
        "visible" => Edit::Visible,
        "normalize_planning" => Edit::NormalizePlanning,
        "drawer_row" => Edit::DrawerRow {
            name: text(request, "name"),
            value: text(request, "value"),
        },
        _ => return Err(JsValue::from_str("Invalid structural operation")),
    };
    r.apply(raw, is_org, edit)
        .map_err(|e| JsValue::from_str(&e))
}

/// Shared comparison form; O(text bytes), no graph access.
#[wasm_bindgen]
pub fn search_fold(text: &str, remove_accents: bool) -> String {
    if remove_accents { tine_search::canonical_fold(text) } else { tine_search::literal_fold(text) }
}

/// Bounded original UTF-16 evidence, O(text × needle scalars).
#[wasm_bindgen]
pub fn search_substring_spans_json(text: &str, needle: &str, limit: usize, remove_accents: bool) -> String {
    serde_json::to_string(&tine_search::substring_spans(text, needle, limit, remove_accents)).unwrap()
}

/// Compile once per hot query (four retained matchers maximum). No raw handles
/// cross into JS, so a parser re-instantiation also safely resets this cache.
fn with_search<T>(query: &str, remove_accents: bool, f: impl FnOnce(&tine_search::Matcher) -> T) -> T {
    use std::cell::RefCell;
    thread_local! { static CACHE: RefCell<Vec<(String, bool, tine_search::Matcher)>> = const { RefCell::new(Vec::new()) }; }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let at = cache.iter().position(|(q, p, _)| q == query && *p == remove_accents);
        let at = if let Some(at) = at { at } else {
            if cache.len() == 4 { cache.remove(0); }
            cache.push((query.to_owned(), remove_accents, tine_search::Matcher::parse_with_policy(query, remove_accents)));
            cache.len() - 1
        };
        f(&cache[at].2)
    })
}

/// Parse metadata for UI builders. Same grammar, errors and folds as native.
#[wasm_bindgen]
pub fn search_query_json(query: &str, remove_accents: bool) -> String {
    use tine_search::Matcher;
    with_search(query, remove_accents, |matcher| {
        let mut value = match matcher {
            Matcher::Empty => serde_json::json!({"kind":"empty"}),
            Matcher::InvalidRegex(error) => serde_json::json!({"kind":"invalid","error":error}),
            Matcher::Regex(re) => serde_json::json!({"kind":"regex","pattern":re.as_str()}),
            Matcher::Boolean(groups) => serde_json::json!({"kind":"boolean","groups":groups.iter().map(|group| group.iter().map(|term| serde_json::json!({"text":term.text,"negated":term.negated,"quoted":term.quoted})).collect::<Vec<_>>()).collect::<Vec<_>>()}),
        };
        value["simple"] = serde_json::json!(matcher.simple_term());
        value.to_string()
    })
}

/// Membership against a policy-matched pre-folded body; O(text × terms).
#[wasm_bindgen]
pub fn search_matches(query: &str, remove_accents: bool, lower: &str, original: &str) -> bool {
    with_search(query, remove_accents, |m| m.matches(lower, original))
}

/// UTF-16 search evidence, capped by limit. First mode retains zero-width hits
/// and considers all positive terms; multi-range mode uses the satisfied group.
#[wasm_bindgen]
pub fn search_spans_json(query: &str, remove_accents: bool, text: &str, limit: usize, first: bool) -> String {
    with_search(query, remove_accents, |m| {
        let spans = if first { m.first_span(text, remove_accents).into_iter().collect() } else { m.spans(text, limit, remove_accents) };
        serde_json::to_string(&spans).unwrap()
    })
}

#[path = "../../tine-core/src/edn.rs"]
mod edn;
#[path = "../../tine-core/src/query_edn.rs"]
mod query_edn;

/// Query EDN reads/splices from the same byte-span reader as native macro_text.
/// O(source bytes), at most 1 MiB / 128 levels; null refuses unreadable EDN.
/// Title edits preserve all unrelated bytes. No I/O or graph state.
#[wasm_bindgen]
pub fn query_edn_json(source: &str, operation: &str, value: &str) -> String {
    match operation {
        "begin_query" => serde_json::to_string(&query_edn::inspect_begin_query(source)).unwrap(),
        "options" => serde_json::to_string(&query_edn::options(source)).unwrap(),
        "title" => serde_json::to_string(&query_edn::edit_title(source, value)).unwrap(),
        "read" => serde_json::to_string(&query_edn::read(source)).unwrap(),
        "split" => serde_json::to_string(&query_edn::split_trailing_map(source)).unwrap(),
        "quote" => serde_json::to_string(&edn::to_string(&edn::Edn::Str(source.into()))).unwrap(),
        "unquote" => {
            let text = match edn::parse_strict(source) { Some(edn::Edn::Str(text)) => Some(text), _ => None };
            serde_json::to_string(&text).unwrap()
        }
        _ => "null".into(),
    }
}

#[path = "../../tine-core/src/query/macro_names.rs"]
mod macro_names;
#[path = "../../tine-core/src/query/macro_extent.rs"]
mod macro_extent;

/// Query raw extents from the native reader. O(raw bytes), no parser or I/O;
/// JSON offsets are UTF-8 bytes. Unterminated candidates are omitted.
#[wasm_bindgen]
pub fn query_macro_extents_json(raw: &str) -> String {
    serde_json::to_string(&macro_extent::query_macro_extents(raw)).unwrap()
}

/// Whether a parser-tokenized macro name is a query; O(name bytes), no I/O.
#[wasm_bindgen]
pub fn is_query_macro_name(name: &str) -> bool {
    macro_extent::is_query_macro_name(name)
}

/// The raw reader's literal grammar for a macro name; O(name bytes), no I/O.
#[wasm_bindgen]
pub fn query_macro_is_tql(name: &str) -> bool {
    macro_extent::FormFamily::for_macro_name(name) == macro_extent::FormFamily::Tql
}

#[path = "../../tine-core/src/date.rs"]
mod date;

thread_local! {
    static DATE_FORMATS: std::cell::RefCell<Vec<(String, date::Format)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn with_date_format<T>(pattern: &str, f: impl FnOnce(&date::Format) -> T) -> T {
    DATE_FORMATS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((_, format)) = cache.iter().find(|(key, _)| key == pattern) {
            return f(format);
        }
        if cache.len() == 16 { cache.remove(0); }
        cache.push((pattern.to_owned(), date::Format::compile(pattern)));
        f(&cache.last().unwrap().1)
    })
}

/// Native calendar format grammar, cached (16 patterns). O(title + pattern) on
/// cold compile, O(title) warm; null means invalid. No clock, graph or I/O.
#[wasm_bindgen]
pub fn parse_journal_format_json(text: &str, pattern: &str) -> String {
    with_date_format(pattern, |format| {
        serde_json::to_string(&format.parse(text).map(|d| serde_json::json!({"y":d.year,"m":d.month,"d":d.day}))).unwrap()
    })
}

/// Native date formatter over explicit civil parts; same bounded format cache.
#[wasm_bindgen]
pub fn format_journal_date(year: i32, month: u32, day: u32, pattern: &str) -> String {
    with_date_format(pattern, |format| format.format(date::JournalDate { year, month, day }))
}

#[path = "../../tine-core/src/standalone_macro.rs"]
mod standalone_macro;

/// Inline-only parsing for property values: bounded by the same shared native
/// tree admission. O(value bytes); null refuses excessive depth, no I/O.
#[wasm_bindgen]
pub fn parse_inline_json(raw: &str, is_org: bool) -> String {
    serde_json::to_string(&lsdoc_block_parse::parse_inline_bounded(raw, if is_org { "org" } else { "md" })).unwrap()
}

#[path = "../../tine-core/src/ordinal.rs"]
mod ordinal;
#[path = "../../tine-core/src/pdf_key.rs"]
mod pdf_key;

/// Ordered-list label without surface punctuation; O(label bytes), no parse/I/O.
#[wasm_bindgen]
pub fn ordered_list_glyph(index: u32, depth: u32) -> String {
    ordinal::glyph(index, depth)
}

/// PDF identity with the existing preview/native suffix policy. O(filename bytes).
#[wasm_bindgen]
pub fn pdf_asset_key(filename: &str, preview: bool) -> String {
    pdf_key::asset_key(filename, if preview { pdf_key::PdfSuffix::Preview } else { pdf_key::PdfSuffix::Native })
}
