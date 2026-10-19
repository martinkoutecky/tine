// UI census budgets (GA5, 2026-10-16): the per-surface ceilings that keep the
// counted UI surfaces from creeping back after the 2026-10 census cuts.
//
// The census (private tine-graph page "Tine UI Census 2026-10" and its
// Proposals page) counted, per surface, the palette commands, the context-menu
// rows, the Settings controls visible at open and the controls at rest. The
// cut lanes landed (ca6ac2fe7, 7082edcea and the parser-tool/carry follow-ups);
// this table pins what is left. `src/uiCensus.budget.test.tsx` measures every
// surface from its source of truth in code (the command registry, the
// autocomplete registries, the real menu and Settings components mounted in
// jsdom), never from pixels, and fails when a count passes its budget here.
//
// Raising a budget is a deliberate, reviewed edit to the table below, in the
// same commit as the feature, with the reason in the commit message and a line
// in docs/ui-census-budget.md. A surface that shrinks passes; the test then
// says to lower the budget so the headroom does not invite the next addition
// (same style as the file-size ratchet in scripts/lib/og-enforcement.mjs).

/**
 * @typedef {{ env: "desktop" | "mobile", budget: number, what: string, source: string }} Surface
 * `env` picks the test file that measures it (the two platforms need different
 * module mocks). `what` names the unit counted; `source` is where the count
 * comes from.
 */

const SETTINGS_SOURCE = "Settings mounted in jsdom: inputs, selects, buttons and summaries in the tab body, minus search results, shortcut rows and spellcheck dictionary rows; the home-page picker is emptied";
const settingsOpen = (tab, budget, note = "") => ({ env: "desktop", budget, what: `controls visible at open on the ${tab} tab${note}`, source: SETTINGS_SOURCE });
const settingsAdvanced = (tab, budget) => ({ env: "desktop", budget, what: `extra controls behind the ${tab} tab's Advanced disclosure`, source: "the same count after opening the tab's Advanced toggle, minus the count at open" });
const mobileMenu = (key, budget) => ({ env: "mobile", budget, what: `rows, menu.${key} on a phone/tablet build`, source: "ContextMenu on an Android build (no new tab, window or PDF rows; the page header adds Find in page)" });

