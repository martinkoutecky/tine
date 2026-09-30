//! WASM entry point for Tine's in-browser block parser.
//!
//! The frontend renders block bodies from lsdoc's AST. To parse synchronously
//! (no Tauri IPC, no fallback flash), `lsdoc` is compiled to WebAssembly and the
//! AST is shipped to JS as a JSON string — the SAME `serde_json` encoding the IPC
//! path used, which `src/render/ast.ts` already mirrors 1:1.

use wasm_bindgen::prelude::*;

#[path = "../../tine-core/src/block_regions.rs"]
mod block_regions;
mod render { pub(crate) use crate::lsdoc_block_parse::parse_block; }

#[path = "../../tine-core/src/logbook.rs"]
mod logbook;
#[path = "../../lsdoc-block-parse.rs"]
mod lsdoc_block_parse;
#[path = "../../tine-core/src/property_line.rs"]
mod property_line;

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
pub fn logbook_clock_out(raw: &str, with_seconds: bool) -> String {
    logbook::clock_out_at(raw, with_seconds, now_parts())
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
pub fn logbook_info_json(raw: &str) -> String {
    let (rows,seconds)=logbook::clock_info(raw);
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
    format!("{{\"blocks\":{},\"regions\":{}}}", lsdoc::blocks_to_json(&blocks).unwrap(), serde_json::to_string(&regions).unwrap())
}
#[wasm_bindgen]
pub fn edit_block_regions_json(raw: &str, is_org: bool, regions: &JsValue, request: &JsValue) -> Result<String, JsValue> {
    use block_regions::{BlockRegions,Range,Property,Planning,Drawer,Edit};
    fn get(v: &JsValue, key: &str) -> JsValue { js_sys::Reflect::get(v,&JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED) }
    fn text(v: &JsValue, key: &str) -> String { get(v,key).as_string().unwrap_or_default() }
    fn optional(v: &JsValue,key: &str) -> Option<String> {get(v,key).as_string()}
    fn array(v: &JsValue, key: &str) -> js_sys::Array { js_sys::Array::from(&get(v,key)) }
    let range = |v: &JsValue| -> Result<Range,JsValue> {
        let a=js_sys::Array::from(v);
        let start=a.get(0).as_f64().ok_or_else(||JsValue::from_str("Invalid region"))? as usize;
        let end=a.get(1).as_f64().ok_or_else(||JsValue::from_str("Invalid region"))? as usize;
        if start>end||end>raw.len()||!raw.is_char_boundary(start)||!raw.is_char_boundary(end) {return Err(JsValue::from_str("Invalid region"));}
        Ok(Range(start,end))
    };
    let mut r=BlockRegions::default();
    r.quarantined=get(regions,"quarantined").as_bool().unwrap_or(true);
    for v in array(regions,"literals").iter() {r.literals.push(range(&v)?);}
    for v in array(regions,"property_regions").iter() {r.property_regions.push(range(&v)?);}
    for p in array(regions,"properties").iter() {
        r.properties.push(Property {key:text(&p,"key"),value:text(&p,"value"),line:range(&get(&p,"line"))?,key_range:range(&get(&p,"key_range"))?,value_range:range(&get(&p,"value_range"))?,region:get(&p,"region").as_f64().unwrap_or(0.0) as usize,primary:get(&p,"primary").as_bool().unwrap_or(false)});
    }
    for p in array(regions,"planning").iter() {
        r.planning.push(Planning {kind:text(&p,"kind"),line:range(&get(&p,"line"))?,timestamp:range(&get(&p,"timestamp"))?,date:serde_json::Value::Null});
    }
    for d in array(regions,"drawers").iter() {
        let clocks=array(&d,"clocks").iter().map(|v|range(&v)).collect::<Result<Vec<_>,_>>()?;
        r.drawers.push(Drawer{name:text(&d,"name"),range:range(&get(&d,"range"))?,close:get(&d,"close").as_f64().unwrap_or(0.0) as usize,clocks});
    }
    let edit=match text(request,"kind").as_str() {
        "property"=>Edit::Property{key:text(request,"key"),value:optional(request,"value")},
        "planning"=>Edit::Planning{which:text(request,"which"),value:optional(request,"value")},
        "strip_copy"=>Edit::StripCopy{template:get(request,"template").as_bool().unwrap_or(false)},
        "visible"=>Edit::Visible,
        "normalize_planning"=>Edit::NormalizePlanning,
        "drawer_row"=>Edit::DrawerRow{name:text(request,"name"),value:text(request,"value")},
        _=>return Err(JsValue::from_str("Invalid structural operation")),
    };
    r.apply(raw,is_org,edit).map_err(|e|JsValue::from_str(&e))
}
