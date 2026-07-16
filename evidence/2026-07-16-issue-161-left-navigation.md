# GH #161 N1 — successful left-navigation completion

## Contract and boundary

- Checklist row: **N1 only** from the persistent-manager checklist.
- Base commit: `f6f6de13e1180f61ce8d54267d164ffe994eeae7`; the worktree already contained
  an uncommitted partial N1 implementation plus other #161 work and was
  preserved.
- Active in-window Journals, Favorite, Recent, path-backed All Pages,
  Namespace, and successful in-place graph outcomes dismiss the compact left
  drawer and focus the active main content. Same-page navigation counts.
- Disclosures, `+more`/search, Shift/right-sidebar, middle/new-tab, context
  menu, peer-window graph, picker cancel, `aborted`, `focused_existing`, and
  rejected graph operations do not report active left navigation. At 640+ the
  same completion callback is a no-op for the persistent sidebar and focus.
- Excluded: W2/S1/S2, right-sidebar teardown, Android, native/browser release
  acceptance, commits, deployment, release, and GitHub communication.

## Implementation

1. `src/ui.ts` now owns `completeActiveLeftNavigation()`, the single N1
   close-plus-navigation-focus boundary. It acts only when the active compact
   drawer is the left drawer.
2. `src/App.tsx` passes that production boundary directly to `Sidebar` instead
   of duplicating close/focus sequencing inline.
3. `src/components/Sidebar.tsx` retains the already-present explicit
   `onActiveNavigationComplete` boundary and graph outcome plumbing, exposes a
   typed `GraphNavigationActions` seam for mounted testing, captures the graph
   click modifier before awaiting, and handles failed Open/New graph promises
   without completing navigation or producing an unhandled rejection.
4. `src/components/Sidebar.p4.navigation.test.tsx` mounts the real Sidebar,
   Namespace tree, router/UI state, page resource, graph menu, and production
   completion callback. One table-like flow proves the full positive/negative
   matrix, route targets, drawer state, and focus result.
5. `tests/ui-regressions/catalog.json` names the mounted N1 regression under
   `UI-MOBILE-SIDEBAR-DRAWER-001`. The umbrella entry remains honestly
   `reproduced` pending the remaining manager rows and native acceptance.

## Necessity and evidence currency

This bounded task began after an earlier workstream had already added the
partial callbacks and graph outcome return types, so no destructive rollback
was performed to manufacture a second fail-before in the shared dirty tree.
The historical umbrella fail-before remains the catalog's causal proof. The
missing manager proof was real: there was no mounted positive/negative
navigation matrix, no focus assertion, and Open/New graph rejections were not
handled at their component boundary.

Focused pass-after evidence on the current moving-tree snapshot:

- `rtk proxy npx vitest run --config vitest.render.config.ts src/components/Sidebar.p4.navigation.test.tsx src/components/Sidebar.test.tsx src/mobileDrawers.shell.test.tsx --reporter=verbose`
  — **3 files, 5 tests passed**.
- `rtk proxy npx vitest run src/components/Sidebar.graphSwitcher.test.ts src/components/Namespace.test.ts src/mobileDrawers.test.ts --reporter=verbose`
  — **3 files, 14 tests passed**.
- `rtk proxy npx tsc --noEmit` — **passed with zero errors**.
- `rtk npm run check:regressions` — **passed; 137 UI entries / 108 GitHub
  issues, native guards and both indexes valid**.
- Scoped `git diff --check` — **passed**.

Pinned inspected hashes after the focused run:

```text
02f83eee9c9f5ff2f72a2aec51f87ffd0da19c077c24d2587383cf32c7965b9f  src/App.tsx
cf005769148e2d86b706ea768e207dc9cd38c888be42c7460cae92a857620c57  src/ui.ts
070dc95c86353f9a3ab48aac4a812b3063e590d36bfc62bfa4707fc7cd0371de  src/components/Sidebar.tsx
541c067960378de7a60fa65b2f7f714be39b1f91c97e4066775b670ce712cede  src/components/Namespace.tsx
02360876121536948a5f3f11a861da044da7e8468cd5c72378cdfca69f691c38  src/graph.ts
e1ec069d41be3fd3a9c9a828fad93b9fb094c2cbd370417f01938339d83921c7  src/components/Sidebar.p4.navigation.test.tsx
4999dc008e42873a60cc4dcc4d0092f40f628284c54be54a262ae3f7d2c1ee1e  tests/ui-regressions/catalog.json
```

## Verdict and residual risk

**N1 is implementation-complete on this snapshot.** The mounted matrix drives
real Sidebar/Namespace DOM and real router/UI/focus behavior; graph filesystem
operations are deliberately injected only at the typed awaited outcome seam.
It therefore does not replace E2's later real Tauri graph-load/native drawer
scenario, nor does it certify the moving dirty worktree as the final immutable
candidate. No commit was created.
