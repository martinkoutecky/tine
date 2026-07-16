# GH #161 manager row R1 — right-sidebar close safety

Date: 2026-07-16  
Worktree: `/aux/koutecky/logseq/tine-agent-worktrees/batch-reference-and-polish-v0510`  
Scope: manager checklist row R1 only  
Status: implemented and focused verification green; no commit created

## Causal boundary

`src/ui.ts` now owns one pre-unmount route for a mounted right sidebar:

- the raw Solid visibility setter is private, so session application cannot
  bypass the close lifecycle;
- a state transition is a no-op when the requested visibility already matches,
  making repeated closes idempotent;
- a real close invokes the callback registered by the mounted
  `RightSidebar`, whose existing production callback blurs the active real
  `Block` textarea and ends the matching editor-controller surface before the
  visibility signal can remove it;
- session item replacement prepares the old mounted collection before writing
  new items, and then uses the same visibility boundary without scheduling a
  restore/save loop;
- clearing the whole collection through `setRightSidebar([])` prepares mounted
  surfaces before removal. This is the exact collection seam used by the graph
  transition and by close-all;
- a reentrancy guard prevents a preparation callback from recursively entering
  itself. A second sequential preparation is harmless because the first pass
  has already removed the active edit owner.

No content write occurs merely from entering and closing an unchanged editor:
the real `Block` commit path detects identical editor text, the page remains
clean, `flushAll()` performs no `savePage`, and a repeated safe close returns
`false`.

## Mounted proof matrix

`src/components/RightSidebar.r1.closeSafety.test.tsx` mounts the production
`RightSidebar`, its real `SurfaceContext` and `Block` editor, and the production
mobile drawer controller. It edits through the rendered textarea and drives:

- desktop explicit right close;
- the toolbar toggle handler;
- mobile explicit close;
- the real mounted scrim;
- plain Escape through the global capture dispatcher;
- the Android Back call seam (`dismissMobileDrawer("back")`), without changing
  the separately owned native callback;
- compact left-drawer exclusivity replacement;
- session application;
- graph-transition collection clearing;
- the public visibility setter (the former raw-signal bypass class).

Every row proves the textarea is removed only after `editingId()` is cleared
and its pending text is present in the shared document. The same mounted suite
also proves:

- completion consumes the first Escape and leaves the drawer/editor alive;
  the second plain Escape closes safely;
- both `isComposing` and legacy `keyCode === 229` Escape leave completion,
  drawer, editing owner, and content untouched;
- unchanged-editor close plus a repeated close does not dirty, rewrite, or
  save the page.

The persistence row uses the established frontend persistence boundary:
after a real toolbar close, `flushAll()` calls `savePage` with the current
`PageDto`; that exact DTO is written to a real temporary file, the store is
reset, the file is read back, and `loadSingle()` restores the edited block.
This proves the pending right-sidebar edit crosses an actual file save/reload,
not merely an in-memory assertion or source-string proxy.

## Files owned by this R1 pass

- `src/ui.ts` — private visibility signal, unified idempotent preparation,
  session ordering, and whole-collection pre-unmount handling.
- `src/components/RightSidebar.r1.closeSafety.test.tsx` — mounted close,
  ordering, no-edit, idempotence, and file persistence proof.
- `evidence/2026-07-16-issue-161-right-close-safety.md` — this receipt.

Concurrent shared-tree changes in `src/components/RightSidebar.tsx`,
`src/components/Block.tsx`, mobile/transient files, and other #161 rows were
preserved and are not claimed here. This pass did not touch W2/S1/S2, N1, A1,
T1/T2, or E1/E2 surfaces and did not edit a regression catalog.

## Verification

```text
rtk vitest run --config vitest.render.config.ts \
  src/components/RightSidebar.r1.closeSafety.test.tsx
=> 14 passed, 0 failed

rtk vitest run --config vitest.render.config.ts \
  src/components/RightSidebar.test.tsx \
  src/components/RightSidebar.r1.closeSafety.test.tsx \
  src/keybindings.p1a1.globalCapture.test.tsx \
  src/mobileDrawers.shell.test.tsx
=> 25 passed, 0 failed

rtk vitest run src/session.test.ts
=> 7 passed, 0 failed

rtk vitest run --config vitest.render.config.ts src/graph.test.tsx
=> 4 passed, 0 failed

rtk tsc --noEmit --pretty false
=> passed, no diagnostics
```

## Residual risk

The mounted proof runs in jsdom, so it establishes event/lifecycle ordering but
does not reproduce WebKit focus quirks. The persistence row writes and reloads
the exact DTO delivered to the backend boundary on a real filesystem, but does
not re-test the Rust Markdown serializer; no serializer code changed in R1.
Native Android callback registration and WebKit/native runs remain with their
explicitly separate A1/E2 owners.
