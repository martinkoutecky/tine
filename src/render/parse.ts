// In-browser block parser — synchronous, via lsdoc compiled to WebAssembly.
// ===========================================================================
// Replaces the old Tauri `parse_blocks` IPC + the async `astParse.ts` batch cache.
// `lsdoc` (the same Rust parser the backend index uses) is compiled to wasm and
// vendored under ./wasm/ by `npm run build:wasm`. The wasm exposes
// `parse_block_json(raw, is_org) -> string` — the SAME `serde_json` encoding the
// IPC path used, which `./ast.ts` mirrors 1:1 — so `JSON.parse` yields `Block[]`
// with zero re-verification.
//
// Loading: the wasm bytes are base64-inlined in ./wasm/lsdoc_wasm_bytes.ts and
// handed to the wasm-bindgen glue's async init as an explicit buffer — NO fetch,
// so it works under Tauri's custom protocol and offline. `initParser()` is awaited
// once at app boot. Main awaits it; Capture paints its seeded empty editor while
// initialization is pending, deferring structural identity reads until ready.

import { createSignal } from "solid-js";
import init, { parse_block_bundle_json, parse_inline_json, edit_block_regions_json, lsdoc_tag, __tineReinstantiate } from "./wasm/lsdoc_wasm.js";
import { WASM_B64, LSDOC_TAG } from "./wasm/lsdoc_wasm_bytes";
import type { Block, MacroInline, Inline } from "./ast";

// `ready` is a Solid signal so components (AstBody) reactively render once the
// parser is loaded. In the normal flow init is awaited before mount, so it's
// already true at first paint (no flash); the signal is the safety net for any
// path that renders before init resolves.
const [ready, setReady] = createSignal(false);
const [failed, setFailed] = createSignal(false);
let initError: unknown = null;
let initPromise: Promise<void> | null = null;

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/** Instantiate the wasm parser once (idempotent). Main awaits it before paint;
 *  Capture starts it before paint and defers structural reads until ready.
 *  Async (not `initSync`) so the vendored module compiles off the
 *  synchronous-compile size limit some engines enforce on the main thread. */
export function initParser(): Promise<void> {
  if (ready()) return Promise.resolve();
  if (!initPromise) {
    initPromise = init({ module_or_path: base64ToBytes(WASM_B64) })
      .then(() => {
        setReady(true);
        // DOM marker so verification (and a human) can confirm the wasm parser is
        // live in WebKitGTK — NOT silently masked by the fallback renderer. The
        // init-failure banner (plan §4) keys off the "failed" state too.
        if (typeof document !== "undefined") document.documentElement.dataset.lsdocParser = "ready";
        // Diagnostic only — the hard stale-wasm guard is in build-wasm.mjs.
        if (lsdoc_tag() !== LSDOC_TAG) {
          console.warn("lsdoc-wasm tag mismatch");
        }
      })
      .catch((e) => {
        initError = e;
        setFailed(true);
        if (typeof document !== "undefined") document.documentElement.dataset.lsdocParser = "failed";
        throw e;
      });
  }
  return initPromise;
}

/** True (reactive) once the wasm parser is loaded and `parseBlock` is safe to
 *  call. Read inside JSX so a component re-renders when init resolves. */
export function parserReady(): boolean {
  return ready();
}

/** The init error, if `initParser()` rejected (used to surface a visible banner
 *  rather than a silently blank app). Null while pending or on success. */
export function parserInitError(): unknown {
  return initError;
}

/** True (reactive) if the wasm parser failed to load — drives the app-level
 *  "renderer failed" banner so a failure isn't a silently degraded app. */
export function parserFailed(): boolean {
  return failed();
}

// Pure parse cache: text+format fully determine the AST (independent of graph
// state), so a plain bounded LRU suffices — no epoch invalidation needed. Bounded
// by insertion-order eviction (NOT a wholesale clear), so a working set larger
// than the cap degrades to partial hits instead of dropping to a 0% hit rate.
const cache = new Map<string, Block[]>();
const CACHE_MAX = 8000;
const quarantined = new WeakSet<Block[]>();
export type ByteRange = [number, number];
export interface RegionProperty {
  key: string; value: string; line: ByteRange; key_range: ByteRange; value_range: ByteRange; region: number; primary: boolean;
}
/** A block-level literal container lsdoc accepted; offsets are UTF-8 bytes. `close_start` begins the
 * container's last non-blank line (its closer); `delim_end` ends the opener's delimiter token. */
export interface RegionLiteralBlock {
  kind: "src" | "example" | "other"; lang: string; range: ByteRange; open_end: number; close_start: number; delim_end: number;
}
/** The editor-state policy for a fence still being typed (named policy in `block_regions.rs`). */
export interface RegionOpenFence { lang: string; start: number; open_end: number; delim_end: number }
export interface BlockRegions {
  header: {
    marker: string | null; priority: string | null; heading: number | null;
    marker_range: ByteRange | null; priority_range: ByteRange | null;
  };
  literals: ByteRange[];
  property_regions: ByteRange[];
  properties: RegionProperty[];
  planning: { kind: string; line: ByteRange; timestamp: ByteRange; date: unknown }[];
  drawers: { name: string; range: ByteRange; close: number; clocks: ByteRange[] }[];
  id: RegionProperty | null;
  quarantined: boolean;
  literal_blocks: RegionLiteralBlock[];
  open_fence: RegionOpenFence | null;
}
/** An accepted identity value carried with the exact editor buffer that owns
 * it. Coordinates are deliberately absent: inserting hidden rows moves spans. */
