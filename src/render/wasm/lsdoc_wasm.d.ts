/* tslint:disable */
/* eslint-disable */

/**
 * Decode a Logseq page stem in legacy or triple-lowbar (legacy = false) format.
 * O(stem bytes), no I/O; malformed percent escapes are preserved.
 */
export function decode_page_name(stem: string, legacy: boolean): string;

export function edit_block_regions_json(raw: string, is_org: boolean, regions: any, request: any): string;

/**
 * Encode a Windows-safe Logseq page stem; legacy selects percent-encoded
 * namespaces, false selects triple-lowbar. O(title bytes), no I/O or failure.
 */
export function encode_page_name(name: string, legacy: boolean): string;

/**
 * Whether a parser-tokenized macro name is a query; O(name bytes), no I/O.
 */
export function is_query_macro_name(name: string): boolean;

export function logbook_apply_marker_transition(raw: string, is_org: boolean, old_marker: string, new_marker: string, enabled: boolean, with_seconds: boolean): string;

export function logbook_clock_in(raw: string, is_org: boolean, with_seconds: boolean): string;

export function logbook_clock_out(raw: string, is_org: boolean, with_seconds: boolean): string;

export function logbook_info_json(raw: string, is_org: boolean): string;

/**
 * The lsdoc git tag this wasm was built against (set by `build:wasm` via the
 * `LSDOC_TAG` env, read from tine-core's Cargo.toml — the single source of truth).
 * Surfaced to the frontend for diagnostics; the hard stale-wasm guard lives in the
 * build:wasm script (it refuses to build if this crate's pin ≠ tine-core's pin).
 * See docs/wasm-parse-plan.md §7D.
 */
export function lsdoc_tag(): string;

/**
 * MIME from the final case-insensitive path extension; O(path bytes), no I/O.
 * Unknown extensions return application/octet-stream. Shared with native media.
 */
export function mime_from_path(path: string): string;

/**
 * Names inside an accepted NestedLink node; O(node bytes).
 */
export function nested_reference_names(content: string): string[];

/**
 * Markdown page header (leading accepted properties, see `block_regions::page_header`).
 * O(preamble bytes); no I/O.
 */
export function page_header_json(raw: string): string;

/**
 * Whole-preamble property ownership, with parser-owned literals excluded.
 * O(preamble bytes); no block wrapper or I/O.
 */
export function page_regions_json(raw: string, is_org: boolean): string;

export function parse_block_bundle_json(raw: string, is_org: boolean): string;

/**
 * Parse one de-bulleted block body into lsdoc's render AST, serialized to JSON.
 *
 * Mirrors `tine_core::render::parse_block` exactly. Both bridges compile the same
 * shared boundary helper: OG-compatible re-bullet parsing plus Tine's deliberate
 * correction for line-leading Markdown inline code containing `::`.
 */
export function parse_block_json(raw: string, is_org: boolean): string;

/**
 * Parse a WHOLE FILE (raw graph file text, NOT re-bulleted) into lsdoc's observable
 * projection `{blocks, refs}`, serialized to JSON — the same thing the `lsdoc-parse`
 * CLI emits. Unlike `parse_block_json` (one de-bulleted block), this is document-level,
 * for the "Help improve Tine" diff panel, which compares whole files against mldoc
 * exactly as `lsdoc/tools/graph-check.mjs` does. Not on the render path.
 */
export function parse_document_json(text: string, is_org: boolean): string;

/**
 * Accepted single Markdown property line, using the native borrowed grammar.
 * O(line bytes), no parse or I/O; malformed input serializes as null.
 */
export function property_line_json(line: string): string;

/**
 * Query EDN reads/splices from the same byte-span reader as native macro_text.
 * O(source bytes), at most 1 MiB / 128 levels; null refuses unreadable EDN.
 * Title edits preserve all unrelated bytes. No I/O or graph state.
 */
export function query_edn_json(source: string, operation: string, value: string): string;

/**
 * Query raw extents from the native reader. O(raw bytes), no parser or I/O;
 * JSON offsets are UTF-8 bytes. Unterminated candidates are omitted.
 */
export function query_macro_extents_json(raw: string): string;

/**
 * The raw reader's literal grammar for a macro name; O(name bytes), no I/O.
 */
export function query_macro_is_tql(name: string): boolean;

/**
 * Shared native target classification on an accepted AST link. O(target).
 */
export function reference_target_name(kind: string, value: string, label: string, org: boolean, filename_candidates: boolean): string | undefined;

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

/**
 * Shared comparison form; O(text bytes), no graph access.
 */
export function search_fold(text: string, remove_accents: boolean): string;

/**
 * Membership against a policy-matched pre-folded body; O(text × terms).
 */
export function search_matches(query: string, remove_accents: boolean, lower: string, original: string): boolean;

/**
 * Parse metadata for UI builders. Same grammar, errors and folds as native.
 */
export function search_query_json(query: string, remove_accents: boolean): string;

/**
 * UTF-16 search evidence, capped by limit. First mode retains zero-width hits
 * and considers all positive terms; multi-range mode uses the satisfied group.
 */
export function search_spans_json(query: string, remove_accents: boolean, text: string, limit: number, first: boolean): string;

/**
 * Bounded original UTF-16 evidence, O(text × needle scalars).
 */
export function search_substring_spans_json(text: string, needle: string, limit: number, remove_accents: boolean): string;

/**
 * Split already-parsed linkable property values with the native separator.
 * O(value bytes), without parsing or I/O. Empty members retain their position.
 */
export function split_linkable_property(value: string): string[];

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly decode_page_name: (a: number, b: number, c: number) => [number, number];
    readonly edit_block_regions_json: (a: number, b: number, c: number, d: any, e: any) => [number, number, number, number];
    readonly encode_page_name: (a: number, b: number, c: number) => [number, number];
    readonly is_query_macro_name: (a: number, b: number) => number;
    readonly logbook_apply_marker_transition: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => [number, number];
    readonly logbook_clock_in: (a: number, b: number, c: number, d: number) => [number, number];
    readonly logbook_clock_out: (a: number, b: number, c: number, d: number) => [number, number];
    readonly logbook_info_json: (a: number, b: number, c: number) => [number, number];
    readonly lsdoc_tag: () => [number, number];
    readonly mime_from_path: (a: number, b: number) => [number, number];
    readonly nested_reference_names: (a: number, b: number) => [number, number];
    readonly page_header_json: (a: number, b: number) => [number, number];
    readonly page_regions_json: (a: number, b: number, c: number) => [number, number];
    readonly parse_block_bundle_json: (a: number, b: number, c: number) => [number, number];
    readonly parse_block_json: (a: number, b: number, c: number) => [number, number];
    readonly parse_document_json: (a: number, b: number, c: number) => [number, number];
    readonly property_line_json: (a: number, b: number) => [number, number];
    readonly query_edn_json: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
    readonly query_macro_extents_json: (a: number, b: number) => [number, number];
    readonly query_macro_is_tql: (a: number, b: number) => number;
    readonly reference_target_name: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number];
    readonly render_block_html: (a: number, b: number, c: number) => [number, number];
    readonly search_fold: (a: number, b: number, c: number) => [number, number];
    readonly search_matches: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly search_query_json: (a: number, b: number, c: number) => [number, number];
    readonly search_spans_json: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number];
    readonly search_substring_spans_json: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
    readonly split_linkable_property: (a: number, b: number) => [number, number];
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __externref_drop_slice: (a: number, b: number) => void;
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
