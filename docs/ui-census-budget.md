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

- 2026-10-30, keys dossier / GH #797, #799: `commands.registry` 131→134 and
  `commands.palette` 74→77 for the explicitly requested **Zoom in on block** and
  **Zoom out one level** (both unbound), plus **Reload window** (unbound), the one
  explicit way to reload now that the webview's Ctrl/Cmd+R and F5 are consumed. No
  new topbar control.
- 2026-10-29, D-20261029-ST (GH #796): `settings.editor.advanced` 10→11 for the explicitly
  decided **Search results open in** (New tab / This tab) setting. It sits behind the Advanced
  disclosure beside "Switch to an already-open tab", so `settings.editor.open` stays 9.
- 2026-10-29, D-20261029-SS (Previous / Next change in the seen notice, discussion 645):
  `commands.registry` 129→131 for the required bindable **Next change since you last looked**
  and **Previous change since you last looked** commands. Both are offered (palette and
  remapping) only while a seen notice with changes is on screen, so the jsdom palette count
  does not move; no key is bound. The two notice buttons are page chrome shown only while
  N > 0, not a census surface.
- 2026-10-28, sweep-nav dossier / GH #785: `commands.registry` 126→129 and
  `commands.palette` 71→74 for the explicitly requested localized **Expand all**,
  **Collapse all**, and OG's remappable **t o** toggle. No new topbar control.
- 2026-10-28, D-20261028-SG ("Changed since you last looked", ADR 0079): `commands.registry` 124→126
  for the required **Mark page seen** and **Mark whole graph seen** commands. Both are
  available only on a git graph that answered a seen read (never on Android/iOS), so the
  jsdom palette count does not move; no key is bound.
- 2026-10-26, D-20261026-778 (GH #778): `commands.registry` 122→124,
  `commands.palette` 69→71, `settings.appearance.advanced` 12→14 for the
  explicitly approved **Open in default app** actions for graph and global CSS,
  beside the existing in-app Edit buttons and in the desktop command palette.
  Both external actions are hidden on Android/iOS; no additional preference.
- 2026-10-26, CM6 stage-2 Land brief (slice s1): `settings.editor.advanced`
  10→11 only for the Beta/development identity, for the explicitly requested
  default-off **Experimental: CodeMirror editor** switch. Stable remains at 10
  and hides the switch and its search entry.
- 2026-10-24, D-20261024-QA (GH #758 and GH #757 part 3): `commands.registry` 120→122,
  `commands.registry.editor` 25→26, `commands.palette` 68→69 for the explicitly approved
  **All graphs** navigation command (`g shift+g`, also required in the palette) and
  **Insert YouTube timestamp** editor command (`mod+shift+y`). Both remain remappable
  through the existing Shortcuts list; no extra settings or slash commands were added.
- 2026-10-22, GX4 S3 target 1 / Change Summary design §5 (D-20261022-HJ, HJ7):
  `settings.diagnostics.open` 5→6 for the explicitly required session-only
  **Refresh everything on every save (debug)** comparison switch.
- 2026-10-19, D-20261018-W1 (Martin: "Open in new tab" back on mobile): `mobile.menu.block.plain` 14→15,
  `block.full` 18→19, `block.multi` 13→14, `block.readonly` 5→6, `page.link` 11→12, `page.header.page` 14→15,
  `page.header.journal` 14→15. The mobile census now runs as a touch-first device (`touchPrimary`), since
  touch surfaces such as the keyboard toolbar follow input capability, not the OS name.
- 2026-10-19, D-20261019-N2 (GH #598): new surface `mobile.keyboardToolbar.withNavigation` (19 = the 15 keyboard-toolbar
  buttons plus Sidebar, Search, Back, Forward). With Navigation buttons at the bottom on, the keyboard toolbar carries the
  four navigation buttons at its leading end while the keyboard is up. `mobile.keyboardToolbar` (setting off) stays 15.
- 2026-10-19, GH #725 (reporter: copy ref/embed/link behind a submenu is friction; OG lists them top-level): block
  menu rows `menu.block.plain` 15→17, `block.full` 19→21, `block.multi` 14→16 and the mobile twins likewise (Copy block
  ref, Copy block embed and Copy link left the Copy as submenu, net +2 rows each); `block.full.nested` 6→3 (Copy as left
  the block menu; the page menu keeps its Copy as). Approved by Martin 2026-10-19 (D-20261019-UC1, option a: all three top-level, as in OG).
- 2026-10-30, D-20261030-PE (GH #418, Ellis): `menu.page.header.nested` 2→3 and `mobile.menu.page.header.nested` 2→3:
  **Copy as ▸ → Copy page embed** (`{{embed [[Page]]}}`) joins Copy page ref and Copy link. OG's page menu has no such row;
  approved by Martin 2026-10-30 ("Mk21 yes").
