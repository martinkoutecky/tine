// UI census budget test (GA5). Counts every census surface from its source of
// truth in code and pins each count against scripts/lib/ui-census-budget.mjs.
// Registries are read as data (command registry, autocomplete registries); the
// menus, Settings and chrome are the real components mounted in jsdom, so a row
// added to a menu definition shows up here with no second list to forget.
// Nothing here measures pixels: jsdom has no layout, so CSS-only visibility
// (hover reveal, container-query tiers) is out of reach and documented in
// docs/ui-census-budget.md.
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { checkSurfaceBudgets, SURFACES, type Surface } from "../scripts/lib/ui-census-budget.mjs";
import { commandDefaults, BUILTIN_KEYS, paletteCommands } from "./keybindings";
import { ADVANCED_BLOCK_COMMANDS, COMMANDS as SLASH_COMMANDS } from "./editor/autocomplete";
import { initParser } from "./render/parse";
import { describeControl, controlsIn, measureContextMenus, mount, required, settled } from "./uiCensus.test.support";
import { App } from "./App";
import { Settings } from "./components/Settings";
import { QueryActions } from "./components/QueryActions";
import { RightSidebar } from "./components/RightSidebar";
import { WindowControls } from "./components/WindowChrome";
import { backend } from "./backend";
import { refreshPageIndex, resetPageIndex } from "./pageIndex";
import { resetStore } from "./document";
import { loadSingle } from "./document/workingSet";
import { endEdit } from "./editorController";
import {
  applySidebarSession, closeSettings, openSettings,
  seedFavorites, setLeftSidebarOpen, setRecentPages, setRightSidebar, type SettingsTabId,
} from "./ui";

// The reference desktop build: Tauri windows are supported, so desktop-only
// rows (new window) are present. Mobile counts live in uiCensus.mobile.budget.test.tsx.
vi.mock("./workspaceWindows", async (original) => ({ ...(await original<typeof import("./workspaceWindows")>()), workspaceWindowsSupported: () => true }));
// WindowControls reads the native window handle; the buttons are what is counted.
vi.mock("@tauri-apps/api/window", () => {
  const handle = { isMaximized: async () => false, onResized: async () => () => {}, minimize: async () => {}, toggleMaximize: async () => {}, close: async () => {}, startResizeDragging: async () => {} };
  return { getCurrentWindow: () => handle };
});

const counts: Record<string, number> = {};
const labels: Record<string, string[]> = {};
// ---- Settings -------------------------------------------------------------
const SETTINGS_TABS: SettingsTabId[] = ["appearance", "editor", "journals", "files", "backups", "graph", "plugins", "diagnostics", "shortcuts", "about"];
async function measureSettings() {
  // The home-page picker lists graph pages; an empty graph keeps it structural.
  vi.spyOn(backend(), "quickSwitch").mockResolvedValue([]);
  const { root, dispose } = mount(() => <Settings />);
  try {
    openSettings("appearance");
    const nav = await settled(() => root.querySelectorAll(".settings-nav-item").length);
    counts["settings.tabs"] = nav;
    for (const tab of SETTINGS_TABS) {
      openSettings(tab);
      // Anchor: the tab is current once its nav item is active.
      await settled(() => [root.querySelector(".settings-nav-item.active")?.textContent, root.querySelector(".settings-pane-body")?.innerHTML.length]);
      const body = required(root.querySelector<HTMLElement>(".settings-pane-body"), `Settings ${tab} body`);
      // Shortcut rows are the command registry rendered once more (143 rows);
      // the chrome around them is the Settings surface. Search results are not settings.
      const own = (el: HTMLElement) => !el.closest(".settings-search-results, .help-shortcut-row, .spellcheck-dict");
      const atOpen = await settled(() => controlsIn(body).filter(own).map(describeControl));
      counts[`settings.${tab}.open`] = atOpen.length;
      labels[`settings.${tab}.open`] = atOpen;
      const toggle = body.querySelector<HTMLButtonElement>(".settings-advanced-toggle");
      if (toggle) {
        toggle.click();
        const expanded = await settled(() => controlsIn(body).filter(own).length);
        counts[`settings.${tab}.advanced`] = expanded - atOpen.length;
        toggle.click();
      }
      if (tab === "shortcuts") counts["settings.shortcuts.rows"] = body.querySelectorAll(".help-shortcut-row").length;
    }
  } finally { dispose(); closeSettings(); vi.restoreAllMocks(); }
}

