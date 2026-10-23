import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InPageFind } from "./InPageFind";
import { initParser } from "../render/parse";
import { closeInPageFind, inPageFindMatches, openInPageFind, setInPageFindQuery } from "../inpageFind";
import { resetPaneLayoutToSingle, paneRouter } from "../panes";
import { loadFeed, resetStore, retireFeedPages } from "../document";

beforeAll(async () => { await initParser(); Range.prototype.getClientRects = vi.fn(() => [] as unknown as DOMRectList); });
afterEach(() => { closeInPageFind({ restoreFocus: false }); resetStore(); document.body.innerHTML = ""; });
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
