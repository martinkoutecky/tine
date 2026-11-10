import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PageView } from "./Page";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { bindTestHost } from "../document/host/wiring.test.support";
import { journalTitle, localDayKey } from "../journal";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { endEdit } from "../editorController";
import { PaneContext, paneRouter, resetPaneLayoutToSingle, restorePaneLayout } from "../panes";
import type { PageRead } from "../types";
import { changeCommentsInPopup } from "../commentMarginPref";
import { openCommentThread } from "../tests/commentEntry";

let dispose: (() => void) | undefined;
beforeAll(initParser);
beforeEach(async () => { await bindTestHost(); });
afterEach(() => {
  endEdit("page-navigation"); dispose?.(); resetStore(); resetTabsToJournals(); resetPaneLayoutToSingle();
  changeCommentsInPopup(false);
  document.body.replaceChildren(); vi.restoreAllMocks();
});
function page(date: Date, id: string): PageRead {
  const name = journalTitle(date);
  return { name, title: name, kind: "journal", format: "md", pre_block: null, id: `journals/${id}.md`, rev: id,
    blocks: [{ id, raw: `Passage ${id}`, collapsed: false, children: [
      { id: `${id}-comment`, raw: `Response ${id}\ntine.quote:: Passage`, properties: [["tine.quote", "Passage"]], collapsed: false, children: [] },
    ] }] };
}

it.each(["journals", "page", "zoom", "split-main", "split-secondary"] as const)("the %s surface uses the same margin, popover and sheet tiers", async kind => {
  const today = new Date(); const yesterday = new Date(today); yesterday.setDate(today.getDate() - 1);
  const pages = [page(today, "today"), page(yesterday, "yesterday")];
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages, done: true, next_before_day: null, as_of_day: localDayKey(today) });
  vi.spyOn(backend(), "getPage").mockResolvedValue(pages[0]);
  const route = { kind: "page", name: pages[0].name, pageKind: "journal", ...(kind === "zoom" ? { block: "today" } : {}) } as const;
  if (kind !== "journals") mainPaneRouter.replaceActiveRoute(route);
  const paneId = kind === "split-secondary" ? "pane-2" : "main";
  if (kind.startsWith("split")) restorePaneLayout(
    { kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }] },
    new Map(["main", "pane-2"].map(id => [id, { tabs: [{ history: [route], pos: 0, pinned: false }], activeIndex: 0 }])), paneId,
  );
  let width = 1400;
  const pane = document.createElement("main"); pane.className = "main-content";
  Object.defineProperty(pane, "clientWidth", { get: () => width });
  document.body.append(pane);
  const resize: ResizeObserverCallback[] = [];
  vi.stubGlobal("ResizeObserver", class {
    constructor(cb: ResizeObserverCallback) { resize.push(cb); }
    observe() {} unobserve() {} disconnect() {}
  });
  try {
    dispose = render(() => <PaneContext.Provider value={{ paneId, router: paneRouter(paneId) }}><PageView /></PaneContext.Provider>, pane);
    const count = kind === "journals" ? 2 : 1;
    await vi.waitFor(() => expect(pane.querySelectorAll(".margin-column")).toHaveLength(count));
    for (const section of pane.querySelectorAll(".page-section:has(.margin-column)")) {
      expect(section.querySelectorAll(".margin-column .comment-card")).toHaveLength(1);
      expect(section.querySelector(".page-blocks .comment-card")).toBeNull();
    }
    changeCommentsInPopup(true);
    expect(pane.querySelector(".margin-column")).toBeNull();
    // D-20261108-MC8: in a popup the highlights are the entry points, without counts.
    expect(pane.querySelectorAll(".page-blocks .margin-marker")).toHaveLength(0);
    expect(pane.querySelectorAll(".page-blocks .comment-quote-anchor")).toHaveLength(count);
    openCommentThread(pane);
    expect(document.querySelector(".comment-thread-popup .comment-card")).not.toBeNull();
    document.querySelector<HTMLButtonElement>(".comment-popup-close")!.click();
    changeCommentsInPopup(false);
    expect(pane.querySelectorAll(".margin-column")).toHaveLength(count);
    width = 800;
    for (const cb of [...resize]) cb([], {} as ResizeObserver);
    await vi.waitFor(() => expect(pane.querySelector(".margin-column")).toBeNull());
    expect(pane.querySelectorAll(".page-blocks .comment-card")).toHaveLength(0);
    expect(pane.querySelectorAll(".page-blocks .margin-marker")).toHaveLength(0);
    expect(pane.querySelectorAll(".page-blocks .comment-quote-anchor")).toHaveLength(count);
    openCommentThread(pane);
    expect(document.querySelector(".comment-thread-popup .comment-card")).not.toBeNull();
    document.querySelector<HTMLButtonElement>(".comment-popup-close")!.click();
    width = 390;
    for (const cb of [...resize]) cb([], {} as ResizeObserver);
    // Phones keep a glyph chip as a tap target, without a count.
    await vi.waitFor(() => expect(pane.querySelectorAll(".page-blocks .margin-marker")).toHaveLength(count));
    expect(pane.querySelector(".page-blocks .margin-marker")!.textContent).toBe("");
    pane.querySelector<HTMLButtonElement>(".margin-marker")!.click();
    expect(document.querySelector(".comment-bottom-sheet .comment-card")).not.toBeNull();
  } finally { vi.unstubAllGlobals(); }
});