export interface BlockIdentityFacts { raw: string; format: "md" | "org"; value: string | null }
const soleMacroCache = new WeakMap<Block[], MacroInline | null>();
const regionCache = new WeakMap<Block[], BlockRegions>();


function remember(key: string, blocks: Block[]): Block[] {
  if (cache.size >= CACHE_MAX) cache.delete(cache.keys().next().value!);
  cache.set(key, blocks);
  return blocks;
}

function quarantine(key: string, text: string): Block[] {
  const blocks: Block[] = [{ kind: "paragraph", inline: [{ k: "plain", text }] }];
  quarantined.add(blocks);
  regionCache.set(blocks, { header: { marker: null, priority: null, heading: null, marker_range: null, priority_range: null },
    literals: [[0, new TextEncoder().encode(text).length]], property_regions: [], properties: [],
    planning: [], drawers: [], id: null, quarantined: true, literal_blocks: [], open_fence: null });
  return remember(key, blocks);
}

export function isQuarantined(blocks: Block[]): boolean {
  return quarantined.has(blocks);
}

// A/B instrumentation: counts parseBlock calls (cold misses + warm hits) so the
// lazy-body virtualization win is measurable on a large page
// (`window.__tineParseStats`). On in dev always; in a PRODUCTION build (what the
// perf bench runs via `vite preview`) it stays off unless `window.__tineBench` is
// set before boot — `import.meta.env.DEV` is a compile-time constant, so for
// normal users the whole path is dead-code-eliminated and never runs.
function statsEnabled(): boolean {
  return (
    import.meta.env.DEV ||
    (typeof window !== "undefined" && (window as unknown as { __tineBench?: boolean }).__tineBench === true)
  );
}
function bumpParseStats(hit: boolean) {
  if (typeof window === "undefined") return;
  const w = window as unknown as { __tineParseStats?: { calls: number; hits: number; misses: number } };
  const s = (w.__tineParseStats ??= { calls: 0, hits: 0, misses: 0 });
  s.calls++;
  if (hit) s.hits++;
  else s.misses++;
}

/** Parse one block body (the blockView-stripped `view.lines.join("\n")`) into
 *  lsdoc's render AST. Synchronous — `initParser()` must have resolved first. */
export function parseBlock(text: string, isOrg: boolean): Block[] {
  if (!ready()) {
    throw new Error("parseBlock called before initParser() resolved");
  }
  const key = (isOrg ? "o\n" : "m\n") + text;
  const hit = cache.get(key);
  if (hit !== undefined) {
    if (statsEnabled()) bumpParseStats(true);
    // Refresh recency: re-insert so hot entries survive eviction.
    cache.delete(key);
    cache.set(key, hit);
    return hit;
  }
  if (statsEnabled()) bumpParseStats(false);
  let json: string;
  try {
    json = parse_block_bundle_json(text, isOrg);
  } catch {
    __tineReinstantiate();
    try {
      json = parse_block_bundle_json(text, isOrg);
    } catch {
      __tineReinstantiate();
      return quarantine(key, text);
    }
  }
  const bundle = JSON.parse(json) as { blocks: Block[]; regions: BlockRegions; sole_macro: MacroInline | null };
  soleMacroCache.set(bundle.blocks, bundle.sole_macro);
  regionCache.set(bundle.blocks, bundle.regions);
  return remember(key, bundle.blocks);
}

/** Raw UTF-8 regions from the render cache: O(block bytes) on a cold miss,
 * zero additional parses on a warm AST. Never call from a typing handler. */
export function blockRegions(raw: string, format: "md" | "org" = "md"): BlockRegions {
  if (!parserReady()) throw new Error("Structural edit refused: parser is not ready");
  return regionCache.get(parseBlock(raw, format === "org"))!;
}
/** Parser-owned splice for one block. Warm regions avoid parsing; parse traps
 * refuse visibly by throwing. Callers must retain raw on refusal. */
export function editBlock(raw: string, format: "md" | "org", request: object): string {
  const regions = blockRegions(raw, format);
  if (regions.quarantined) throw new Error("Structural edit refused: block parsing is quarantined");
  return edit_block_regions_json(raw, format === "org", regions, request);
}

/** Sole visible macro from the native AST policy (standalone_macro::sole_macro).
 * O(1) on a warm block; cold parsing is O(block bytes), with no second parse. */
export function soleBlockMacro(raw: string, format: "md" | "org" = "md"): MacroInline | null {
  // Candidate admission only: without an opening token there cannot be a macro.
  // Positive classification still belongs entirely to the native AST policy.
  if (!parserReady() || !raw.includes("{{")) return null;
  return soleMacroCache.get(parseBlock(raw, format === "org")) ?? null;
}

const propertyInlineCache = new WeakMap<RegionProperty, Inline[]>();
/** Inline syntax of one accepted property value, using lsdoc's bounded inline
 * door. O(value bytes) cold, O(1) warm; coordinates start at the value's byte 0.
 * Refusal throws, preserving source rather than dropping unreadable content. */
export function propertyValueInline(property: RegionProperty, format: "md" | "org"): Inline[] {
  const cached = propertyInlineCache.get(property);
  if (cached) return cached;
  const inline = JSON.parse(parse_inline_json(property.value, format === "org")) as Inline[] | null;
  if (!inline) throw new Error("Inline export refused: property value is too deep");
  propertyInlineCache.set(property, inline);
  return inline;
}
