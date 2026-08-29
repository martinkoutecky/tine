import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import { mainPaneRouter, openPage } from "../router";
import { resetPaneLayoutToSingle } from "../panes";
import {
  applySidebarSession,
  bumpDataRev,
  closeContextMenu,
  contextMenu,
  rightSidebar,
  setFavorites,
  setRightSidebar,
  setRightSidebarOpen,
} from "../ui";
import { Sidebar } from "./Sidebar";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { BlockReferences } from "./BlockReferences";
import { Block } from "./Block";
import { NamespaceHierarchy, NamespaceMacro } from "./Namespace";
import { RightSidebar } from "./RightSidebar";
import { PageView } from "./Page";
import type { PageEntry, QueryExecution } from "../types";
import { LONG_PRESS_DELAY } from "../render/longPress";

// GH #207: the internal-link gesture contract (linkGesture.ts, GH #283) is ONE
// decision — plain click opens, Shift+click → right sidebar, Ctrl/Cmd+click or
// middle-click → background tab, and the middle/shift mousedown defaults
// (autoscroll, range-selection) are suppressed up front. These tests pin the
// surfaces that had drifted off the contract: unlinked-reference page headers,
// namespace macro/hierarchy links, the zoom breadcrumb, the right-sidebar item
// title, query search-presentation rows, and the sidebar/reference/query rows
// whose middle-mousedown still reached the browser (autoscroll).

const journalsSnapshot = () => ({
  tabs: [{ history: [{ kind: "journals" as const }], pos: 0, pinned: false }],
  activeIndex: 0,
});

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  closeContextMenu();
  setRightSidebar([]);
  setRightSidebarOpen(false);
  setFavorites([]);
  applySidebarSession({ right: false, items: [] });
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
  return mainPaneRouter.tabs().length;
}

function activeRouteName(): string | null {
  const r = mainPaneRouter.route();
  return r.kind === "page" ? r.name : r.kind;
}

function backgroundRoutes(): { name: string; block?: string }[] {
  const active = activeRouteName();
  return mainPaneRouter.tabs()
    .map((t) => t.history[t.pos])
    .filter((r) => r.kind === "page" && r.name !== active)
    .map((r) => ({ name: (r as { name: string }).name, block: (r as { block?: string }).block }));
}

function sidebarPageNames(): string[] {
  return rightSidebar().map((item) => (item.kind === "page" ? item.name : item.page));
}

function click(el: HTMLElement, init: MouseEventInit = {}) {
  el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ...init }));
}

function auxMiddle(el: HTMLElement) {
  el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 1 }));
  el.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
}

function touch(type: string, x = 20, y = 30): PointerEvent {
  return new PointerEvent(type, {
    bubbles: true,
    cancelable: true,
    pointerType: "touch",
    isPrimary: true,
    pointerId: 7,
    clientX: x,
    clientY: y,
  });
}

/** Dispatch a middle-button mousedown and report whether the browser default
 *  (autoscroll / PRIMARY-paste) was suppressed. */
function middleDefaultPrevented(el: HTMLElement): boolean {
  const e = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 1 });
  el.dispatchEvent(e);
  return e.defaultPrevented;
}

function page(name: string, roots: string[]): FeedPage {
  return { name, kind: "page", title: name, preBlock: null, roots, format: "md", readOnly: false, guide: false };
}

function node(id: string, raw: string, pageName: string, parent: string | null = null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: pageName, children };
}

function seedNamespaceInventory(names: string[]) {
  const entries: PageEntry[] = names.map((name) => ({
    name,
    kind: "page",
    date_key: null,
    path: `pages/${name.replaceAll("/", "___")}.md`,
  }));
  vi.spyOn(backend(), "listPages").mockResolvedValue(entries);
  vi.spyOn(backend(), "referencedPageNames").mockResolvedValue({ digest: 1, names });
  vi.spyOn(backend(), "pageIcons").mockResolvedValue({});
  bumpDataRev();
}

