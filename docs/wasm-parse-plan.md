# Plan: parse in-browser via lsdoc-WASM (option A)

**Status: SHIPPED (2026-06-29).** Implemented as planned with these concrete
choices: vendor a prebuilt `.wasm` (zero CI change); a `crates/lsdoc-wasm`
wrapper crate that git-deps `lsdoc` directly (excluded from the workspace);
base64-inline the bytes + async `WebAssembly.instantiate` (no `fetch`, no Vite
wasm plugins); the wasm returns a JSON string that JS `JSON.parse`s (identical to
the old IPC `serde_json` encoding). Regenerate with `npm run build:wasm`.

What shipped vs the original deletion list: the legacy block renderers
(`segmentBody`/`BodyContent`/`parseList`/`MdList`/`BodySeg`), `parseInline.ts`(+test),
`astParse.ts`, `renderSeg`/`renderSegs`, and the IPC `parse_block`/`parse_blocks`
(+ backend/mock bindings) were all deleted (~1,300 lines). `InlineText` was NOT
deleted — it has live non-block callers (property values, breadcrumbs, ref
previews, PDF-annotation lines) and was **reimplemented** on the wasm parser.
Verified: a 99-block wasm-vs-Rust diff oracle (0 mismatches), the real-app
WebKitGTK screenshots (md + org, `data-lsdoc-parser="ready"`, no flash), 264 unit
+ 20 render tests (the render suite now also smoke-tests wasm init in Node).

---

_Original plan (for reference):_

## Goal

Replace the Tauri `parse_blocks` IPC with **synchronous in-browser parsing** by
compiling `lsdoc` to WebAssembly. Payoff:

- **Synchronous render** — no `createResource` per block, no async boundary →
  **no first-paint flash, no fallback.**
- **Mock harness parses identically** (no precompute, no special-casing).
- **Delete the entire legacy render path** (the only reason it still exists is to
  be the fallback): `parseInline.ts` + `parseInline.test.ts`, `renderSeg` /
  `InlineText` / `Seg` (inline.tsx), `segmentBody` / `BodyContent` / `parseList` /
  legacy `MdList` (body.tsx), `astParse.ts` (the async batch cache), and the
  `parse_block`/`parse_blocks` Tauri commands + their backend.ts/mock.ts bindings.
- **One parser everywhere:** Rust `lsdoc` for the backend index (tine-core,
  unchanged); `lsdoc`-WASM for the frontend render.

## Why now (after the IPC cutover, not instead of it)