/** @type {Readonly<Record<string, Surface>>} */
export const SURFACES = Object.freeze({
  // --- Commands: registries, no rendering ---------------------------------
  "commands.registry": { env: "desktop", budget: 120, what: "command ids in the Shortcuts list (remappable)", source: "src/keybindings.ts COMMANDS via commandDefaults()" },
  "commands.registry.editor": { env: "desktop", budget: 25, what: "editor-scope command ids", source: "commandDefaults() rows with scope \"editor\"" },
  "commands.palette": { env: "desktop", budget: 68, what: "built-in rows in the command palette (plugin rows excluded)", source: "paletteCommands() minus plugin: ids; a registry command with paletteHidden stays out of it" },
  "keys.builtin": { env: "desktop", budget: 23, what: "non-remappable builtin key rows (documentation only)", source: "src/keybindings.ts BUILTIN_KEYS" },
  "slash.menu": { env: "desktop", budget: 53, what: "slash-menu rows", source: "src/editor/autocomplete.ts COMMANDS (COMMAND_DEFINITIONS)" },
  "angle.menu": { env: "desktop", budget: 16, what: "`<` block-menu rows", source: "src/editor/autocomplete.ts ADVANCED_BLOCK_COMMANDS" },

  // --- Settings ---------------------------------------------------------------
  "settings.tabs": { env: "desktop", budget: 10, what: "Settings tabs", source: "Settings nav items (components/Settings.tsx TABS, plugins tab included)" },
  "settings.appearance.open": settingsOpen("appearance", 28),
  "settings.appearance.advanced": settingsAdvanced("appearance", 12),
  "settings.editor.open": settingsOpen("editor", 9),
  "settings.editor.advanced": settingsAdvanced("editor", 10),
  "settings.journals.open": settingsOpen("journals", 13),
  "settings.journals.advanced": settingsAdvanced("journals", 1),
  "settings.files.open": settingsOpen("files", 7),
  "settings.files.advanced": settingsAdvanced("files", 3),
  "settings.backups.open": settingsOpen("backups", 3),
  "settings.graph.open": settingsOpen("graph", 7),
  "settings.plugins.open": settingsOpen("plugins", 5),
  "settings.diagnostics.open": settingsOpen("diagnostics", 5),
  "settings.shortcuts.open": settingsOpen("shortcuts", 1, " (the rows are the commands budget)"),
  "settings.about.open": settingsOpen("about", 8),

  // --- Context menus (top-level rows; submenu entries are counted apart) ----
  "menu.block.plain": { env: "desktop", budget: 17, what: "text rows, block bullet menu, one writable block", source: "ContextMenu mounted on a plain block" },
  "menu.block.colors": { env: "desktop", budget: 8, what: "background-colour swatches in the block menu", source: "ContextMenu ctx-color buttons" },
  "menu.block.headings": { env: "desktop", budget: 8, what: "heading buttons in the block menu", source: "ContextMenu ctx-h buttons" },
  "menu.block.full": { env: "desktop", budget: 21, what: "text rows, block bullet menu with every conditional row present", source: "ContextMenu on a block with children, a template property, a trailing pipe table and selected rendered words" },
  "menu.block.full.nested": { env: "desktop", budget: 3, what: "submenu entries inside the full block menu (Show children as)", source: "ContextMenu .ctx-submenu-menu rows of the full block" },
  "menu.block.multi": { env: "desktop", budget: 16, what: "text rows, block menu on a two-block selection", source: "ContextMenu with two plain blocks selected" },
  "menu.block.readonly": { env: "desktop", budget: 6, what: "text rows, block menu on a block of a read-only page", source: "ContextMenu on a read-only page" },
  "menu.page.link": { env: "desktop", budget: 14, what: "rows, page menu from a link, sidebar row or title (no file actions)", source: "ContextMenu page kind, fileActions false, icon set so Remove page icon is present" },
  "menu.page.header.page": { env: "desktop", budget: 16, what: "rows, page header menu of an ordinary page (file actions, icon, rename, delete)", source: "ContextMenu page kind, fileActions true" },
  "menu.page.header.nested": { env: "desktop", budget: 2, what: "submenu entries inside the page header menu (Copy as)", source: "ContextMenu .ctx-submenu-menu rows" },
  "menu.page.header.journal": { env: "desktop", budget: 16, what: "rows, page header menu of a past journal day (carry, delete journal)", source: "ContextMenu journal kind, fileActions true, a day that is not today" },
  "menu.blockref": { env: "desktop", budget: 7, what: "rows, block-reference menu on a reference occurrence", source: "ContextMenu blockref kind with an occurrence (edit and remove-reference rows)" },
  "menu.query": { env: "desktop", budget: 8, what: "rows, query block menu (editable, with results and fold)", source: "QueryActions disclosure opened with every action offered" },
  "menu.query.readonly": { env: "desktop", budget: 5, what: "rows, query block menu on a read-only query", source: "QueryActions with editable false" },
  "menu.rsHeader": { env: "desktop", budget: 3, what: "rows, right sidebar header menu", source: "RightSidebar header ⋯ menu" },
  "menu.rsItem": { env: "desktop", budget: 3, what: "rows, right sidebar item menu (three items open, the first a page)", source: "RightSidebar item ⋯ menu" },
  "menu.topbarOverflow": { env: "desktop", budget: 7, what: "rows, topbar ··· overflow menu", source: "TopbarOverflowMenu opened inside the app" },

  // --- Chrome at rest (DOM present; CSS-only hiding is not visible to jsdom) --
  "topbar.atRest": { env: "desktop", budget: 13, what: "controls in the desktop topbar: toolbar buttons (not the ··· trigger) plus the frameless window controls", source: "App header.topbar controls outside .topbar-overflow, plus WindowControls buttons" },
  "topbar.windowControls": { env: "desktop", budget: 3, what: "frameless window controls", source: "WindowControls buttons" },
  "sidebar.left.fixed": { env: "desktop", budget: 5, what: "controls in the left sidebar of an empty graph", source: "App .left-sidebar: workspace and graph switchers, Journals, All pages, New page" },
  "sidebar.left.full": { env: "desktop", budget: 8, what: "controls in the left sidebar with Favorites, Recent and Namespaces present", source: "the same selector with a favorite, a recent page and a namespaced page" },
  "sidebar.right.header": { env: "desktop", budget: 2, what: "controls in the right sidebar header", source: "RightSidebar .right-sidebar-header buttons" },
  "sidebar.right.item": { env: "desktop", budget: 4, what: "controls in one right sidebar item head", source: "RightSidebar .rs-item-head: toggle, title, ⋯, close" },

  // --- Phone/tablet app (uiCensus.mobile.budget.test.tsx) ---------------------
  "mobile.keyboardToolbar": { env: "mobile", budget: 15, what: "buttons in the mobile keyboard toolbar (with Hide keyboard)", source: "MobileKeyboardToolbar mounted by the app shell on an Android build" },
  "mobile.keyboardToolbar.withNavigation": { env: "mobile", budget: 19, what: "buttons in the keyboard toolbar while the keyboard is up with Navigation buttons at the bottom on (the 15 above plus Sidebar, Search, Back, Forward; GH #598, D-20261019-N2)", source: "MobileKeyboardToolbar with the App's navigation group, keyboard simulated up" },
  "mobile.bottomBar": { env: "mobile", budget: 4, what: "buttons in the bottom navigation row (Navigation buttons at the bottom on)", source: "App .mobile-navigation-toolbar" },
  "mobile.menu.block.plain": mobileMenu("block.plain", 17), // +1 Open in new tab, D-20261018-W1; +2 copy rows out of Copy as, GH #725
  "mobile.menu.block.full": mobileMenu("block.full", 21), // +1 Open in new tab, D-20261018-W1; +2 copy rows out of Copy as, GH #725
  "mobile.menu.block.full.nested": mobileMenu("block.full.nested", 3), // Copy as left the block menu, GH #725
  "mobile.menu.block.multi": mobileMenu("block.multi", 16), // +1 Open in new tab, D-20261018-W1; +2 copy rows out of Copy as, GH #725
  "mobile.menu.block.readonly": mobileMenu("block.readonly", 6), // +1 Open in new tab, D-20261018-W1
  "mobile.menu.page.link": mobileMenu("page.link", 12), // +1 Open in new tab, D-20261018-W1
  "mobile.menu.page.header.page": mobileMenu("page.header.page", 15), // +1 Open in new tab, D-20261018-W1
  "mobile.menu.page.header.nested": mobileMenu("page.header.nested", 2),
  "mobile.menu.page.header.journal": mobileMenu("page.header.journal", 15), // +1 Open in new tab, D-20261018-W1
  "mobile.menu.blockref": mobileMenu("blockref", 7),
});