describe("reference page headers follow the gesture contract (GH #207)", () => {
  it("unlinked-reference page header: middle/ctrl → background tab, shift → sidebar, mousedown default suppressed", async () => {
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{
      page: "Source",
      kind: "page",
      blocks: [{ id: "u1", raw: "plain mention of Target", collapsed: false, children: [] }],
    }]);
    const m = mount(() => <UnlinkedReferences name="Target" />);
    try {
      m.root.querySelector<HTMLElement>(".references-header")!.click();
      const header = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".reference-page");
        expect(el?.textContent).toContain("Source");
        return el!;
      });
      expect(middleDefaultPrevented(header)).toBe(true);

      openPage("Elsewhere", "page");
      let before = tabsCount();
      auxMiddle(header);
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Elsewhere");
      expect(backgroundRoutes().map((r) => r.name)).toContain("Source");

      before = tabsCount();
      click(header, { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Elsewhere");

      click(header, { shiftKey: true });
      expect(activeRouteName()).toBe("Elsewhere");
      expect(sidebarPageNames()).toContain("Source");

      // Sibling parity with linked references: right-click offers the page menu.
      header.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
      expect(contextMenu()).toMatchObject({ kind: "page", name: "Source", pageKind: "page" });
    } finally {
      m.dispose();
    }
  });

  it("linked-reference page header suppresses the middle-mousedown default", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([{
      page: "Backlink Owner",
      kind: "page",
      blocks: [{ id: "b1", raw: "mentions [[Target]]", collapsed: false, children: [] }],
    }]);
    const m = mount(() => <LinkedReferences name="Target" />);
    try {
      const header = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".reference-page");
        expect(el?.textContent).toContain("Backlink Owner");
        return el!;
      });
      expect(header.hasAttribute("data-page-context-menu")).toBe(true);
      expect(middleDefaultPrevented(header)).toBe(true);

      vi.useFakeTimers();
      openPage("Elsewhere", "page");
      header.dispatchEvent(touch("pointerdown"));
      vi.advanceTimersByTime(LONG_PRESS_DELAY);
      expect(contextMenu()).toMatchObject({ kind: "page", name: "Backlink Owner" });
      header.dispatchEvent(touch("pointerup"));
      header.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      expect(activeRouteName()).toBe("Elsewhere");
      vi.useRealTimers();
    } finally {
      vi.useRealTimers();
      m.dispose();
    }
  });

  it("block-references page header suppresses the middle-mousedown default", async () => {
    vi.spyOn(backend(), "getBlockReferrers").mockResolvedValue([{
      page: "Referrer",
      kind: "page",
      blocks: [{ id: "b1", raw: "refs ((block-x))", collapsed: false, children: [] }],
    }]);
    const m = mount(() => <BlockReferences id="block-x" />);
    try {
      const header = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".reference-page");
        expect(el?.textContent).toContain("Referrer");
        return el!;
      });
      expect(middleDefaultPrevented(header)).toBe(true);
    } finally {
      m.dispose();
    }
  });
});

describe("sidebar rows suppress the middle-mousedown default (GH #207)", () => {
  it("favorites row and journals row suppress autoscroll up front", () => {
    setFavorites([{ name: "Fav One", kind: "page" }]);
    const m = mount(() => <Sidebar />);
    try {
      const fav = m.root.querySelector<HTMLElement>("#sidebar-favorites-list .nav-page")!;
      expect(fav).toBeTruthy();
      expect(middleDefaultPrevented(fav)).toBe(true);
      const journals = [...m.root.querySelectorAll<HTMLElement>(".nav-item")]
        .find((el) => el.textContent?.includes("Journals"))!;
      expect(middleDefaultPrevented(journals)).toBe(true);
    } finally {
      m.dispose();
    }
  });
});

