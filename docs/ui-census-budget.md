# UI census budgets

The 2026-10 UI census counted every surface a user sees: palette commands,
context-menu rows, Settings controls visible at open, controls at rest. The cut
lanes landed; this test keeps the counts from creeping back.

`src/uiCensus.budget.test.tsx` (desktop) and `src/uiCensus.mobile.budget.test.tsx`
(Android build) measure each surface from its source of truth in code, never from
pixels, and compare it with the frozen table `SURFACES` in
`scripts/lib/ui-census-budget.mjs`. Both run under `npm test` (render config).

## What is counted, and from where

- **Commands** (registries, nothing rendered): `commandDefaults()` (Shortcuts
  list), its editor-scope subset, `paletteCommands()` minus `plugin:` ids (the
  `paletteHidden` flag decides membership), `BUILTIN_KEYS`, the slash menu
  (`COMMANDS` in `src/editor/autocomplete.ts`) and the `<` menu
  (`ADVANCED_BLOCK_COMMANDS`).
- **Settings**: `<Settings/>` mounted in jsdom. Per tab, the inputs, selects,
  buttons and summaries in the tab body at open, then the extra controls after the
  tab's Advanced toggle. Search results, shortcut rows and spellcheck dictionary
  rows are excluded (data, not chrome).
- **Context menus**: the real `ContextMenu`, `QueryActions`, `RightSidebar` and
  topbar overflow menu, mounted with a fixture that makes every conditional row
  present (the test asserts the fixture reached them). Top-level rows are counted
  apart from submenu entries. The phone/tablet variants run in the mobile file.
- **Chrome at rest**: the whole `<App/>` mounted: topbar controls (plus the three
  frameless window controls), left sidebar (empty graph and with Favorites, Recent
  and Namespaces), right sidebar header and item head, the mobile keyboard toolbar
  and bottom navigation row.

## Failure modes

- A count above its budget fails, naming the surface, the count, the budget, and
  how to raise it.
- A count of 0, or a surface the test did not reach, fails: a probe that counts
  nothing is broken, not green.
- A measured surface without a table entry fails.
- A count below its budget passes and prints "lower the budget" so headroom does
  not invite the next addition.

## Raising a budget

Edit `SURFACES["<key>"].budget` in `scripts/lib/ui-census-budget.mjs` in the same
commit as the feature, with the reason in the commit message and one line below.
The alternatives to growing a surface are palette-only, config-only or cut.
Run `UI_CENSUS_PRINT=1 npx vitest run --config vitest.render.config.ts src/uiCensus.budget.test.tsx`
to print every count with the labels of the controls counted.

## Known gap

jsdom has no layout. Controls that CSS hides at rest (hover reveal, container-query
topbar tiers) are in the DOM and therefore counted, and the topbar width tiers are
not measured. The `topbar.atRest` budget is "controls present in the desktop
topbar", not "pixels visible at 1280 px". A layout-aware count belongs in a native
journey (`scripts/e2e-*.mjs`), not here.

## Raised budgets

- 2026-10-19, D-20261018-W1 (Martin: "Open in new tab" back on mobile): `mobile.menu.block.plain` 14→15,
  `block.full` 18→19, `block.multi` 13→14, `block.readonly` 5→6, `page.link` 11→12, `page.header.page` 14→15,
  `page.header.journal` 14→15. The mobile census now runs as a touch-first device (`touchPrimary`), since
  touch surfaces such as the keyboard toolbar follow input capability, not the OS name.