// ---- Context menus ---------------------------------------------------------
// ---- Query block menu, right sidebar --------------------------------------
async function measureQueryMenu() {
  const noop = () => {};
  for (const [key, editable] of [["menu.query", true], ["menu.query.readonly", false]] as const) {
    const { root, dispose } = mount(() => <QueryActions editable={editable} results edit={noop} copy={noop} search={noop} export={noop} share={noop} fold={noop} folded={false} />);
    try {
      const details = required(root.querySelector<HTMLDetailsElement>("details.qc-actions"), "the query actions disclosure");
      details.open = true;
      details.dispatchEvent(new Event("toggle"));
      const buttons = await settled(() => controlsIn(document.body, ".qc-menu button").length);
      counts[key] = buttons;
    } finally { dispose(); }
  }
}
async function measureRightSidebar() {
  const pageDto = { name: "Census page", kind: "page" as const, title: "Census page", pre_block: null, blocks: [
    { id: "cs-one", raw: "One", collapsed: false, children: [] }, { id: "cs-two", raw: "Two", collapsed: false, children: [] }] };
  loadSingle(pageDto);
  applySidebarSession({ right: true, items: [
    { kind: "page", name: pageDto.name, pageKind: "page" },
    { kind: "block", uuid: "cs-one", page: pageDto.name, pageKind: "page" },
    { kind: "block", uuid: "cs-two", page: pageDto.name, pageKind: "page" }] });
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
  const { root, dispose } = mount(() => <RightSidebar />);
  try {
    const header = required(root.querySelector<HTMLElement>(".right-sidebar-header"), "the right sidebar header");
    const item = required(root.querySelector<HTMLElement>(".rs-item-head"), "a right sidebar item head");
    counts["sidebar.right.header"] = controlsIn(header).length;
    // The item title is an anchor without href (a click handler), so match `a` itself.
    counts["sidebar.right.item"] = controlsIn(item, "button, a").length;
    labels["sidebar.right.item"] = controlsIn(item, "button, a").map(describeControl);
    // Header ⋯ menu.
    required(header.querySelector<HTMLButtonElement>("[data-right-sidebar-actions]"), "the header actions button").click();
    counts["menu.rsHeader"] = await settled(() => document.querySelectorAll("[data-right-sidebar-action]").length);
    required(header.querySelector<HTMLButtonElement>("[data-right-sidebar-actions]"), "the header actions button").click();
    // Item ⋯ menu of the first (page) item among three: every conditional row present.
    required(root.querySelector<HTMLButtonElement>("[data-right-sidebar-item-actions]"), "an item actions button").click();
    counts["menu.rsItem"] = await settled(() => document.querySelectorAll("[data-right-sidebar-item-action]").length);
  } finally { dispose(); endEdit("page-navigation"); setRightSidebar([]); applySidebarSession({ right: false, items: [] }); resetStore(); vi.restoreAllMocks(); }
}

// ---- Chrome at rest (the whole app mounted) ---------------------------------
const leftSidebarControlSet = (scope: ParentNode) =>
  new Set(controlsIn(required(scope.querySelector<HTMLElement>(".left-sidebar"), "the left sidebar"),
    "[data-workspace-switcher-sidebar] button, .sidebar-header button, .nav-contents > .nav-item, .nav-contents .nav-section-header, .sidebar-footer button, .sidebar-footer .conflict-queue-badge"));