describe("namespace links follow the gesture contract (GH #207)", () => {
  it("namespace macro head + node links: middle/ctrl → background tab, shift → sidebar", async () => {
    seedNamespaceInventory(["ns/a", "ns/a/b"]);
    const m = mount(() => <NamespaceMacro root="ns" />);
    try {
      const head = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".ns-macro-head .page-ref");
        expect(el?.textContent?.trim()).toContain("ns");
        return el!;
      });
      openPage("Elsewhere", "page");
      let before = tabsCount();
      auxMiddle(head);
      expect(tabsCount()).toBe(before + 1);
      expect(backgroundRoutes().map((r) => r.name)).toContain("ns");
      before = tabsCount();
      click(head, { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      click(head, { shiftKey: true });
      expect(sidebarPageNames()).toContain("ns");

      const node = m.root.querySelector<HTMLElement>(".ns-macro-node .page-ref")!;
      expect(node.textContent?.trim()).toContain("a");
      before = tabsCount();
      auxMiddle(node);
      expect(tabsCount()).toBe(before + 1);
      expect(backgroundRoutes().map((r) => r.name)).toContain("ns/a");
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      m.dispose();
    }
  });

  it("namespace hierarchy path links: middle/ctrl → background tab, shift → sidebar", async () => {
    seedNamespaceInventory(["ns/a/b"]);
    const m = mount(() => <NamespaceHierarchy name="ns/a" />);
    try {
      const links = await vi.waitFor(() => {
        const found = [...m.root.querySelectorAll<HTMLElement>(".ns-hier-row .page-ref")];
        expect(found.length).toBe(3);
        return found;
      });
      openPage("Elsewhere", "page");
      let before = tabsCount();
      auxMiddle(links[0]);
      expect(tabsCount()).toBe(before + 1);
      expect(backgroundRoutes().map((r) => r.name)).toContain("ns");
      before = tabsCount();
      click(links[1], { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      click(links[2], { shiftKey: true });
      expect(sidebarPageNames()).toContain("ns/a/b");
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      m.dispose();
    }
  });
});

describe("zoom breadcrumb follows the gesture contract (GH #207)", () => {
  it("crumb-page and ancestor crumbs: middle/ctrl → background tab, shift → sidebar", async () => {
    setDoc({
      byId: {
        r1: node("r1", "Ancestor text", "Host", null, ["z1"]),
        z1: node("z1", "Zoom root", "Host", "r1"),
      },
      pages: [page("Host", ["r1"])],
      feed: [],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue({
      name: "Host",
      kind: "page",
      title: "Host",
      pre_block: null,
      blocks: [
        { id: "r1", raw: "Ancestor text", collapsed: false, children: [{ id: "z1", raw: "Zoom root", collapsed: false, children: [] }] },
      ],
    });
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Host", pageKind: "page", block: "z1" });
    const m = mount(() => <PageView />);
    // A background open must never move the FOREGROUND off the zoomed block:
    // count host-page routes (with/without anchor) instead of navigating away.
    const hostRoutes = (withBlock: boolean) =>
      mainPaneRouter.tabs()
        .map((t) => t.history[t.pos])
        .filter((r) => r.kind === "page" && r.name === "Host" && ("block" in r ? !!r.block : false) === withBlock);
    const foregroundBlock = () =>
      mainPaneRouter.route().kind === "page" ? (mainPaneRouter.route() as { block?: string }).block : undefined;
    try {
      const crumbPage = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".zoom-breadcrumb .crumb-page");
        expect(el?.textContent).toContain("Host");
        return el!;
      });
      const ancestor = m.root.querySelector<HTMLElement>(".zoom-breadcrumb .crumb:not(.crumb-page)")!;
      expect(ancestor.textContent).toContain("Ancestor text");

      // crumb-page middle/ctrl: a plain Host page tab in the background.
      let before = hostRoutes(false).length;
      auxMiddle(crumbPage);
      expect(hostRoutes(false).length).toBe(before + 1);
      expect(foregroundBlock()).toBe("z1");
      before = hostRoutes(false).length;
      click(crumbPage, { ctrlKey: true });
      expect(hostRoutes(false).length).toBe(before + 1);
      expect(foregroundBlock()).toBe("z1");

      click(crumbPage, { shiftKey: true });
      expect(sidebarPageNames()).toContain("Host");
      setRightSidebar([]);
      click(ancestor, { shiftKey: true });
      expect(rightSidebar().some((item) => item.kind === "block" && item.page === "Host")).toBe(true);

      // ancestor crumb middle: a Host tab anchored at that block, in background.
      before = hostRoutes(true).length;
      auxMiddle(ancestor);
      expect(hostRoutes(true).length).toBe(before + 1);
      expect(foregroundBlock()).toBe("z1");
      const anchored = mainPaneRouter.tabs()
        .map((t) => t.history[t.pos])
        .filter((r) => r.kind === "page" && r.name === "Host" && "block" in r)
        .map((r) => (r as { block?: string }).block);
      expect(anchored).toContain("r1");
    } finally {
      m.dispose();
    }
  });
});

describe("right-sidebar item title follows the gesture contract (GH #207)", () => {
  it("middle/ctrl → background tab; shift keeps the ordinary main-pane navigation", async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue({
      name: "Twin",
      kind: "page",
      title: "Twin",
      pre_block: null,
      blocks: [{ id: "twin-b", raw: "body", collapsed: false, children: [] }],
    });
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
    applySidebarSession({ right: true, items: [{ kind: "page", name: "Twin", pageKind: "page" }] });
    const m = mount(() => <RightSidebar />);
    try {
      const title = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".rs-item-title");
        expect(el?.textContent).toContain("Twin");
        return el!;
      });
      openPage("Elsewhere", "page");
      let before = tabsCount();
      auxMiddle(title);
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Elsewhere");
      expect(backgroundRoutes().map((r) => r.name)).toContain("Twin");
      before = tabsCount();
      click(title, { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      // Shift is the sidebar destination, which is meaningless for a title that
      // already lives in the sidebar: it keeps the ordinary navigation.
      click(title, { shiftKey: true });
      expect(activeRouteName()).toBe("Twin");
      expect(rightSidebar()).toHaveLength(1);
    } finally {
      m.dispose();
    }
  });
});

