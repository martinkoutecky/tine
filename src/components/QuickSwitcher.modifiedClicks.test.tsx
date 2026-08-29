import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { __setBackendForTest, type Backend } from "../backend";
import {
  closeSwitcher,
  openCommandPalette,
  openSwitcher,
  setRecentPages,
  setRightSidebarOpen,
  rightSidebarOpen,
  switcherOpen,
  contextMenu,
  closeContextMenu,
} from "../ui";
import { layoutPaneIds, paneRouter, resetPaneLayoutToSingle } from "../panes";
import { routeTitle } from "../router";
import { clearTransientLayersForTest } from "../transientLayers";
import { LONG_PRESS_DELAY } from "../render/longPress";

// GH #288: pointer modifiers on result rows must mirror the Enter-key
// semantics — Ctrl/Cmd+click fans a background tab out of the origin pane
// without closing the switcher, Alt+click opens in the other pane, Shift+click
// opens in the right sidebar, and create/command rows keep only their safe
// existing actions. Ordinary left-click and the keyboard paths are untouched.

type Mounted = { root: HTMLDivElement; dispose: () => void };

const journalsSnapshot = () => ({
  tabs: [{ history: [{ kind: "journals" as const }], pos: 0, pinned: false }],
  activeIndex: 0,
});

function mount(): Mounted {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(() => <QuickSwitcher />, root) };
}

async function settle(rounds = 4) {
  for (let i = 0; i < rounds; i++) await Promise.resolve();
}

async function settleSearch() {
  // 110ms input debounce + resource promise.
  await new Promise((resolve) => setTimeout(resolve, 180));
  await settle();
}

function rows(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(".switcher-row")];
}

function input(root: HTMLElement): HTMLInputElement {
  return root.querySelector<HTMLInputElement>(".switcher-input")!;
}

function pressMouse(el: HTMLElement, init: MouseEventInit) {
  el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, ...init }));
}

function pressEnter(root: HTMLElement, init: KeyboardEventInit) {
  input(root).dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, ...init })
  );
}

function installEmptySearchBackend(blockHit?: unknown) {
  __setBackendForTest({
    runGraphSearch: async () => ({
      hits: blockHit ? [blockHit] : [],
      diagnostics: [],
      explanation: { branches: [] },
      has_more: { pages: false, blocks: false },
      cancelled: false,
    }),
    savePage: async () => undefined,
    getPage: async () => null,
    getPageByPath: async () => null,
  } as unknown as Backend);
}

const blockHit = {
  entity: "block",
  page: "BetaPage",
  kind: "page",
  path: "",
  display_text: "needle block target",
  block: { id: "blk-1", breadcrumb: [] },
  evidence: [],
  match_class: "substring",
};

afterEach(() => {
  vi.useRealTimers();
  closeContextMenu();
  closeSwitcher();
  setRecentPages([]);
  setRightSidebarOpen(false);
  __setBackendForTest(null);
  clearTransientLayersForTest();
  resetPaneLayoutToSingle(journalsSnapshot());
  document.body.innerHTML = "";
});

