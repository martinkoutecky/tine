import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For } from "solid-js";
import { InPageFind } from "./InPageFind";
import { initParser } from "../render/parse";
import { closeInPageFind, inPageFindActiveIndex, inPageFindMatches, openInPageFind, scopedInPageFindMatchesForQuery, setInPageFindQuery, stepInPageFind } from "../inpageFind";
import { resetPaneLayoutToSingle, paneRouter } from "../panes";
import { ensurePageLoaded, loadFeed, mainPages, node, resetStore, retireFeedPages } from "../document";
import { setDoc } from "../document/model";
import { renderedBlockTextCallCountForTests, resetRenderedBlockTextCallCountForTests } from "../render/renderedText";

beforeAll(async () => { await initParser(); Range.prototype.getClientRects = vi.fn(() => [] as unknown as DOMRectList); });
afterEach(() => { closeInPageFind({ restoreFocus: false }); resetStore(); document.body.innerHTML = ""; vi.unstubAllGlobals(); });
it.each([390, 1200].flatMap(width => ["before", "after"].map(when => ({ width, when }))))(
  "keeps the visible count in sync at $width px when Find opens $when retirement and a day reloads", async ({ width, when }) => {
  vi.stubGlobal("innerWidth", width);
  window.dispatchEvent(new Event("resize"));
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  const days = Array.from({ length: 60 }, (_, i) => ({ name: `Day${i}`, title: `Day${i}`, kind: "journal" as const,
    pre_block: null, blocks: Array.from({ length: 30 }, (_, j) => ({ id: `b${i}-${j}`, raw: "needle", collapsed: false, children: [] })) }));
  loadFeed(days);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <><div data-pane-id="main"><For each={mainPages()}>{page =>
    <div data-feed-day={page.name}><For each={page.roots}>{id => <div class="ls-block" data-block-id={id}><div class="block-content">{node(id)?.raw}</div></div>}</For></div>
  }</For></div><InPageFind /></>, host);
  try {
    if (when === "before") { openInPageFind(); setInPageFindQuery("needle"); }
    retireFeedPages(days.map(day => day.name));
    if (when === "after") { openInPageFind(); setInPageFindQuery("needle"); }
    await Promise.resolve();
    expect(host.querySelectorAll(".ls-block")).toHaveLength(360);
    expect(host.querySelector(".inpage-find-count")?.textContent).toBe("1 / 360");
    expect(host.querySelector(".inpage-find-retired")?.textContent).toContain("48 older days not searched");
    ensurePageLoaded(days[0]);
    expect(host.querySelector(".inpage-find-count")?.textContent).toBe("1 / 390");
    expect(host.querySelector(".inpage-find-retired")?.textContent).toContain("47 older days not searched");
    resetRenderedBlockTextCallCountForTests();
    retireFeedPages(["Day48"]);
    expect(host.querySelectorAll(".ls-block")).toHaveLength(360);
    expect(host.querySelector(".inpage-find-count")?.textContent).toBe("1 / 360");
    expect(host.querySelector(".inpage-find-retired")?.textContent).toContain("48 older days not searched");
    expect(renderedBlockTextCallCountForTests()).toBe(0);
    const renderedIds = new Set([...host.querySelectorAll<HTMLElement>(".ls-block")].map(block => block.dataset.blockId));
    expect(new Set(inPageFindMatches().map(match => match.blockId))).toEqual(renderedIds);
    expect(scopedInPageFindMatchesForQuery("needle").every(match => renderedIds.has(match.blockId))).toBe(true);
    stepInPageFind(-1);
    expect(inPageFindActiveIndex()).toBe(359);
    stepInPageFind(1);
    expect(inPageFindActiveIndex()).toBe(0);
    setDoc("byId", "b0-0", "raw", "needle needle edited after reload");
    expect(host.querySelector(".inpage-find-count")?.textContent).toBe("1 / 361");
    expect(renderedBlockTextCallCountForTests()).toBe(1);
  } finally { dispose(); }
});
it("ignores displayed rows and text surfaces whose containing journal day is unloaded", () => {
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  const day = (i: number) => ({ name: `Day${i}`, title: `Day${i}`, kind: "journal" as const, pre_block: null,
    blocks: [{ id: `b${i}`, raw: "needle", collapsed: false, children: [] }] });
  loadFeed([day(1)]);
  setDoc("feed", ["Day0", "Day1"]);
  const host = document.createElement("div"); document.body.append(host);
  host.innerHTML = `<div data-pane-id="main"><div data-feed-day="Day0">
    <div class="ls-block" data-block-id="b0"><div class="block-content">needle</div></div>
    <div class="sheet-cell sheet-title-cell" data-block-id="query-row"><div class="sheet-cell-body">needle</div></div>
    <div data-inpage-find-surface="query-text">needle</div>
    </div></div>`;
  const bar = document.createElement("div"); host.append(bar);
  const dispose = render(() => <InPageFind />, bar);
  try {
    openInPageFind(); setInPageFindQuery("needle");
    expect(bar.querySelector(".inpage-find-count")?.textContent).toBe("1 / 1");
    expect(scopedInPageFindMatchesForQuery("needle").map(match => match.blockId)).toEqual(["b1"]);
    ensurePageLoaded(day(0));
    expect(bar.querySelector(".inpage-find-count")?.textContent).toBe("1 / 4");
    expect(new Set(inPageFindMatches().map(match => match.blockId))).toEqual(new Set(["b0", "b1", "query-row", "surface:query-text"]));
  } finally { dispose(); }
});
it("counts only retired days and hands the draft query to the existing graph search tab", async () => {
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  loadFeed(Array.from({ length: 20 }, (_, i) => ({ name: `Day${i}`, title: `Day${i}`, kind: "journal" as const,
    pre_block: null, blocks: [{ id: `b${i}`, raw: "needle", collapsed: false, children: [] }] })));
  retireFeedPages(Array.from({ length: 20 }, (_, i) => `Day${i}`));
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <InPageFind />, host);
  try {
    openInPageFind(); setInPageFindQuery("needle"); await Promise.resolve();
    expect(inPageFindMatches()).toHaveLength(12);
    expect(host.querySelector(".inpage-find-retired")?.textContent).toContain("8 older days not searched");
    const input = host.querySelector<HTMLInputElement>(".inpage-find-input")!;
    input.value = "fresh query"; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    host.querySelector<HTMLButtonElement>(".inpage-find-retired button")!.click();
    expect(paneRouter("main").route()).toMatchObject({ kind: "query", source: "fresh query", sourceKind: "search" });
    expect(host.querySelector(".inpage-find-bar")).toBeNull();
  } finally { dispose(); }
});
