# 0008. Lazy block-body rendering (render-once-keep), not viewport windowing

- **Status:** Accepted
- **Date:** 2026-06-30

## Context

Since the in-browser WASM parse cutover ([0006](0006-in-browser-wasm-parsing.md)),
every block parses its body **synchronously at render time** (`parseBlock` →
`renderBlocks`, the AST→DOM build), in the single chokepoint `AstBody`
(`src/render/body.tsx`). That is great for normal pages, but on a *large* page
(hundreds–thousands of blocks) it means a synchronous parse + inline-render for
**every** block up front, on load. Performance is Tine's reason to exist (it targets
older machines than the author's i7-8565U), so this off-screen work is the standing
**P1** performance item.

Two mechanisms were on the table:

1. **Lazy body** — keep every `Block` mounted (tree, bullets, geometry intact) but
   defer the *body* parse/render until the block nears the viewport.
2. **True windowing** — unmount off-screen blocks entirely (smaller DOM), with
   measured-height spacers.

The dominant constraint is **scroll-height stability**: `content-visibility: auto`
on the feed was shipped and then **reverted** (commit `e2cdfc7`) because estimating
off-screen heights and snapping them to real height *churns* the total `scrollHeight`
during scroll, which makes WebKitGTK's auto-hiding overlay scrollbar flicker. Any
scheme that re-sizes seen content as you scroll reintroduces that flicker.

True windowing also concentrates real hazards: caret loss on unmount, drag-drop
across unmounted regions, scroll-anchor jumps, Lenis coupling, and it forecloses
native Ctrl+F. Its *extra* win over lazy body is DOM **node count** — a second-order
cost relative to the named *parse-on-load* cost, and only material on a single huge
page.

## Decision

We will defer the **body** parse + render of each block until it is within ~1200px
of the viewport (a shared singleton `IntersectionObserver`, `src/lazyObserve.ts`,
reused from `LiveRefGroup`), and we will **keep it rendered once rendered**
(`renderedBlocks` latch). Off-screen blocks show their raw text in the existing
`.ast-fallback` span — a good height proxy — until they come near. We will **not**
window/unmount blocks for P1; true windowing is deferred and measurement-gated.

The block shell stays mounted, so collapse/zoom, deep-link `scrollIntoView`, the
editor, and drag-drop are untouched. The edited block is latched on `startEditing`,
and deep-link targets are latched in `openPageAtBlock`, so neither shows a placeholder
frame.

## Consequences

- **The win:** on a 2000-block page, ~11 blocks render on load instead of 2000 (the
  rest are deferred raw-text placeholders) — the synchronous parse + AST→DOM cost is
  paid only for what's near the viewport, then incrementally on scroll.
- **Zero re-entry churn by construction.** Render-once-keep means there is no second
  placeholder↔real transition, so scrolling back over seen content never changes
  `scrollHeight` — this is what keeps us clear of the `e2cdfc7` scrollbar-flicker trap.
  We deliberately add **no** `content-visibility`/`contain` CSS.
- **Output is byte-identical** once on-screen (the gate is *when* we parse, not
  *what*) — OG parity and the render test suite are unaffected.
- **Costs accepted:** (a) a one-time first-render-in for tall constructs (headings,
  display math, media) can shift layout slightly; `estimateBodyReserve` reserves an
  approximate `min-height` to shrink it. (b) Native browser Ctrl+F finds only rendered
  (near-viewport) text — Tine's own search (Ctrl+K, `((`) uses the backend index and
  is unaffected. (c) DOM node count is **not** reduced — if a single huge page is still
  heavy after this, the next lever is true windowing (feed-day first), gated on
  measurement.
- A dev-only `window.__tineParseStats` counter (stripped from production) makes the
  parse-saved measurable; `scripts/shot-virtualize.mjs` is the regression harness.

## 2026-10-22 amendment: cache ownership

L10 measured historical facet seeds, large parse versions, and completed block
answers outliving their consumers. Backend seeds now follow loaded-node ownership;
historical parses and inactive block answers have explicit byte/admission limits,
and resets clear graph text eagerly. Weak parse reuse and live reference consumers
keep current content warm. See [frontend memory contract](../contracts/frontend-memory.md).
The render-once-keep decision still applies. Journal-day retirement (L10-03) is a
separate decision because of its Find/Replace and scroll-geometry consequences.
