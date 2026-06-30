//! WASM entry point for Tine's in-browser block parser.
//!
//! The frontend renders block bodies from lsdoc's AST. To parse synchronously
//! (no Tauri IPC, no fallback flash), `lsdoc` is compiled to WebAssembly and the
//! AST is shipped to JS as a JSON string — the SAME `serde_json` encoding the IPC
//! path used, which `src/render/ast.ts` already mirrors 1:1.

use wasm_bindgen::prelude::*;

/// Parse one de-bulleted block body into lsdoc's render AST, serialized to JSON.
///
/// Mirrors `tine_core::render::parse_block` EXACTLY (the OG-faithful boundary):
/// re-prepend the block pattern (`-` Markdown / `*` Org) to `raw.trim_start()`,
/// then `lsdoc::parse`. The first returned block carries the block header
/// (marker / priority / heading size); continuation constructs follow as siblings.
///
/// KEEP IN SYNC with `crates/tine-core/src/render.rs::parse_block`. The two share
/// no code (this crate can't cheaply depend on tine-core — see Cargo.toml), so the
/// 3-line re-prepend is duplicated; the transition diff-oracle (parse via WASM and
/// via the `parse_blocks` IPC, assert equal) guards against drift.
#[wasm_bindgen]
pub fn parse_block_json(raw: &str, is_org: bool) -> String {
    let (pattern, fmt) = if is_org { ("*", "org") } else { ("-", "md") };
    let input = format!("{pattern} {}", raw.trim_start());
    let ast = lsdoc::parse(&input, fmt);
    serde_json::to_string(&ast).unwrap_or_else(|_| "[]".to_string())
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
    let (pattern, fmt, rfmt) = if is_org {
        ("*", "org", lsdoc::Format::Org)
    } else {
        ("-", "md", lsdoc::Format::Md)
    };
    let input = format!("{pattern} {}", raw.trim_start());
    let blocks = lsdoc::parse(&input, fmt);
    lsdoc::render_html(&blocks, &lsdoc::RenderOpts { format: rfmt })
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