/**
 * Compare measured counts with the budgets of one env.
 * Throws naming every violating surface; returns the surfaces that shrank.
 * @param {Record<string, number>} counts
 * @param {"desktop" | "mobile"} env
 * @param {Readonly<Record<string, Surface>>} [surfaces]
 * @returns {{ slack: string[] }}
 */
export function checkSurfaceBudgets(counts, env, surfaces = SURFACES) {
  const failures = [];
  const slack = [];
  const own = Object.entries(surfaces).filter(([, surface]) => surface.env === env);
  for (const [key, surface] of own) {
    const count = counts[key];
    if (!Number.isInteger(count)) {
      failures.push(`${key}: not measured. Every budgeted surface must be reached by the census test (${surface.source}).`);
    } else if (count === 0) {
      failures.push(`${key}: counted 0 ${surface.what}. A surface that counts nothing is a broken probe, not a pass (${surface.source}).`);
    } else if (count > surface.budget) {
      failures.push(
        `${key}: ${count} ${surface.what} > budget ${surface.budget}. The UI census (2026-10) pins this surface; ` +
        `a growing surface needs a deliberate decision. To raise it, edit SURFACES["${key}"].budget to ${count} in ` +
        `scripts/lib/ui-census-budget.mjs in a reviewed change (reason in the commit message, line in docs/ui-census-budget.md); ` +
        `otherwise move the new entry to palette-only, config-only or cut. Counted from: ${surface.source}.`,
      );
    } else if (count < surface.budget) {
      slack.push(`${key}: ${count} ${surface.what} < budget ${surface.budget}; lower SURFACES["${key}"].budget to ${count} in scripts/lib/ui-census-budget.mjs.`);
    }
  }
  const known = new Set(own.map(([key]) => key));
  for (const key of Object.keys(counts)) {
    if (!known.has(key)) failures.push(`${key}: measured but has no entry in SURFACES (env ${env}). Add its budget to scripts/lib/ui-census-budget.mjs.`);
  }
  if (failures.length) throw new Error(`UI census budget exceeded or unreachable:\n${failures.join("\n")}`);
  return { slack };
}
