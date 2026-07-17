//! WASM entry point for Tine's in-browser block parser.
//!
//! The frontend renders block bodies from lsdoc's AST. To parse synchronously
//! (no Tauri IPC, no fallback flash), `lsdoc` is compiled to WebAssembly and the
//! AST is shipped to JS as a JSON string — the SAME `serde_json` encoding the IPC
//! path used, which `src/render/ast.ts` already mirrors 1:1.

use wasm_bindgen::prelude::*;

#[path = "../../tine-core/src/logbook.rs"]
mod logbook;
#[path = "../../lsdoc-block-parse.rs"]
mod lsdoc_block_parse;

/// Parse one de-bulleted block body into lsdoc's render AST, serialized to JSON.
///
/// Mirrors `tine_core::render::parse_block` exactly. Both bridges compile the same
/// shared boundary helper: OG-compatible re-bullet parsing plus Tine's deliberate
/// correction for line-leading Markdown inline code containing `::`.
#[wasm_bindgen]
pub fn parse_block_json(raw: &str, is_org: bool) -> String {
    let ast = lsdoc_block_parse::parse_block(raw, is_org);
    serde_json::to_string(&ast).unwrap_or_else(|_| "[]".to_string())
}

/// Parse a WHOLE FILE (raw graph file text, NOT re-bulleted) into lsdoc's observable
/// projection `{blocks, refs}`, serialized to JSON — the same thing the `lsdoc-parse`
/// CLI emits. Unlike `parse_block_json` (one de-bulleted block), this is document-level,
/// for the "Help improve Tine" diff panel, which compares whole files against mldoc
/// exactly as `lsdoc/tools/graph-check.mjs` does. Not on the render path.
#[wasm_bindgen]
pub fn parse_document_json(text: &str, is_org: bool) -> String {
    let fmt = if is_org { "org" } else { "md" };
    serde_json::to_string(&lsdoc::parse_format(text, fmt)).unwrap_or_else(|_| "{}".to_string())
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
    let rows = logbook::clock_rows(raw)
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
        "seconds": logbook::clock_summary_seconds(raw),
        "summary": logbook::clock_summary_compact(raw),
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
