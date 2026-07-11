# 0041 — Outline navigation is scoped to the rendered model, not the whole page

- **Status:** Accepted
- **Date:** 2026-07-11

## Context

A routed page, its zoomed block subtree, the journals feed, and satellite block
renderings can expose different subsets of the same edit tree. Navigation used a
whole-page fallback whenever a block was absent from the feed. After zoom began
revealing a durably-collapsed root without changing `collapsed`, that fallback
could not describe the UI: it omitted rendered children and included invisible
page siblings. Shift-selection, delete/cut, arrows, and Enter could therefore act
outside the visible zoom.

`SurfaceContext` already distinguishes rendered instances for editor focus, but
focus ownership and outline membership are different concerns.

## Decision

Model the rendered outline boundary explicitly with `OutlineScope`:

- roots define the only addressable subtree;
- `forceExpandedRoot` provides the zoom root's one-level view override while
  descendants continue to honor their durable collapse;
- a model traversal, never the DOM, derives visible order and still treats sheet
  views as opaque;
- `OutlineScopeContext` carries the scope to block editors and mouse gestures;
- block-selection retains the initiating scope so global key handling resolves
  selection, copy/cut/delete, and movement against the same order;
- ordinary feed/routed/capture behavior keeps the existing unscoped traversal.

At a zoom root, Enter creates a child that remains mounted. The caret-at-start
case keeps Tine's block-identity invariant (the root retains its content/UUID and
gets a new empty child), rather than creating an invisible page sibling.

## Consequences

- Commands issued in a zoom cannot address invisible page siblings.
- View expansion remains separate from on-disk `collapsed` metadata.
- Multi-pane instances may navigate different scopes without consulting DOM
  layout or a process-global zoom signal.
- New outline surfaces that render a subset or override visibility must provide an
  `OutlineScope`; focus-only surfaces continue to use `SurfaceContext` separately.
