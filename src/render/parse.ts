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
// once at app boot (main.tsx + capture.tsx) before the first render.

import { createSignal } from "solid-js";
import init, { parse_block_json, lsdoc_tag } from "./wasm/lsdoc_wasm.js";
import { WASM_B64, LSDOC_TAG } from "./wasm/lsdoc_wasm_bytes";
import type { Block } from "./ast";

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

/** Instantiate the wasm parser once (idempotent). Awaited before first paint in
 *  every window. Async (not `initSync`) so the ~189 KB module compiles off the
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
          console.warn(`lsdoc-wasm tag mismatch: wasm=${lsdoc_tag()} bytes=${LSDOC_TAG}`);
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
// state), so a plain bounded Map suffices — no epoch invalidation needed.
const cache = new Map<string, Block[]>();

/** Parse one block body (the blockView-stripped `view.lines.join("\n")`) into
 *  lsdoc's render AST. Synchronous — `initParser()` must have resolved first. */
export function parseBlock(text: string, isOrg: boolean): Block[] {
  if (!ready()) {
    throw new Error("parseBlock called before initParser() resolved");
  }
  const key = (isOrg ? "o\n" : "m\n") + text;
  const hit = cache.get(key);
  if (hit) return hit;
  if (cache.size > 8000) cache.clear();
  const blocks = JSON.parse(parse_block_json(text, isOrg)) as Block[];
  cache.set(key, blocks);
  return blocks;
}