const leftSidebarControls = (scope: ParentNode) => leftSidebarControlSet(scope).size;
async function measureChrome() {
  setLeftSidebarOpen(true);
  // An empty graph: no favorites, no recent pages, no namespaced pages.
  const inventory = vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [] });
  const app = mount(() => <App />);
  try {
    await refreshPageIndex();
    const topbar = await settled(() => required(app.root.querySelector<HTMLElement>("header.topbar"), "the topbar"));
    const overflow = controlsIn(topbar, ".topbar-overflow button");
    const own = controlsIn(topbar).filter((el) => !el.closest(".topbar-overflow"));
    labels["topbar.atRest"] = own.map(describeControl);
    const windowControls = mount(() => <WindowControls />);
    const windowButtons = controlsIn(windowControls.root, "button").length;
    windowControls.dispose();
    counts["topbar.atRest"] = own.length + windowButtons;
    counts["topbar.windowControls"] = windowButtons;
    counts["sidebar.left.fixed"] = await settled(() => leftSidebarControls(app.root));
    labels["sidebar.left.fixed"] = [...leftSidebarControlSet(app.root)].map(describeControl);
    // The overflow menu: open it and count what it offers.
    required(overflow.find((el) => el.classList.contains("topbar-overflow-trigger")) ?? null, "the toolbar overflow trigger").click();
    counts["menu.topbarOverflow"] = await settled(() => document.querySelectorAll("[data-topbar-overflow-action]").length);
    // Every conditional left-sidebar header: Favorites, Recent, Namespaces.
    seedFavorites(["Alpha"]);
    setRecentPages([{ name: "Alpha", kind: "page" }]);
    inventory.mockResolvedValue({ rev: "2", entries: [{ key: "project/roadmap", name: "Project/Roadmap", is_journal: false, day: null, target: { kind: "absent", id: "pages/Project___Roadmap.md" } }] });
    await refreshPageIndex();
    counts["sidebar.left.full"] = await settled(() => leftSidebarControls(app.root));
    labels["sidebar.left.full"] = [...leftSidebarControlSet(app.root)].map(describeControl);
  } finally { app.dispose(); seedFavorites([]); setRecentPages([]); resetPageIndex(); inventory.mockRestore(); }
}

describe("UI census budgets (desktop)", () => {
  beforeAll(async () => {
    await initParser();
    // Registries.
    const defaults = commandDefaults();
    counts["commands.registry"] = defaults.length;
    counts["commands.registry.editor"] = defaults.filter((command) => command.scope === "editor").length;
    counts["commands.palette"] = paletteCommands().filter((command) => !command.id.startsWith("plugin:")).length;
    counts["keys.builtin"] = BUILTIN_KEYS.length;
    counts["slash.menu"] = SLASH_COMMANDS.length;
    counts["angle.menu"] = ADVANCED_BLOCK_COMMANDS.length;
    // Rendered surfaces.
    await measureSettings();
    measureContextMenus(counts, labels, "menu");
    await measureQueryMenu();
    await measureRightSidebar();
    await measureChrome();
  }, 120_000);
  afterAll(() => {
    if (process.env.UI_CENSUS_PRINT) console.info(`UI_CENSUS ${JSON.stringify({ counts, labels }, null, 1)}`);
  });

  it("holds every desktop surface within its budget", () => {
    // Settings' derived row count is a cross-check, not a surface: it must equal
    // the registry it renders (commands + builtin keys), or the tab dropped rows.
    expect(counts["settings.shortcuts.rows"]).toBe(counts["commands.registry"] + counts["keys.builtin"]);
    delete counts["settings.shortcuts.rows"];
    const { slack } = checkSurfaceBudgets(counts, "desktop");
    // Shrinking passes; say so, so the headroom is not left for the next addition.
    for (const line of slack) console.warn(`UI census: ${line}`);
  });

  it("fails on planted growth, a missing probe, a zero probe and an unbudgeted surface", () => {
    const table: Record<string, Surface> = { "menu.x": { env: "desktop", budget: 5, what: "rows", source: "a test menu" } };
    expect(() => checkSurfaceBudgets({ "menu.x": 6 }, "desktop", table)).toThrow(/menu\.x: 6 rows > budget 5[\s\S]*edit SURFACES\["menu\.x"\]\.budget to 6/);
    expect(() => checkSurfaceBudgets({}, "desktop", table)).toThrow(/menu\.x: not measured/);
    expect(() => checkSurfaceBudgets({ "menu.x": 0 }, "desktop", table)).toThrow(/menu\.x: counted 0 rows/);
    expect(() => checkSurfaceBudgets({ "menu.x": 5, "menu.y": 1 }, "desktop", table)).toThrow(/menu\.y: measured but has no entry/);
    expect(checkSurfaceBudgets({ "menu.x": 4 }, "desktop", table).slack).toEqual([expect.stringMatching(/menu\.x: 4 rows < budget 5; lower SURFACES\["menu\.x"\]\.budget to 4/)]);
    expect(checkSurfaceBudgets({ "menu.x": 5 }, "desktop", table).slack).toEqual([]);
    // The other platform's keys are not demanded of this file.
    expect(checkSurfaceBudgets({ "menu.x": 5 }, "desktop", { ...table, "mobile.y": { env: "mobile", budget: 1, what: "rows", source: "x" } }).slack).toEqual([]);
    expect(Object.values(SURFACES).every((surface) => surface.env === "desktop" || surface.env === "mobile")).toBe(true);
  });
});