describe("query search-presentation rows follow the gesture contract (GH #207)", () => {
  function loadQueryDoc(raw: string, id = "query") {
    setDoc({
      byId: { [id]: node(id, raw, "Sheet") },
      pages: [page("Sheet", [id])],
      feed: ["Sheet"],
      loaded: true,
    });
  }

  it("block-hit rows: middle/ctrl → background tab with block anchor, shift → sidebar", async () => {
    loadQueryDoc("{{query (task TODO)}}\ntine.view:: search");
    vi.spyOn(backend(), "runQuery").mockResolvedValue([{
      page: "Sheet",
      kind: "page",
      blocks: [{ id: "b1", raw: "Body hit", collapsed: false, children: [] }],
    }]);
    const m = mount(() => <Block id="query" />);
    try {
      const row = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".query-search-hit");
        expect(el?.textContent).toContain("Body hit");
        return el!;
      });
      openPage("Elsewhere", "page");
      let before = tabsCount();
      auxMiddle(row);
      expect(tabsCount()).toBe(before + 1);
      const bg = backgroundRoutes().find((r) => r.name === "Sheet");
      expect(bg?.block).toBe("b1");
      before = tabsCount();
      click(row, { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      click(row, { shiftKey: true });
      expect(rightSidebar().some((item) => item.kind === "block" && item.page === "Sheet")).toBe(true);
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      m.dispose();
    }
  });

  it("page-hit rows: middle/ctrl → background tab, shift → sidebar", async () => {
    const execution: QueryExecution = {
      hits: [{
        entity: "page",
        page: { name: "Twin", kind: "page", date_key: null, path: "pages/Twin.md" },
        display_text: "Twin",
        evidence: [],
        score: 1,
      }],
      diagnostics: [],
      explanation: { branches: [] },
      cancelled: false,
    };
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue(execution);
    loadQueryDoc('{{query (search "Twin")}}\ntine.view:: search');
    const m = mount(() => <Block id="query" />);
    try {
      const row = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".query-search-page");
        expect(el?.textContent).toContain("Twin");
        return el!;
      });
      openPage("Elsewhere", "page");
      let before = tabsCount();
      auxMiddle(row);
      expect(tabsCount()).toBe(before + 1);
      expect(backgroundRoutes().map((r) => r.name)).toContain("Twin");
      before = tabsCount();
      click(row, { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      click(row, { shiftKey: true });
      expect(sidebarPageNames()).toContain("Twin");
      expect(activeRouteName()).toBe("Elsewhere");
    } finally {
      m.dispose();
    }
  });

  it("legacy table page cells and list headers suppress the middle-mousedown default", async () => {
    vi.spyOn(backend(), "runQuery").mockResolvedValue([{
      page: "Query Owner",
      kind: "page",
      blocks: [{ id: "q1", raw: "TODO row", collapsed: false, children: [] }],
    }]);
    loadQueryDoc("{{query (task TODO) {:table-view? true}}}");
    const table = mount(() => <Block id="query" />);
    try {
      const cell = await vi.waitFor(() => {
        const el = table.root.querySelector<HTMLElement>(".qt-page");
        expect(el?.textContent).toContain("Query Owner");
        return el!;
      });
      expect(middleDefaultPrevented(cell)).toBe(true);
    } finally {
      table.dispose();
    }

    loadQueryDoc("{{query (task DONE)}}", "query2");
    const list = mount(() => <Block id="query2" />);
    try {
      const header = await vi.waitFor(() => {
        const el = list.root.querySelector<HTMLElement>(".query-page");
        expect(el?.textContent).toContain("Query Owner");
        return el!;
      });
      expect(middleDefaultPrevented(header)).toBe(true);
    } finally {
      list.dispose();
    }
  });
});

describe("page title ctrl-click matches the contract (GH #207)", () => {
  it("ctrl/cmd+click on the page title opens a background tab; plain and shift clicks unchanged", async () => {
    setDoc({
      byId: { "twin-b": node("twin-b", "Body", "Twin") },
      pages: [page("Twin", ["twin-b"])],
      feed: [],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue({
      name: "Twin",
      kind: "page",
      title: "Twin",
      pre_block: null,
      blocks: [{ id: "twin-b", raw: "Body", collapsed: false, children: [] }],
    });
    mainPaneRouter.openPage("Twin", "page", { inPlace: true });
    const m = mount(() => <PageView />);
    try {
      const title = await vi.waitFor(() => {
        const el = m.root.querySelector<HTMLElement>(".page-title");
        expect(el?.textContent).toContain("Twin");
        return el!;
      });
      const before = tabsCount();
      click(title, { ctrlKey: true });
      expect(tabsCount()).toBe(before + 1);
      expect(activeRouteName()).toBe("Twin"); // background: the foreground stays put
      click(title, { shiftKey: true });
      expect(sidebarPageNames()).toContain("Twin");
      click(title);
      expect(activeRouteName()).toBe("Twin");
    } finally {
      m.dispose();
    }
  });
});