The IPC cutover (shipped: dd913ca…3f8debe) proved the AST renderer is correct +
OG-faithful in the real app. Its only wart is the async parse boundary, which
forces a fallback (the legacy `parseInline` path) for the sub-frame before the
parse lands and for the mock (which can't run Rust). WASM removes the boundary,
which lets the whole legacy path be deleted. Doing it as a deliberate, planned
effort (new build toolchain) — not rushed at the tail of the render batch.

## Design

### 1. lsdoc-WASM wrapper (coordinate with the lsdoc session via FOR-TINE)
- `lsdoc` is `serde` + `serde_json` only → wasm-bindgen-friendly.
- **Recommend a thin wrapper crate** `crates/lsdoc-wasm/` in the Tine repo that
  git-deps `lsdoc` and exposes `#[wasm_bindgen] pub fn parse(raw: &str, fmt:
  &str) -> JsValue` (and maybe `refs`). Keeps `lsdoc` a pure lib; the WASM concern
  lives with Tine. (Alt: a `wasm` feature on lsdoc itself — one fewer crate but
  couples lsdoc to wasm-bindgen.)
- Use **`serde-wasm-bindgen`** to return a JS object directly (no JSON string
  round-trip) matching `src/render/ast.ts` 1:1 (same serde encoding it already
  mirrors).

### 2. Build pipeline (the main new machinery — decide here)
- `wasm-pack build --target web` (or `bundler`) → `.wasm` + JS glue.
- Vite: `vite-plugin-wasm` + `vite-plugin-top-level-await` (wasm-bindgen glue uses
  TLA), or `--target web` + manual `init()`.
- **Build-vs-vendor decision:**
  - *Build from source* (npm prebuild → wasm-pack): needs `wasm-pack` + the
    `wasm32-unknown-unknown` target in **CI** (the GitHub Actions release
    workflow for Linux/Win/Mac) **and** locally. Cleanest source-of-truth.
  - *Vendor a prebuilt `.wasm`* committed as an asset, regenerated on each lsdoc
    bump. Simpler CI, but a committed binary + regen discipline.
  - Lean *build-from-source* if CI can host wasm-pack; else vendor.
- **Bundle size:** lsdoc + serde + the 339-entry entity table → est. ~150–400 KB
  wasm (~60–150 KB gzip). Load **eagerly at boot** (lazy-load would reintroduce
  async). Measure; only split if it hurts startup.

### 3. Frontend integration
- New `src/render/parse.ts`: `initParser(): Promise<void>` (instantiate the wasm
  once at app boot) + `parseBlock(text, isOrg): Block[]` (sync, post-init).
- App bootstrap (`main.tsx`/`capture.tsx`): `await initParser()` before mount —
  one-time (~tens of ms), both real app + mock.
- `AstBody`: render synchronously from `parseBlock(view.lines.join("\n"), isOrg)`
  — drop `createResource` + the `BodyContent` fallback. Optional plain-Map memo
  keyed by text for re-render cheapness.
- On block edit: re-parse synchronously (no IPC, no cache-invalidation dance).

### 4. Deletions (the payoff)
- `src/render/parseInline.ts`, `parseInline.test.ts`.
- `inline.tsx`: `renderSeg`, `InlineText`, the `Seg` type + `parseInline` import.
- `body.tsx`: `segmentBody`, `BodyContent`, `parseList`, legacy `MdList`, `BodySeg`.
- `src/render/astParse.ts`.
- `src-tauri` `parse_block`/`parse_blocks` + `backend.ts`/`mock.ts` `parseBlocks`.
- **Keep:** `blockView` (block.ts) — header extraction (marker/priority/scheduled/
  properties → `view.lines`) is app-layer, still needed. `tine_core::render::{
  parse_block, block_refs}` — the Rust **index** path (backlinks/search), unchanged.

## Risks / open decisions
- **wasm-pack in CI** — the 3-OS release workflow must run wasm-pack or consume a
  vendored wasm. (Build-vs-vendor decision above.)
- **jsdom render tests are SAFE** — `astRender.test.tsx` feeds hand-built ASTs to
  `renderInlines`/`renderBlocks` directly; it never calls the parser, so it's
  unaffected by the WASM switch (keep it as the renderer's unit gate).
- **WebKitGTK wasm** — supported, but verify on the real app (`shot-ast-render`).
- **App-boot await** — a one-time ~tens-of-ms wasm init before first paint;
  acceptable (brief splash/empty), and it's once, not per block.
- **Output shape must match `ast.ts`** — guaranteed (same serde); add a tiny
  parity check (parse a known input via wasm AND via the Rust `parse_block`
  command during transition, diff) before deleting the IPC path.

## Verification
- Render unit tests (jsdom, hand-built ASTs) — unaffected, must stay green.
- Real-app screenshots (`scripts/shot-ast-render.mjs` Kitchen + Korg) — parity
  with the IPC cutover; confirm **no flash** (sync).
- Bundle-size check; CI build (all 3 OSes) green.

## Sequencing
1. `lsdoc-wasm` wrapper + wasm-pack build → `parse(raw,fmt)` callable from a
   throwaway HTML page.
2. Vite integration + `initParser`/`parseBlock` + app-boot await.
3. Switch `AstBody` to sync parse; screenshot-verify parity (keep the IPC path as
   a transient diff oracle).
4. Delete the legacy path + the IPC parse path.
5. Verify: unit tests, screenshots, real app, bundle size, CI.
