import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { initParser } from "../render/parse";
import { renderInlines } from "../render/inline";
import { resetStore } from "../document";
import { backend } from "../backend";
import { openPage } from "../router";
import { layoutPaneIds, paneRouter, resetPaneLayoutToSingle } from "../panes";
import { rightSidebar, setRightSidebar, setRightSidebarOpen, setFavorites } from "../ui";
import { Sidebar } from "./Sidebar";
import { NamespaceCrumb } from "./Namespace";
import { LinkedReferences } from "./LinkedReferences";
import { QueryMacro } from "./Macro";
import type { RefGroup } from "../types";
import { backendReadsQueries } from "../tests/queryReadingsTestkit";
import { blockRunResult } from "../tests/queryReadingsTestkit";

// GH #283 (approved contract): ONE modified-click decision across internal
// page/block link surfaces:
//   plain click   → ordinary navigation;
//   Shift+click   → right sidebar;
//   Ctrl/Cmd(Linux/macOS)+click → background tab;
//   middle-click  → background tab.
// GH #438 adds the missing fourth destination, matching the Search / Quick
// Switcher ladder (GH #288):
//   Alt+click     → the other pane (splitting right when there is only one).
// Context menus still expose explicit destinations; text selection / editor
// focus are preserved the same way they always were.

const journalsSnapshot = () => ({
  tabs: [{ history: [{ kind: "journals" as const }], pos: 0, pinned: false }],
  activeIndex: 0,
});

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setRightSidebar([]);
  setRightSidebarOpen(false);
  setFavorites([]);
  resetPaneLayoutToSingle(journalsSnapshot());
  localStorage.clear();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function tabsCount(): number {
  return paneRouter("main").tabs().length;
}

function activeRouteName(): string | null {
  const r = paneRouter("main").route();
  return r.kind === "page" ? r.name : r.kind;
}

function newBackgroundPage(): string | null {
  const tabs = paneRouter("main").tabs();
  const active = activeRouteName();
  const other = tabs.find((t) => {
    const r = t.history[t.pos];
    return r.kind === "page" && r.name !== active;
  });
  return other ? (other.history[other.pos] as { name: string }).name : null;
}

function mountPageRef(): { root: HTMLDivElement; dispose: () => void; anchor: () => HTMLElement } {
  const m = mount(() =>
    renderInlines([{ k: "link", url: { type: "page_ref", v: "Target Page" }, full: "[[Target Page]]" }]) as JSX.Element
  );
  return { ...m, anchor: () => m.root.querySelector<HTMLElement>("a.page-ref")! };
}

function click(el: HTMLElement, init: MouseEventInit = {}) {
  el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ...init }));
}

function auxMiddle(el: HTMLElement) {
  el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 1 }));
  el.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
}

