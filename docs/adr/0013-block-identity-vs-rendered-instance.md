# 0013. One block, many rendered instances: focus/caret ownership rules

Date: 2026-07-02

## Status

Accepted

## Context

One block uuid can render live in several places at once — main feed, journal
agenda, sidebar, `{{query}}`/`{{embed}}` results, linked refs. "Which instance
owns the editor, and where does the caret land" caused the repo's clearest
fix→revert→refix chain (`ae8d4bb` → `171a173` → `00d244f`) and makes `Block.tsx`
the #1 churn file. The coordination protocol existed only implicitly.

## Decision

Block identity (uuid) and rendered-instance identity (surface) are distinct,
and the store's five signals coordinate them with these invariants:

- Every distinct live-rendering context has its own **`SurfaceContext`**
  (`Block.tsx`); `LiveRefGroup` is the precedent. **A new surface that renders
  live blocks MUST create its own SurfaceContext** — reusing `"main"` re-opens
  focus stealing by duplicate instances.
- `editingId` + `editingOwner` say which uuid is being edited and by which
  instance. `owner = null` means *unscoped* (keyboard nav): any instance may
  edit, and the caret is pinned to the surface that had it via the
  `pendingFocusSurface` stamp (`focusSurfaceFor`). A non-null owner (a click)
  means exactly one instance mounts the editor.
- `caretTarget` (`takeCaretFor`) carries the caret position — an offset, or an
  OG-style `{col, edge}` goal-column for cross-block Up/Down.
- **`startEditing` is the only entry point** that sets these signals, and it
  sets them atomically (`batch`) — setting them individually reintroduces the
  one-flush intermediate window that dropped focus in WebKitGTK.

## Consequences

- Exactly one instance focuses per edit; asserting that (a headless two-surface
  invariant test) is the missing guard this ADR calls for.
- Longer-term, an extracted `editorController` as the sole writer of these
  signals would turn the convention into structure.

## Status update (2026-07-02, later the same day)

Both consequences are now built: `npm run e2e:caret` is the two-surface
invariant guard (release-required), and `src/editorController.ts` is the sole
writer — the raw setters are module-private, every caller goes through the
intent API (`startEditing` / `endEdit(reason)` / `noteSurfaceFocused`), and
`src/editorController.contract.test.ts` fails CI if any other file under `src/`
references a raw setter again.
