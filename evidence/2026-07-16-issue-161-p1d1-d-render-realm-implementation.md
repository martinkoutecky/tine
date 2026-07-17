# GH #161 P1D1-D — render-realm implementation receipt

## Scope

- **Tier:** medium; approved, settled render-realm continuity contract.
- **Pinned base:** `src/transientLayers.ts` `cb08140530814899891d0d333ee8a252a20e60a3`; `vitest.render.config.ts` `c2b47ebb088922dee06d24008969ec53972f1b59`.
- **Candidate:** uncommitted working-tree patch; no commit, GitHub, deploy, release, catalog, test, Android, P1D2+, or owner-inventory changes.
- **Owned files:** `src/transientLayers.ts`, `vitest.render.config.ts`, new `src/testSetup.render.ts`, and this receipt.

## Contract implemented

Before each render test, `src/testSetup.render.ts` calls
`delegateEvents([...DelegatedEvents], document)`. `registerTransientLayer`
retains its entry-identity and root checks, but sends its event target through a
small guarded `root.contains(target as Node)` helper. Invalid or cross-realm
targets fail closed; token ordering, capture listeners, disposer safety,
dismissal, and focus restoration are otherwise unchanged.

## Causal fail-before

With the final patch temporarily removed only from its three owned source/config
paths, the two exact no-isolation commands both failed as specified:

- Seed `161`: **4 files failed, 6 tests failed, 46 passed (52)**. Failed the
  three Settings interaction tests, P1A1-F2 shortcut recording,
  P1D1-B Settings Advanced focus witness, and P1D1-C Help inside-pointer
  reactivation (`in-page-find` remained top instead of `help`).
- Seed `42`: **3 files failed, 6 tests failed, 46 passed (52)**. Failed the
  three Settings interaction tests, P1D1-B Settings Advanced focus witness,
  and P1D1-A's two concrete Settings-child activation assertions.

The approved patch was restored before all pass-after gates below.

## Pass-after

- Nine-file render command: **9 files / 52 tests passed**.
- Seed `161`, no isolation: **9 files / 52 tests passed**. Actual file order:
  `QuickSwitcher.p1c-q`, `keybindings.p1a1.drawerFocus`, `transientDispatch.p1`,
  `transientRegistry.p1d1.hierarchy`, `Settings`,
  `keybindings.p1a1.globalCapture`, `transientRegistry.p1d1.focus`,
  `Settings.p1a1.recording`, `transientRegistry.p1d1.lifecycle`.
- Seed `42`, no isolation: **9 files / 52 tests passed**. Actual file order:
  `Settings.p1a1.recording`, `transientDispatch.p1`,
  `keybindings.p1a1.globalCapture`, `transientRegistry.p1d1.lifecycle`,
  `Settings`, `transientRegistry.p1d1.focus`,
  `keybindings.p1a1.drawerFocus`, `QuickSwitcher.p1c-q`,
  `transientRegistry.p1d1.hierarchy`.
- `rtk proxy npx vitest run src/keybindings.test.ts src/mobileDrawers.test.ts`:
  **2 files / 26 tests passed**.
- `rtk npm run check:regressions`: passed (131 UI entries; 108 GitHub issues;
  both catalog inventories valid).
- `rtk git diff --check -- src/transientLayers.ts vitest.render.config.ts src/testSetup.render.ts`:
  passed.

Full typecheck, repository, real-app, deploy, release, fresh Sol/high
post-implementation approval, fresh C verification, and recombined P1D1
verification remain deferred to their stated boundaries.
