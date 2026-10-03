// GH #619: the query sheet opened at the viewport's top-left. The anchor was drawn from a rect measured on a
// sentence that was not in the document yet (a detached element's rect is all zeros), and the one-shot
// `/query` auto-open flag was consumed by whichever builder was CONSTRUCTED first, connected or not.
// jsdom has no layout, so the sentence's rect is stubbed to a recognisable non-zero box; the zero case is what
// jsdom already returns for a detached node.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetQueryRegistryRevisionForTests, QueryBuilder, type BuilderSession } from "./QueryBuilder";
import { clearTransientLayersForTest } from "../transientLayers";
import { queryBuilderAutoOpen, setQueryBuilderAutoOpen } from "../ui";
import { taskFilter } from "../editor/queryBuilder";
import type { Filter } from "../editor/queryIr";

const BOX = { top: 100, bottom: 140, left: 60, right: 460, width: 400, height: 40, x: 60, y: 100 };

function session(filter: Filter = taskFilter(["TODO"])): BuilderSession {
  return { query: { anchor: "block", filter, source: { kind: "builder" } }, view: {} };
}

/** Mount a builder into a host that is NOT in the document yet. */
function mountDetached(blockId?: string, filter?: Filter) {
  const host = document.createElement("div");
  const [current, setCurrent] = createSignal<BuilderSession>(session(filter));
  const dispose = render(() => <QueryBuilder session={current} onChange={setCurrent} blockId={blockId} />, host);
  return { host, dispose };
}

const frames = async (n: number) => {
  for (let i = 0; i < n; i += 1) await new Promise((resolve) => setTimeout(resolve, 20));
};

beforeEach(() => {
  resetQueryRegistryRevisionForTests();
  vi.spyOn(backend(), "queryFacets").mockResolvedValue([]);
  vi.spyOn(backend(), "queryRegistry").mockResolvedValue({ generation: 1, rows: [] });
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(and (task TODO))");
  // Layout exists only for connected nodes, as in a browser.
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    return (this.isConnected && this.classList.contains("qs-sentence")
      ? { ...BOX, toJSON() {} }
      : { top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON() {} }) as DOMRect;
  });
});

afterEach(() => {
  setQueryBuilderAutoOpen(null);
  clearTransientLayersForTest();
  resetSharedQueryResultsForTests();
  resetQueryRegistryRevisionForTests();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("query sheet placement (GH #619)", () => {
  it("a detached builder never draws the sheet at the top-left, and leaves the /query flag for a connected one", async () => {
    setQueryBuilderAutoOpen("blk");
    const detached = mountDetached("blk");
    await frames(4);
    // Nothing is drawn from a zero rect, and the one-shot flag was not eaten by the unseen instance.
    expect(document.querySelector(".qs-sheet-anchor")).toBeNull();
    expect(queryBuilderAutoOpen()).toBe("blk");
    detached.dispose();
  });

  it("opens beneath the sentence, with a real rect, once the sentence is connected", async () => {
    setQueryBuilderAutoOpen("blk");
    const { host, dispose } = mountDetached("blk");
    await frames(2);
    expect(document.querySelector(".qs-sheet-anchor")).toBeNull();
    document.body.append(host);
    await frames(6);
    const anchor = document.querySelector<HTMLElement>(".qs-sheet-anchor");
    expect(anchor).not.toBeNull();
    expect(anchor!.style.top).toBe("140px");
    expect(anchor!.style.left).toBe("60px");
    expect(queryBuilderAutoOpen()).toBeNull();
    dispose();
  });

  it("the gear on a connected builder opens at the sentence, never at 0,0", async () => {
    const { host, dispose } = mountDetached();
    document.body.append(host);
    host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
    await frames(4);
    const anchor = document.querySelector<HTMLElement>(".qs-sheet-anchor");
    expect(anchor).not.toBeNull();
    expect(anchor!.style.top).not.toBe("0px");
    expect(anchor!.style.left).not.toBe("0px");
    dispose();
  });

  // Martin 2026-10-03 (GH #619 comment 2): `/query` opens the sheet on the empty condition list. The field
  // chooser is an explicit user action; it must not open by itself.
  it("/query opens the sheet with the chooser CLOSED and no condition rows", async () => {
    setQueryBuilderAutoOpen("blk");
    const { host, dispose } = mountDetached("blk", { kind: "and", items: [] });
    document.body.append(host);
    await frames(6);
    const sheet = document.querySelector<HTMLElement>(".qs-sheet");
    expect(sheet).not.toBeNull();
    expect(sheet!.querySelectorAll(".qs-row").length).toBe(0);
    const add = sheet!.querySelector<HTMLButtonElement>(".qs-add");
    expect(add).not.toBeNull();
    expect(add!.getAttribute("aria-expanded")).not.toBe("true");
    expect(document.querySelector(".qs-menu")).toBeNull();
    dispose();
  });

  // The sheet is `position:fixed` and not scrollable on a wide layout, so a sentence near the bottom of the
  // window used to leave the sheet's lower half (and its text-pane input) off screen and unreachable.
  describe("vertical placement", () => {
    const place = async (sheetHeight: number, view: number, box = BOX) => {
      vi.spyOn(window, "innerHeight", "get").mockReturnValue(view);
      vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
        return this.classList.contains("qs-sheet") ? sheetHeight : 0;
      });
      vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
        return (this.isConnected && this.classList.contains("qs-sentence")
          ? { ...box, toJSON() {} }
          : { top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON() {} }) as DOMRect;
      });
      const { host, dispose } = mountDetached();
      document.body.append(host);
      host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
      await frames(8);
      const anchor = document.querySelector<HTMLElement>(".qs-sheet-anchor");
      const result = { top: anchor?.style.top, hidden: anchor?.style.visibility === "hidden" };
      dispose();
      return result;
    };
    it("stays under the sentence when it fits", async () => {
      expect(await place(300, 820)).toEqual({ top: "140px", hidden: false });
    });
    it("flips above a sentence near the bottom of the window", async () => {
      const low = { ...BOX, top: 700, bottom: 740, y: 700 };
      expect(await place(290, 820, low)).toEqual({ top: "410px", hidden: false });
    });
    it("is clamped into the window when it fits on neither side", async () => {
      expect(await place(700, 820)).toEqual({ top: "112px", hidden: false });
    });
  });
});
