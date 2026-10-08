import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InPageFind } from "./components/InPageFind";
import {
  clearInPageFindRenderedTextCacheForTests,
  closeInPageFind,
  inPageFindBlockElement,
  inPageFindMatches,
  openInPageFind,
  setInPageFindQuery,
  inPageFindActiveIndex, stepInPageFind, revealInPageFindMatch, refreshInPageFindHighlights,
} from "./inpageFind";
import { initParser } from "./render/parse";
import { renderedBlockTextCallCountForTests, resetRenderedBlockTextCallCountForTests } from "./render/renderedText";
import { focusPane, resetPaneLayoutToSingle, restorePaneLayout, togglePaneMaximize } from "./panes";
import { resetStore } from "./document";
import { setDoc } from "./document/model";
import { openPageInSidebar, openBlockInSidebar, closeAllRightSidebarItems, sidebarItemKey, rightSidebar, setRightSidebarItemCollapsed, toggleRightSidebar } from "./ui";
import type { PaneSnapshot } from "./router";

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setInPageFindQuery("");
  closeInPageFind({ restoreFocus: false });
  document.body.innerHTML = "";
  clearInPageFindRenderedTextCacheForTests();
  resetRenderedBlockTextCallCountForTests();
  closeAllRightSidebarItems();
  resetStore();
  resetPaneLayoutToSingle(journalsSnapshot());
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("in-page find DOM scoping", () => {
  it("resolves duplicate block DOM under the focused pane captured by opening find", () => {
    restorePaneLayout(
      {
        kind: "split",
        dir: "row",
        ratio: 0.5,
        children: [
          { kind: "pane", paneId: "main" },
          { kind: "pane", paneId: "pane-2" },
        ],
      },
      new Map([["main", pageSnapshot("Shared")], ["pane-2", pageSnapshot("Shared")]]),
      "pane-2"
    );
    document.body.innerHTML = `
      <div data-pane-id="main"><div id="main-hit" class="ls-block" data-block-id="shared"></div></div>
      <div data-pane-id="pane-2"><div id="pane-hit" class="ls-block" data-block-id="shared"></div></div>
    `;
    focusPane("pane-2");

    openInPageFind();

    expect(inPageFindBlockElement("shared")).toBe(document.getElementById("pane-hit"));
  });

  it("GH #559: searches main, split and sidebar in stable order, revealing the right duplicate", async () => {
    const pages = ["Main", "Split", "Side"].map((name) => ({ name, kind: "page" as const, title: name,
      preBlock: null, roots: [name], format: "md" as const, readOnly: false, guide: false }));
    setDoc({ loaded: true, feed: [], pages, byId: Object.fromEntries(pages.map((p) => [p.name,
      { id: p.name, raw: "needle", collapsed: false, parent: null, page: p.name, children: [] }])) });
    restorePaneLayout({ kind: "split", dir: "row", ratio: 0.5, children: [
      { kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" } ] },
      new Map([["main", pageSnapshot("Main")], ["pane-2", pageSnapshot("Split")]]), "pane-2");
    openPageInSidebar("Side", "page");
    const sidebar = `sidebar:${sidebarItemKey(rightSidebar()[0])}`;
    document.body.innerHTML = `<div data-pane-id="main"><div class="ls-block" data-block-id="Main"><div class="block-content">needle</div></div></div>
      <div data-pane-id="pane-2"><div class="ls-block" data-block-id="Split"><div class="block-content">needle</div></div></div>
      <div data-sidebar-surface="${sidebar}"><div class="ls-block" data-block-id="Side"><div class="block-content">needle</div></div></div>`;
    Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
    const scroll = vi.fn();
    document.querySelector<HTMLElement>('[data-block-id="Side"]')!.scrollIntoView = scroll;
    const host = document.createElement("div"); document.body.append(host);
    const dispose = render(() => <InPageFind />, host);
    openInPageFind(); await Promise.resolve();
    const input = host.querySelector<HTMLInputElement>(".inpage-find-input")!;
    input.value = "needle"; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => expect(inPageFindMatches()).toHaveLength(3));
    expect(inPageFindMatches().map((m) => m.blockId)).toEqual(["Main", "Split", "Side"]);
    stepInPageFind(1); expect(inPageFindActiveIndex()).toBe(1);
    stepInPageFind(1); expect(inPageFindActiveIndex()).toBe(2);
    expect(await revealInPageFindMatch(inPageFindMatches()[2])).toBe(true);
    expect(scroll).toHaveBeenCalled();
    stepInPageFind(1); expect(inPageFindActiveIndex()).toBe(0);
    stepInPageFind(-1); expect(inPageFindActiveIndex()).toBe(2);
    expect(host.querySelector(".inpage-find-count")?.textContent).toBe("3 / 3");
    setRightSidebarItemCollapsed(0, true);
    expect(inPageFindMatches().map((m) => m.blockId)).toEqual(["Main", "Split"]);
    setRightSidebarItemCollapsed(0, false);
    toggleRightSidebar();
    expect(inPageFindMatches().map((m) => m.blockId)).toEqual(["Main", "Split"]);
    togglePaneMaximize("pane-2");
    expect(inPageFindMatches().map((m) => m.blockId)).toEqual(["Split"]);
    dispose();
  });

  it("GH #559: repeated blocks in two panes have separate matches and only one active highlight", () => {
    Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
    setDoc({ loaded: true, feed: [], pages: [{ name: "Shared", kind: "page", title: "Shared", preBlock: null,
      roots: ["shared"], format: "md", readOnly: false, guide: false }],
      byId: { shared: { id: "shared", raw: "needle", collapsed: false, parent: null, page: "Shared", children: [] } } });
    restorePaneLayout({ kind: "split", dir: "row", ratio: 0.5, children: [
      { kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" } ] },
      new Map([["main", pageSnapshot("Shared")], ["pane-2", pageSnapshot("Shared")]]), "main");
    document.body.innerHTML = ["main", "pane-2"].map((id) => `<div data-pane-id="${id}"><div class="ls-block" data-block-id="shared"><div class="block-content">needle</div></div></div>`).join("");
    openInPageFind(); setInPageFindQuery("needle");
    expect(inPageFindMatches().map((m) => m.scopeId)).toEqual(["main", "pane-2"]);
    stepInPageFind(1); refreshInPageFindHighlights();
    expect(document.querySelectorAll(".inpage-find-active-block")).toHaveLength(1);
    expect(document.querySelector(".inpage-find-active-block")?.closest<HTMLElement>("[data-pane-id]")?.dataset.paneId).toBe("pane-2");
  });

  it("GH #559: a sidebar block searches its subtree without unrelated page siblings", () => {
    const page = { name: "Side", kind: "page" as const, title: "Side", preBlock: null, roots: ["root", "sibling"],
      format: "md" as const, readOnly: false, guide: false };
    setDoc({ loaded: true, feed: [], pages: [page], byId: {
      root: { id: "root", raw: "needle", collapsed: false, parent: null, page: "Side", children: ["child"] },
      child: { id: "child", raw: "needle child", collapsed: false, parent: "root", page: "Side", children: [] },
      sibling: { id: "sibling", raw: "needle elsewhere", collapsed: false, parent: null, page: "Side", children: [] },
    } });
    openBlockInSidebar({ uuid: "root", page: "Side", pageKind: "page" });
    const scope = `sidebar:${sidebarItemKey(rightSidebar()[0])}`;
    document.body.innerHTML = `<div data-sidebar-surface="${scope}"></div>`;
    openInPageFind(); setInPageFindQuery("needle");
    expect(inPageFindMatches().map((m) => m.blockId)).toEqual(["root", "child"]);
  });

  it("debounces rapid query input to one in-page find scan", async () => {
    vi.useFakeTimers();
    const byId = Object.fromEntries(
      Array.from({ length: 64 }, (_, i) => [
        `block-${i}`,
        { id: `block-${i}`, raw: `needle target ${i}`, collapsed: false, parent: null, page: "Pane", children: [] },
      ])
    );
    setDoc({
      loaded: true,
      feed: ["Pane"],
      pages: [
        {
          name: "Pane",
          kind: "page",
          title: "Pane",
          preBlock: null,
          roots: Object.keys(byId),
          format: "md",
          readOnly: false,
          guide: false,
        },
      ],
      byId,
    });
    resetPaneLayoutToSingle(pageSnapshot("Pane"));
    focusPane("main");
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <InPageFind />, host);
    try {
      openInPageFind();
      await Promise.resolve();
      const input = host.querySelector(".inpage-find-input") as HTMLInputElement;
      expect(input).toBeTruthy();

      for (const q of ["n", "ne", "nee", "need"]) {
        input.value = q;
        input.dispatchEvent(new InputEvent("input", { bubbles: true }));
        await vi.advanceTimersByTimeAsync(25);
      }

      expect(renderedBlockTextCallCountForTests()).toBe(0);
      await vi.advanceTimersByTimeAsync(110);
      expect(inPageFindMatches()).toHaveLength(64);
      expect(renderedBlockTextCallCountForTests()).toBe(64);
    } finally {
      dispose();
      host.remove();
    }
  });

  it("keeps the caret collapsed while typing and selects all only on an explicit refocus (GH #224)", async () => {
    vi.useFakeTimers();
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <InPageFind />, host);
    try {
      openInPageFind();
      await Promise.resolve();
      const input = host.querySelector(".inpage-find-input") as HTMLInputElement;
      expect(input).toBeTruthy();

      input.value = "a";
      input.setSelectionRange(1, 1);
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await vi.advanceTimersByTimeAsync(110);
      await Promise.resolve();

      expect(input.value).toBe("a");
      expect([input.selectionStart, input.selectionEnd]).toEqual([1, 1]);

      // Repeating Ctrl+F is a distinct semantic request: focus the existing
      // query and select it so the user can replace it in one keystroke.
      openInPageFind();
      await Promise.resolve();
      expect([input.selectionStart, input.selectionEnd]).toEqual([0, 1]);
    } finally {
      dispose();
      host.remove();
    }
  });
});
