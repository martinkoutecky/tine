# 0010. lsdoc owns the canonical HTML skeleton; the export consumes it, the frontend conforms

- **Status:** Accepted
- **Date:** 2026-06-30

## Context

[ADR 0005](0005-lsdoc-separate-parser-crate.md)/[0006](0006-in-browser-wasm-parsing.md)
made `lsdoc` the one parser for block bodies, and [0009](0009-one-block-facet-source-on-the-dto.md)
made its parse the one source for header *facets*. But two **renderers** still turned
that AST into HTML independently:

- the **frontend** (`src/render/*.tsx`) — reactive SolidJS that resolves refs, assets,
  KaTeX and macros into interactive DOM; and
- the **static HTML export** (`crates/tine-core/src/publish.rs`) — which did **not**
  use lsdoc at all. It carried its own hand-rolled `render_inline` (a second Markdown
  inline parser) plus a `strip_inline_markup` search-text stripper. Two consequences:
  the export silently diverged from the app (it rendered fenced code, tables, callouts
  and block-math as raw text or per-line `<div>`s — the ``` fences leaked verbatim),
  and `render_inline`/`strip_inline_markup` were exactly the duplicate-scanner pattern
  the project `CLAUDE.md` forbids ("one parser per grammar").

lsdoc v0.2.2 ships `render_html(&[Block], opts) -> String`: a **canonical structural
skeleton** (tags + classes + nesting) with a `data-*` hook carrying the RAW input for
every consumer-dependent concern (`data-page`/`data-block` for refs, `data-asset`,
`data-tex`, `data-lang`, `data-macro`, `data-align`, `data-raw`). lsdoc owns structure +
classes + escaping + timestamp formatting; it never resolves a ref/asset/macro and never
emits live HTML. The options were: **(A)** both renderers consume lsdoc's HTML string
(rip out the frontend's reactive renderer); **(B)** lsdoc owns the skeleton, the export
consumes `render_html` + decorates the hooks, and the frontend stays reactive but is
**gated** to render the identical skeleton.

## Decision

We will adopt **(B) — Option C2**: lsdoc's `render_html` is the canonical skeleton; the
two renderers conform to it.

- **Export:** `publish.rs` renders each block via `lsdoc::render_html`, then a single
  O(n) `decorate` pass resolves the `data-*` hooks for a static page (page/tag ref →
  `slug.html` link, block ref → in-page anchor, `data-tex` → KaTeX `\(..\)`/`\[..\]`,
  `data-asset` → `src`, `data-lang` → a highlight.js `language-X` class, macros dropped,
  raw HTML escaped). The duplicate `render_inline` **and** `strip_inline_markup` are
  deleted; the search index now flattens the same AST (`ast_plain_text`).
- **Frontend:** unchanged in spirit (reactive), but its skeleton must match lsdoc's —
  the def-list term (`md-list-term`) was added; page-ref brackets, timestamp ranges and
  callouts already matched.
- **Anti-drift gate:** `src/render/skeleton-drift.test.tsx` renders fixtures both ways
  (lsdoc `render_html` via a new wasm `render_block_html`, and the frontend `renderBlocks`
  into jsdom), normalizes away the legitimate differences (lsdoc's `data-*` + escaped
  text; the frontend's handlers, `.bracket` spans, interactive chrome), and asserts the
  skeletons are identical. It runs in CI (`npm test`).

The frontend is **not** rewritten to consume lsdoc's HTML string — that (deleting its own
skeleton-building to read lsdoc's `data-*`) is a separate, measured, later step.

## Consequences

- The export and the app can no longer silently diverge: a structural change in one that
  isn't mirrored in the other fails the gate. The export gains correct code blocks
  (highlight.js), tables (with `data-align`), callouts, and inline/block math for free —
  all previously broken.
- One fewer Markdown parser in the tree. `render_inline`/`strip_inline_markup` are gone;
  the export's non-fence-aware `is_meta_line` bug (a `SCHEDULED:` inside a code fence was
  dropped) is fixed by construction, since filtering is now at the AST-block level.
- New surface to keep stable: the `render_html` skeleton (class names + `data-*`) is now a
  contract two consumers depend on. The gate is what enforces it; lsdoc bumps must keep it
  (or update both sides + the gate together). The vendored wasm grows ~54 KB
  (`render_html` + its helpers).
- The export pulls highlight.js + KaTeX from a CDN at view time (offline → plain code /
  raw TeX, never broken). Assets are still referenced by path, not copied into `publish/`
  (a pre-existing limitation, unchanged). Macros render as nothing in a static page.
