# 0032. Split view: pane-router factory + focused-pane shims, single feed pane

- **Status:** Accepted (Claude, per the sheets-branch mandate — Martin to review)
- **Date:** 2026-07-08

## Context

Split view (breadth-grid-spec §10 v3, requested by Martin Jul 8 2026 "with the
grid nav model") needs N panes, each a full navigable editor with its own
tabs/history/scroll. Today `src/router.ts` is a module singleton with ~60
external call sites, `doc.feed` is "what the main area shows" (a store
singleton clobbered on every navigation, which also ends editing globally),
and scroll/find/watcher logic hardcodes one `.main-content`. Three
decompositions were mapped (`subagent-tasks/notes/split-view-architecture.md`
§7): (A) per-pane router factory + PaneContext, with the existing exports kept
as shims delegating to the focused pane; (B) one router owning a pane tree
with `paneId` parameters everywhere; (C) a hardcoded second pane grown from
the sidebar pattern, deferring the tree. Load-bearing fact: `installPaneTracker`
updates pane focus on capture-phase pointerdown/focusin BEFORE click handlers
run, so "focused pane" already equals "the pane you clicked in" for every
pointer- and keyboard-initiated navigation.

## Decision

We will take shape (A): `createPaneRouter()` per pane, a pane split-tree
(binary: split{dir, ratio} | leaf{paneId}) + `focusedPaneId` registry, a
Solid `PaneContext` for content, and the existing router exports retained as
focused-pane shims so the ~40+ content call sites stay unchanged.
Non-interactive callers (file watcher, carry, delete-fallback, graph switch,
session restore) must take explicit pane handles — never the shims.
The journals feed stays a singleton owned by at most ONE pane (the feed
pane); all other panes are single-page surfaces loaded via the satellite
`ensurePageLoaded` path, sidestepping `doc.feed` decoupling in v1. One caret
app-wide stays; panes are `SurfaceContext` surfaces (`pane:{id}`). Layout
persists inside `tine-session.json` (atomic-rename backend path), never
localStorage, with back-compat parsing both directions.

## Consequences

- Content code (link clicks, breadcrumbs, query results) needs no per-pane
  threading; the blast radius concentrates in `router.ts`, `App.tsx`,
  `Page.tsx`, `TabBar.tsx`, `ui.ts`, and the store's eviction pins.
- B was rejected because deep content can't name its pane without prop-drilling
  ~10 render files — in practice it converges on A with more coupling. C was
  rejected because it builds a second architecture to migrate later and cannot
  host the seam/grid nav model Martin explicitly asked for.
- The shim correctness rests on the pointerdown-before-click ordering; WebKitGTK
  `auxclick` ordering must be probe-verified in S1 before anything builds on it.
- Two journals feeds are impossible until someone does the real `doc.feed`
  decoupling (deliberate v2 cut, recorded in the spec's deferred list).
- Every new drag gesture (seam resize, tab drag) is pointer-based; the TabBar's
  legacy HTML5 DnD gets replaced, not extended.