describe("modified-click contract on internal links (GH #283)", () => {
  it("plain click navigates; Shift+click opens the right sidebar", async () => {
    const { anchor, dispose } = mountPageRef();
    try {
      click(anchor());
      expect(activeRouteName()).toBe("Target Page");
      expect(rightSidebar()).toHaveLength(0);

      openPage("Elsewhere", "page");
      click(anchor(), { shiftKey: true });
      expect(rightSidebar().length).toBeGreaterThan(0);
      const item = rightSidebar()[0];
      expect(item.kind === "page" ? item.name : null).toBe("Target Page");
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      dispose();
    }
  });

  it.each([{ ctrlKey: true }, { metaKey: true }])("component-click variant (%s) opens a background tab and never steals focus", async (mod) => {
    const { anchor, dispose } = mountPageRef();
    try {
      const before = tabsCount();
      openPage("Elsewhere", "page");
      click(anchor(), mod);
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Elsewhere");
      expect(newBackgroundPage()).toBe("Target Page");
    } finally {
      dispose();
    }
  });

  it("combined modifiers (shift+ctrl) do not shadow anything — plain click semantics", async () => {
    const { anchor, dispose } = mountPageRef();
    try {
      const before = tabsCount();
      click(anchor(), { shiftKey: true, ctrlKey: true });
      expect(activeRouteName()).toBe("Target Page"); // navigated like a plain click
      expect(tabsCount()).toBe(before);
      expect(rightSidebar()).toHaveLength(0);
    } finally {
      dispose();
    }
  });

  it("middle-click on a page ref opens a background tab (existing convention, asserted)", async () => {
    const { anchor, dispose } = mountPageRef();
    try {
      openPage("Elsewhere", "page");
      const before = tabsCount();
      auxMiddle(anchor());
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Elsewhere");
      expect(newBackgroundPage()).toBe("Target Page");
    } finally {
      dispose();
    }
  });

  it("block refs follow the same rule: Ctrl+click background tab with block anchor, middle too", async () => {
    const group: RefGroup = {
      page: "Anchor Page",
      kind: "page",
      blocks: [{ id: "58900000-0000-4000-8000-0000000000a1", raw: "referenced content", collapsed: false, children: [] }],
    };
    vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map((id) => (id === "58900000-0000-4000-8000-0000000000a1" ? group : null)));
    const m = mount(() =>
      renderInlines([{ k: "link", url: { type: "block_ref", v: "58900000-0000-4000-8000-0000000000a1" }, full: "((58900000-0000-4000-8000-0000000000a1))" }]) as JSX.Element
    );
    try {
      const anchor = await vi.waitFor(() => {
        const a = m.root.querySelector<HTMLElement>("span.block-ref");
        expect(a?.textContent).toContain("referenced content");
        return a!;
      });
      const before = tabsCount();
      openPage("Elsewhere", "page");
      anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, ctrlKey: true }));
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Elsewhere");
      const bg = paneRouter("main").tabs().find((t) => {
        const r = t.history[t.pos];
        return r.kind === "page" && r.name === "Anchor Page";
      });
      expect(bg).toBeTruthy();
      // middle-click gets the same background treatment.
      const before2 = tabsCount();
      auxMiddle(anchor);
      expect(tabsCount()).toBe(before2 + 1);
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      m.dispose();
    }
  });

  it("namespace crumbs follow the same rule (a real link surface", async () => {
    const m = mount(() => <NamespaceCrumb name="a/b/c" />);
    try {
      const crumb = m.root.querySelector<HTMLElement>(".ns-crumb-item");
      expect(crumb).not.toBeNull();
      openPage("Elsewhere", "page");
      const before = tabsCount();
      crumb!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, ctrlKey: true }));
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Elsewhere");
      expect(newBackgroundPage()).toBe("a");
    } finally {
      m.dispose();
    }
  });

  it("Favorites rows: Shift→sidebar, Ctrl→background tab", async () => {
    setFavorites([{ name: "Fav One", kind: "page" }]);
    const m = mount(() => <Sidebar />);
    try {
      // og: the whole favorite row is the link (master GH #464 makes only the title one).
      const row = [...m.root.querySelectorAll<HTMLElement>("#sidebar-favorites-list .nav-page")][0];
      expect(row).toBeTruthy();
      click(row, { shiftKey: true });
      expect(rightSidebar().length).toBeGreaterThan(0);
      setRightSidebar([]);
      const before = tabsCount();
      click(row, { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      expect(newBackgroundPage()).toBe("Fav One");
    } finally {
      m.dispose();
    }
  });

  it("linked-reference page headers use the shared background-tab destination", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([{
      page: "Backlink Owner",
      kind: "page",
      blocks: [{ id: "backlink-1", raw: "mentions [[Target]]", collapsed: false, children: [] }],
    }]);
    const m = mount(() => <LinkedReferences name="Target" />);
    try {
      const header = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".reference-page");
        expect(el?.textContent).toContain("Backlink Owner");
        return el!;
      });
      openPage("Elsewhere", "page");
      const before = tabsCount();
      click(header, { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Elsewhere");
      expect(newBackgroundPage()).toBe("Backlink Owner");
    } finally {
      m.dispose();
    }
  });

  it("legacy query-table page cells use the shared background-tab destination", async () => {
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([{
      page: "Query Owner",
      kind: "page",
      blocks: [{ id: "query-hit", raw: "TODO row", collapsed: false, children: [] }],
    }]));
    backendReadsQueries({
      "(task TODO) {:table-view? true}": { form: "(task TODO)", opts: "{:table-view? true}", legacy_table: true },
    });
    const m = mount(() => <QueryMacro body={'query (task TODO) {:table-view? true}'} />);
    try {
      const cell = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".qt-page");
        expect(el?.textContent).toContain("Query Owner");
        return el!;
      });
      openPage("Elsewhere", "page");
      const before = tabsCount();
      click(cell, { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Elsewhere");
      expect(newBackgroundPage()).toBe("Query Owner");
    } finally {
      m.dispose();
    }
  });
});

