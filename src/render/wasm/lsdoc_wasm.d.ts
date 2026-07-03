/* tslint:disable */
/* eslint-disable */

/**
 * The lsdoc git tag this wasm was built against (set by `build:wasm` via the
 * `LSDOC_TAG` env, read from tine-core's Cargo.toml — the single source of truth).
 * Surfaced to the frontend for diagnostics; the hard stale-wasm guard lives in the
 * build:wasm script (it refuses to build if this crate's pin ≠ tine-core's pin).
 * See docs/wasm-parse-plan.md §7D.
 */
export function lsdoc_tag(): string;

/**
 * Parse one de-bulleted block body into lsdoc's render AST, serialized to JSON.
 *
 * Mirrors `tine_core::render::parse_block` EXACTLY (the OG-faithful boundary):
 * re-prepend the block pattern (`-` Markdown / `*` Org) to `raw.trim_start()`,
 * then `lsdoc::parse`. The first returned block carries the block header
 * (marker / priority / heading size); continuation constructs follow as siblings.
 *
 * KEEP IN SYNC with `crates/tine-core/src/render.rs::parse_block`. The two share
 * no code (this crate can't cheaply depend on tine-core — see Cargo.toml), so the
 * 3-line re-prepend is duplicated; the transition diff-oracle (parse via WASM and
 * via the `parse_blocks` IPC, assert equal) guards against drift.
 */
export function parse_block_json(raw: string, is_org: boolean): string;

/**
 * Render one de-bulleted block body to lsdoc's CANONICAL HTML skeleton (M3 render
 * contract — `lsdoc::render_html`): structural tags + classes + `data-*` hooks, no
 * ref/asset/math/macro resolution. Re-bullets EXACTLY like `parse_block_json` so the
 * rendered AST is identical, then renders it.
 *
 * NOT on the app's render path — the frontend renders the AST reactively (interactive
 * DOM, resolved refs/assets), never lsdoc's HTML string. This exists ONLY so the
 * anti-drift gate (`src/render/skeleton-drift.test.tsx`) can compare lsdoc's canonical
 * skeleton against the frontend's reactive skeleton, from the SAME wasm the app ships —
 * catching drift between the two renderers (Option C2: both conform to one skeleton).
 */
export function render_block_html(raw: string, is_org: boolean): string;

export function __tineReinstantiate(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly lsdoc_tag: () => [number, number];
    readonly parse_block_json: (a: number, b: number, c: number) => [number, number];
    readonly render_block_html: (a: number, b: number, c: number) => [number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
