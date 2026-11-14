# UI census budgets

The 2026-10 UI census counted every surface a user sees: palette commands,
context-menu rows, Settings controls visible at open, controls at rest. The cut
lanes landed; this test keeps the counts from creeping back.

`src/uiCensus.budget.test.tsx` (desktop) and `src/uiCensus.mobile.budget.test.tsx`
(Android build) measure each surface from its source of truth in code, never from
pixels, and compare it with the frozen table `SURFACES` in
`scripts/lib/ui-census-budget.mjs`. Both run under `npm test` (render config).

The editor Advanced budget excludes the retired parent-copy preference and its
Logseq revert (#941).

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

FHj1Y R1 (2026-11-14) pins `settings.sharing.open` at 1 (Connect GitHub,
enabled quick share with no connected account or shares). The census measures
every declared Settings tab with Sharing enabled; `settings.tabs` keeps its
ordinary gate-off count of 11. All existing budgets stay unchanged.

H9TvA (D-20261113-CO/CO2) adds the approved **Include comments** checkbox to
the existing Graph publication controls. `settings.graph.open` is 8 (was 7),
accounting for that one export content choice. Other surface and performance
budgets stay unchanged.

MC9 (D-20261109-MC9; GH #891 c13) explicitly adds **Next comment** and
**Previous comment** to both the palette and Keyboard shortcuts. The approved
review capability sets `commands.registry` to 153 (from 151) and
`commands.palette` to 93 (from 91).

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

- 2026-11-08, D-20261107-MC3 §7 (UI-MC010-VISIBILITY, GH #891): ordinary page
  and journal header menus gain one Show comments/Hide comments row: desktop
  16→17, mobile 15→16. The approved design requires a visible mobile control
  and delegates page menu versus keyboard toolbar placement to this lane.
  The page header menu keeps this reading action available without occupying
  the editing toolbar; the same action is available on desktop. Link/sidebar
  context menus gain no row. Review rationale is also in the lane receipt;
  this implementation lane is explicitly instructed not to commit.

## Known gap

jsdom has no layout. Controls that CSS hides at rest (hover reveal, container-query
topbar tiers) are in the DOM and therefore counted, and the topbar width tiers are
not measured. The `topbar.atRest` budget is "controls present in the desktop
topbar", not "pixels visible at 1280 px". A layout-aware count belongs in a native
journey (`scripts/e2e-*.mjs`), not here.

## Raised budgets

- 2026-11-13, GH #926 (Martin: "926 yes to all", Q-S3): `settings.tabs`
  10→11 for the explicitly approved first-level **Themes** category. Its eight
  existing controls move out of Appearance: `settings.appearance.open` 31→23
  and `settings.themes.open` = 8, with no net growth in setting controls.
  A palette-only entry would not provide the approved category or keep the
  dynamic theme gallery out of Appearance. The census derives its category
  probes from the rendered navigation IDs. With the toolbar section below
  (+22 controls, 31→53 before the move), Appearance is 23 + 22 = 45.
- 2026-11-14, D-20261113-UI / #926 (926-LEFTPANEL, desktop sidebar and phone drawer):
  `sidebar.left.fixed` 5→6 and `sidebar.left.full` 8→9 for the close icon beside
  the graph picker. This keeps the explicitly requested close control in the
  shared header at both widths. Palette-only or a keyboard shortcut cannot
  provide the requested visible control.
- 2026-11-13, GH #926 (D-20261113-UI, approved toolbar design section 3):
  `settings.appearance.open` 31→53 for the seven actions' up/down and section-move
  buttons (21), plus Reset (1). These are the approved accessible alternatives
  to dragging in the two ordered lists. `menu.topbarOverflow` 8→10 for Workspaces
  and Customize toolbar; its probe chooses all optional actions for the menu
  and counts every menuitem, including the fixed Customize row. The default
  toolbar count and all timing/memory budgets are unchanged. This records the
  approved capability's controls, rather than raising a performance budget.
- 2026-11-08, GH #891 (D-20261108-MC4 and the 0.10 On click lane brief):
  `settings.appearance.open` 30→31 for the explicitly requested **Comments**
  display selector, **In the margin** / **On click**, beside the other display
  preferences. It reuses the annotation settings group and adds no command,
  menu or topbar control. Palette-only or graph configuration would not provide
  the approved device-local Settings choice. Integration commit rationale is
  recorded in the lane receipt; this implementation lane does not commit.
- 2026-11-08, GH #896 (D-20261107-MC3 §10 and the 0.10 author-marks lane brief):
  `commands.registry` 149→150, `commands.palette` 90→91 and
  `settings.appearance.open` 29→30 for the explicitly requested independent
  **Show/Hide author marks** command and switch beside **Show/Hide comments**.
  The command has no default key; there is no additional topbar or menu control.
- 2026-11-10, D-20261110-TT / GH #824: `settings.editor.advanced` 13→14 on Beta/development
  (12→13 without the Beta-only CM6 switch) for the explicitly decided **Reference previews**
  switch, Logseq's `:ui/enable-tooltip?`, beside the hover delay (which stays). It sits in
  Advanced, not Basic; the open count is unchanged.
- 2026-11-10, GH #918 (D-20261109-SG, card G-WTQ): `commands.registry` 149→150 for the
  explicitly requested bindable **Remove author** command (`editor/remove-author`, no default
  key). It runs the block context menu's own Remove author and is offered in the palette only
  where that menu offers it (one authored, writable block), so the default palette count stays
  at 90; no new Settings control or chrome.
- 2026-11-11, merge of the two entries above (#896 and #918, each approved): both took
  `commands.registry` 149→150 on separate lines, so together it is 151. No other change.
- 2026-11-07, GH #805 (0.10 lane brief, Martin's 10/30 AA and TRIAGE-1031 placement):
  `commands.registry` 148→149 for the explicitly requested remappable **Quit Tine**
  command (`app/quit`, Ctrl+Q on Linux/Windows). It reaches the existing safe Quit
  path. The command is native-only, so the browser-mock palette count stays at 90;
  no new Settings control or topbar button.
- 2026-11-06, GH #860 / roadmap AG (Martin's 2026-11-03 placement, SWEEP-1103):
  `commands.registry` 147→148 and `commands.palette` 89→90 for **Toggle embed breadcrumbs**,
  explicitly requested as a palette command, rebindable with no default key like Toggle custom CSS.
  It changes the existing Fine-tuning setting; no new Settings control or topbar button.
- 2026-11-06, GH #861 (roadmap AG, Martin's 2026-11-03 placement, Section B of SMALL-010-1106):
  `settings.editor.open` 9→10 for the explicitly approved **Auto-embed pasted video links**
  setting. This one default-on switch keeps pasted video URLs as plain links when off;
  no title-fetch setting or new command is added.
- 2026-11-01, GH #764 and GH #818 (D-20261101-L5 decided both for 0.10 on development; this entry records only
  that): `commands.registry` 145→147, `commands.registry.editor` 26→27 and `commands.palette` 88→89. **Merge selected
  blocks** (#764) is a palette and keymap command with no default key and no menu row. **New block after this block's
  children** (#818) is a rebindable editor command with the default key **Alt+Enter** (unbound in OG; in Tine it
  only split the block like Enter, and sheet cells keep their own Alt+Enter). It is not a palette row.
- 2026-11-12, D-20261111-QP2 / GH #926: `settings.appearance.advanced` 17→18 for the approved
  **Show the help button** device switch (behind Advanced; Settings search finds it);
  `settings.diagnostics.open` 6→7 for a closed **Help** disclosure listing the help menu's entries,
  so help stays reachable with the button hidden; `menu.topbarOverflow` 7→8 for **Settings** as the
  "…" menu's last row (Settings two taps away at any zoom). The phone drawer's fixed Settings row is
  drawer-only and outside the desktop census.
- 2026-11-10, D-20261110-HF / GH #927: `settings.appearance.advanced` 16→17
  for the requested **Show Workspaces** device switch. It stays behind Advanced;
  Settings search reveals it, and the initial Appearance control count stays 29.
- 2026-11-01, D-20261101-L5 / GH #794: `commands.registry` 135→145 and `commands.palette`
  78→88 for the decided **Set depth 0** to **Set depth 9** (ten commands, all
  unbound; Mod+digit stays pane focus). D-20261101-L5 takes the full 0 to 9 range over the
  issue's "stopping at 5 is fine". No new topbar control and no Settings row.
- 2026-11-01, GH #845 (Martin, 2026-11-01, D-20261101-MT: "a good idea and should also go in the beta"):
  `commands.registry` 134→135,
  `commands.palette` 77→78 and `settings.appearance.open` 28→29 for the requested **Toggle
  comment margin** command (default shortcut **t m**, free in OG and Tine, rebindable) and
  its **Comment margin** switch beside Wide mode and Document mode. No topbar control.
- 2026-11-01, GH #823 (brief: one setting for the outline mirroring):
  `settings.appearance.advanced` 14→15 for **Right-to-left note layout**. It sits behind
  Advanced so the open count stays 28; an interface in a right-to-left language turns it on by
  default, and search finds it.
- 2026-11-01, D-20261031-PEEK / GH #824: `settings.editor.advanced` 12→13
  on Beta/development (11→12 without the Beta-only CM6 switch) for the explicitly
  decided numeric **Hover preview delay** setting. This one control also covers
  people who want no incidental previews; no separate toggle. The open count stays 9.
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