// GH #438: Alt+click is the mouse+modifier route to the OTHER pane on every
// internal page/block link — the same destination Alt+click/Alt+Enter already
// had on Search and Quick Switcher results (GH #288). The click stays at the
// gesture/routing boundary: surfaces keep deciding from the shared
// internalLinkDest() ladder and openRouteInOtherPane does the pane work.
describe("Alt+click opens the link in the other pane (GH #438)", () => {
  function otherPaneRoute(): { kind: string; name?: string; block?: string } | null {
    const other = layoutPaneIds().find((id) => id !== "main");
    if (!other) return null;
    return paneRouter(other).route() as { kind: string; name?: string; block?: string };
  }

  it("Alt+click on a page ref opens the target in the other pane — origin pane and sidebars untouched", () => {
    const { anchor, dispose } = mountPageRef();
    try {
      openPage("Elsewhere", "page");
      const mainTabs = tabsCount();
      click(anchor(), { altKey: true });
      expect(layoutPaneIds().length).toBe(2);
      const route = otherPaneRoute();
      expect(route?.kind).toBe("page");
      expect(route?.name).toBe("Target Page");
      // The origin pane keeps its foreground: a copy, not a move.
      expect(activeRouteName()).toBe("Elsewhere");
      expect(tabsCount()).toBe(mainTabs);
      expect(rightSidebar()).toHaveLength(0);
    } finally {
      dispose();
    }
  });

  it("Alt+click opens at the referenced block on block refs", async () => {
    const group: RefGroup = {
      page: "Anchor Page",
      kind: "page",
      blocks: [{ id: "58900000-0000-4000-8000-0000000000a1", raw: "referenced content", collapsed: false, children: [] }],
    };
    vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map((id) => (id === "58900000-0000-4000-8000-0000000000a1" ? group : null)));
    const m = mount(() =>
      renderInlines([{ k: "link", url: { type: "block_ref", v: "58900000-0000-4000-8000-0000000000a1" }, full: "((58900000-0000-4000-8000-0000000000a1))" }]) as JSX.Element
    );
    try {
      const anchor = await vi.waitFor(() => {
        const a = m.root.querySelector<HTMLElement>("span.block-ref");
        expect(a?.textContent).toContain("referenced content");
        return a!;
      });
      openPage("Elsewhere", "page");
      click(anchor, { altKey: true });
      expect(layoutPaneIds().length).toBe(2);
      const route = otherPaneRoute();
      expect(route?.kind).toBe("page");
      expect(route?.name).toBe("Anchor Page");
      // The click surface hands the block anchor to openRouteInOtherPane; the
      // helper's own contract (a fresh split opens the page at the block, an
      // existing pane keeps the anchor on the tab) is deliberately NOT pinned
      // here — the frontier lane replaces it after merge (GH #438 scope).
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      m.dispose();
    }
  });

  it("modifier precedence matches the switcher ladder (GH #288): a pure Shift/Ctrl still wins, Alt claims the rest", () => {
    const { anchor, dispose } = mountPageRef();
    try {
      openPage("Elsewhere", "page");
      // Ctrl+Alt is still a pane, like the switcher's fall-through Alt branch —
      // never a background tab.
      const mainTabs = tabsCount();
      click(anchor(), { altKey: true, ctrlKey: true });
      expect(layoutPaneIds().length).toBe(2);
      expect(otherPaneRoute()?.name).toBe("Target Page");
      expect(tabsCount()).toBe(mainTabs);
      // Shift+Alt also goes to the pane, never the sidebar.
      click(anchor(), { altKey: true, shiftKey: true });
      expect(otherPaneRoute()?.name).toBe("Target Page");
      expect(rightSidebar()).toHaveLength(0);
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      dispose();
    }
  });

  it("Alt+click on a namespace crumb opens the ancestor page in the other pane", () => {
    const m = mount(() => <NamespaceCrumb name="a/b/c" />);
    try {
      const crumb = m.root.querySelector<HTMLElement>(".ns-crumb-item");
      expect(crumb).not.toBeNull();
      openPage("Elsewhere", "page");
      click(crumb!, { altKey: true });
      expect(layoutPaneIds().length).toBe(2);
      expect(otherPaneRoute()?.name).toBe("a");
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      m.dispose();
    }
  });

  it("Alt+click on a Favorites row opens the page in the other pane", () => {
    setFavorites([{ name: "Fav One", kind: "page" }]);
    const m = mount(() => <Sidebar />);
    try {
      // og: the whole favorite row is the link (master GH #464 makes only the title one).
      const row = [...m.root.querySelectorAll<HTMLElement>("#sidebar-favorites-list .nav-page")][0];
      expect(row).toBeTruthy();
      openPage("Elsewhere", "page");
      click(row, { altKey: true });
      expect(layoutPaneIds().length).toBe(2);
      expect(otherPaneRoute()?.name).toBe("Fav One");
      expect(activeRouteName()).toBe("Elsewhere");
      expect(rightSidebar()).toHaveLength(0);
    } finally {
      m.dispose();
    }
  });

  it("Alt+click on a linked-reference page header opens the source page in the other pane", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([{
      page: "Backlink Owner",
      kind: "page",
      blocks: [{ id: "backlink-1", raw: "mentions [[Target]]", collapsed: false, children: [] }],
    }]);
    const m = mount(() => <LinkedReferences name="Target" />);
    try {
      const header = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".reference-page");
        expect(el?.textContent).toContain("Backlink Owner");
        return el!;
      });
      openPage("Elsewhere", "page");
      click(header, { altKey: true });
      expect(layoutPaneIds().length).toBe(2);
      expect(otherPaneRoute()?.name).toBe("Backlink Owner");
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      m.dispose();
    }
  });

  it("Alt+click on a legacy query-table page cell opens the hit page in the other pane", async () => {
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([{
      page: "Query Owner",
      kind: "page",
      blocks: [{ id: "query-hit", raw: "TODO row", collapsed: false, children: [] }],
    }]));
    backendReadsQueries({
      "(task TODO) {:table-view? true}": { form: "(task TODO)", opts: "{:table-view? true}", legacy_table: true },
    });
    const m = mount(() => <QueryMacro body={'query (task TODO) {:table-view? true}'} />);
    try {
      const cell = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".qt-page");
        expect(el?.textContent).toContain("Query Owner");
        return el!;
      });
      openPage("Elsewhere", "page");
      click(cell, { altKey: true });
      expect(layoutPaneIds().length).toBe(2);
      expect(otherPaneRoute()?.name).toBe("Query Owner");
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      m.dispose();
    }
  });
});
