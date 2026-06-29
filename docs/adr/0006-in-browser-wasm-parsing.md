# 0006. Parse in the browser via `lsdoc` compiled to WASM (vendored)

- **Status:** Accepted
- **Date:** 2026-06-29

## Context

The block renderer originally parsed block bodies through Tauri IPC
(`parse_blocks` → `tine-core` → `lsdoc`). Because IPC is async, the renderer wrapped
it in a resource and fell back to a **legacy TypeScript renderer** for the sub-frame
before the AST arrived — and *permanently* in the mock/dev harness, which has no Rust
backend. That fallback was the sole reason ~1,300 lines of legacy TS renderer
survived, and it produced a first-paint flash wherever legacy ≠ AST. `lsdoc` is
`serde`-only and verified WASM-safe.

## Decision

We will compile `lsdoc` to **WebAssembly** (a small `crates/lsdoc-wasm` wrapper,
`cdylib`) and parse **synchronously in the browser**, giving one parser everywhere:
Rust `lsdoc` for the backend index, `lsdoc`-WASM for the frontend render. Specifics:
**vendor a prebuilt, base64-inlined `.wasm`** committed under `src/render/wasm/` and
instantiate it from bytes (no `fetch`, no CI wasm build); the WASM returns a **JSON
string** the frontend `JSON.parse`s, matching the existing IPC encoding exactly. The
pinned `lsdoc` tag is **stamped into the bytes module and asserted at boot** against
`tine-core`'s pin.

## Consequences

- **Easier:** no async boundary, no first-paint flash, identical parse in the mock
  harness — which let the entire legacy render path be deleted (the payoff). One
  parser, two compile targets.
- **Harder:** regenerating the vendored WASM (`npm run build:wasm`, needs `wasm-pack`)
  is a required step on every `lsdoc` bump; forgetting it silently renders with the
  old parser (hence the boot-time version assert). One-time `initParser()` cost
  before first paint in both windows.
- **Rejected alternatives:** `serde-wasm-bindgen` (different encoding to re-verify),
  `vite-plugin-wasm` + TLA (custom-protocol/MIME risk under WebKitGTK — kept only as
  a fallback), and building WASM in the 3-OS release CI (would force `wasm-pack`
  onto every runner).