describe("QuickSwitcher modified clicks (GH #288)", () => {
  it("opens a page result's context menu on a still touch hold (GH #207)", async () => {
    const { root } = mount();
    setRecentPages([{ name: "Alpha", kind: "page" }]);
    openSwitcher();
    await settle();
    const row = rows(root)[0];
    expect(row.hasAttribute("data-page-context-menu")).toBe(true);

    vi.useFakeTimers();
    row.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      cancelable: true,
      pointerType: "touch",
      isPrimary: true,
      pointerId: 7,
      clientX: 12,
      clientY: 24,
    }));
    vi.advanceTimersByTime(LONG_PRESS_DELAY);

    expect(contextMenu()).toMatchObject({ kind: "page", name: "Alpha" });
    expect(switcherOpen()).toBe(true);
  });

  it("middle-click opens a background tab in the origin pane, keeps the switcher open, and swallows the follow-up paste", async () => {
    const { root } = mount();
    setRecentPages([{ name: "Alpha", kind: "page" }]);
    openSwitcher();
    await settle();
    const tabCountBefore = paneRouter("main").tabs().length;

    pressMouse(rows(root)[0], { button: 1 });
    await settle();

    expect(switcherOpen()).toBe(true);
    const tabs = paneRouter("main").tabs();
    expect(tabs.length).toBe(tabCountBefore + 1);
    expect(tabs.some((t) => routeTitle(t.history[t.pos]) === "Alpha")).toBe(true);
    // The origin tab stays active — "background" means no focus theft.
    expect(routeTitle(paneRouter("main").route())).toBe("Journals");

    const paste = new Event("paste", { bubbles: true, cancelable: true });
    input(root).dispatchEvent(paste);
    expect(paste.defaultPrevented).toBe(true);
  });

  it("Ctrl/Cmd+click behaves like middle-click: background tab, switcher and input focus stay", async () => {
    for (const mod of [{ ctrlKey: true }, { metaKey: true }]) {
      const { root, dispose } = mount();
      setRecentPages([{ name: "Alpha", kind: "page" }]);
      openSwitcher();
      await settle();
      const tabCountBefore = paneRouter("main").tabs().length;

      pressMouse(rows(root)[0], { button: 0, ...mod });
      await settle();

      expect(switcherOpen()).toBe(true);
      expect(document.activeElement).toBe(input(root));
      const tabs = paneRouter("main").tabs();
      expect(tabs.length).toBe(tabCountBefore + 1);
      expect(tabs.some((t) => routeTitle(t.history[t.pos]) === "Alpha")).toBe(true);

      // No PRIMARY-selection paste follows a left click — nothing to swallow.
      const paste = new Event("paste", { bubbles: true, cancelable: true });
      input(root).dispatchEvent(paste);
      expect(paste.defaultPrevented).toBe(false);

      dispose();
      closeSwitcher();
      setRecentPages([]);
      resetPaneLayoutToSingle(journalsSnapshot());
      document.body.innerHTML = "";
    }
  });

  it("Alt+click uses the existing other-pane action and closes the switcher", async () => {
    const { root } = mount();
    setRecentPages([{ name: "Alpha", kind: "page" }]);
    openSwitcher();
    await settle();
    expect(layoutPaneIds()).toEqual(["main"]);

    pressMouse(rows(root)[0], { button: 0, altKey: true });
    await settle(8);

    expect(switcherOpen()).toBe(false);
    expect(layoutPaneIds().length).toBe(2);
    const otherId = layoutPaneIds().find((id) => id !== "main")!;
    expect(routeTitle(paneRouter(otherId).route())).toBe("Alpha");
    // Origin pane keeps its own route — a copy, not a move.
    expect(routeTitle(paneRouter("main").route())).toBe("Journals");
  });

  it("plain left-click and Enter still open in the origin pane and close", async () => {
    const { root } = mount();
    setRecentPages([{ name: "Alpha", kind: "page" }]);
    openSwitcher();
    await settle();

    pressMouse(rows(root)[0], { button: 0 });
    await settle();

    expect(switcherOpen()).toBe(false);
    expect(routeTitle(paneRouter("main").route())).toBe("Alpha");
    expect(layoutPaneIds()).toEqual(["main"]);
  });

  it("Shift+click opens page results in the right sidebar and closes the switcher", async () => {
    const { root } = mount();
    setRecentPages([{ name: "Alpha", kind: "page" }]);
    openSwitcher();
    await settle();
    const tabCountBefore = paneRouter("main").tabs().length;

    pressMouse(rows(root)[0], { button: 0, shiftKey: true });
    await settle();

    expect(switcherOpen()).toBe(false);
    expect(rightSidebarOpen()).toBe(true);
    expect(paneRouter("main").tabs().length).toBe(tabCountBefore);
  });

  it("Shift+click opens block results in the right sidebar", async () => {
    installEmptySearchBackend(blockHit);
    const { root } = mount();
    openSwitcher();
    await settle();
    const inputEl = input(root);
    inputEl.value = "needle";
    inputEl.dispatchEvent(new Event("input", { bubbles: true }));
    await settleSearch();

    const target = rows(root).find((r) => r.textContent?.includes("needle block target"));
    expect(target).toBeTruthy();
    pressMouse(target!, { button: 0, shiftKey: true });
    await settle();

    expect(switcherOpen()).toBe(false);
    expect(rightSidebarOpen()).toBe(true);
  });

  it("Ctrl/Shift+click on a create row keep only its safe ordinary action", async () => {
    installEmptySearchBackend();
    for (const mod of [{ ctrlKey: true }, { shiftKey: true }]) {
      const { root, dispose } = mount();
      openSwitcher();
      await settle();
      const inputEl = input(root);
      inputEl.value = "ZzzBrandNewPage";
      inputEl.dispatchEvent(new Event("input", { bubbles: true }));
      await settleSearch();

      const createRow = rows(root).find((r) => r.textContent?.includes("Create page"));
      expect(createRow).toBeTruthy();
      const tabCountBefore = paneRouter("main").tabs().length;
      const panesBefore = layoutPaneIds().length;

      pressMouse(createRow!, { button: 0, ...mod });
      await settle(8);

      // Only the ordinary create+open: closed switcher, no fanned background
      // tab, no extra pane, page opened in the origin pane.
      expect(switcherOpen()).toBe(false);
      expect(paneRouter("main").tabs().length).toBe(tabCountBefore);
      expect(layoutPaneIds().length).toBe(panesBefore);
      expect(routeTitle(paneRouter("main").route())).toBe("ZzzBrandNewPage");

      dispose();
      closeSwitcher();
      resetPaneLayoutToSingle(journalsSnapshot());
      document.body.innerHTML = "";
    }
  });

  it("Alt+click on a create row keeps its existing other-pane action", async () => {
    installEmptySearchBackend();
    const { root } = mount();
    openSwitcher();
    await settle();
    const inputEl = input(root);
    inputEl.value = "ZzzBrandNewPage";
    inputEl.dispatchEvent(new Event("input", { bubbles: true }));
    await settleSearch();

    const createRow = rows(root).find((r) => r.textContent?.includes("Create page"));
    expect(createRow).toBeTruthy();

    pressMouse(createRow!, { button: 0, altKey: true });
    await settle(8);

    // Same as Alt+Enter on the create row: the page is created and opened in
    // the other pane — the only modified action create rows ever had.
    expect(switcherOpen()).toBe(false);
    expect(layoutPaneIds().length).toBe(2);
    const otherId = layoutPaneIds().find((id) => id !== "main")!;
    expect(routeTitle(paneRouter(otherId).route())).toBe("ZzzBrandNewPage");
  });

  it("Ctrl+click on a command row runs the ordinary action instead of fanning out", async () => {
    const { root } = mount();
    openCommandPalette();
    await settle();
    const inputEl = input(root);
    inputEl.value = "Toggle dark / light";
    inputEl.dispatchEvent(new Event("input", { bubbles: true }));
    await settle();

    const cmd = rows(root).find((r) => r.textContent?.includes("Toggle dark / light"));
    expect(cmd).toBeTruthy();
    const tabCountBefore = paneRouter("main").tabs().length;

    pressMouse(cmd!, { button: 0, ctrlKey: true });
    await settle();

    expect(switcherOpen()).toBe(false);
    expect(paneRouter("main").tabs().length).toBe(tabCountBefore);
    expect(layoutPaneIds()).toEqual(["main"]);
    expect(rightSidebarOpen()).toBe(false);
  });

  it("keyboard equivalents stay intact: Alt+Enter opens other pane, Shift+Enter opens sidebar", async () => {
    // Alt+Enter
    {
      const { root, dispose } = mount();
      setRecentPages([{ name: "Alpha", kind: "page" }]);
      openSwitcher();
      await settle();

      pressEnter(root, { altKey: true });
      await settle(8);

      expect(switcherOpen()).toBe(false);
      expect(layoutPaneIds().length).toBe(2);
      dispose();
      closeSwitcher();
      resetPaneLayoutToSingle(journalsSnapshot());
      document.body.innerHTML = "";
    }
    // Shift+Enter
    {
      const { root } = mount();
      setRecentPages([{ name: "Alpha", kind: "page" }]);
      openSwitcher();
      await settle();

      pressEnter(root, { shiftKey: true });
      await settle();

      expect(switcherOpen()).toBe(false);
      expect(rightSidebarOpen()).toBe(true);
    }
  });
});
