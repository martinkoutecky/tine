// GH #210 F1 follow-up (UI-GH210-HEADER-MOVE-TOAST, D-20261019-H210F2): the
// kernel refuses (Reason::HeaderRoot) any verb that would leave a page's
// materialized property header anywhere but first. The refusal used to be
// silent. Every entry path that can hit it now says why with one plain info
// toast, and the page stays exactly as it was: no change, no undo entry, no write.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { initParser } from "../../render/parse";
import { clearSeededFacets } from "../../render/facets";
import {
  beginPageHeaderEdit, canIndentBlock, indentBlock, indentSelection, insertOutlineBefore, isDirty, moveBlock, moveBlockFeed,
  moveBlocksRelative, moveItem, moveSelectionItems, resetStore, selectBlock, undo,
} from "..";
import { doc } from "../model";
import { loadFeed, loadSingle } from "../workingSet";
import { pageToDto } from "../convert";
import { bindTestHost } from "../host/wiring.test.support";
import { installDiskHost } from "../../diskHost.test.support";
import { setToasts, toasts } from "../../toasts";
import type { PageDto, PageRead } from "../../types";

beforeAll(async () => { await initParser(); });
afterEach(() => { vi.restoreAllMocks(); resetStore(); setToasts([]); });

const HEADER = "alias:: book\ntags:: reading";
const WHY = "Page properties stay at the top of the page";
const block = (id: string, raw: string) => ({ id, raw, collapsed: false, children: [] });
const page = (name: string) => doc.pages.find((p) => p.name === name)!;

async function host() {
  clearSeededFacets();
  const files = new Map<string, PageRead>();
  const writes: PageDto[] = [];
  const h = await bindTestHost();
  installDiskHost(h, {
    read: (path) => files.get(path) ?? null,
    write: (path, dto) => {
      writes.push(structuredClone(dto));
      files.set(path, { ...structuredClone(dto), id: path, rev: `r${writes.length}` });
      return `r${writes.length}`;
    },
  });
  return writes;
}

/** A page `Reading list` whose properties are its header, then `o1`, `o2`. */
async function headerPage() {
  loadSingle({ name: "Reading list", kind: "page", title: "Reading list", pre_block: HEADER, format: "md", blocks: [block("o1", "one"), block("o2", "two")] });
  const writes = await host();
  const h = beginPageHeaderEdit("Reading list")!;
  expect(page("Reading list").roots).toEqual([h, "o1", "o2"]);
  return { h, writes };
}

/** The user outcome of a refused gesture: the toast, an untouched outline, no write pending. */
function toldWhyAndUntouched(roots: string[], writes: PageDto[], name = "Reading list", why = WHY) {
  expect(toasts().map((t) => [t.kind, t.message])).toEqual([["info", why]]);
  expect(toasts()[0].sticky).toBeFalsy(); // not red, not sticky
  expect(page(name).roots).toEqual(roots);
  expect(writes).toEqual([]);
}

describe("a refused move above the page properties says why (GH #210 F1, D-20261019-H210F2)", () => {
  it("keyboard move up (Alt+Shift+Up) of the first block", async () => {
    const { h, writes } = await headerPage();
    expect(moveItem("o1", -1)).toBe(false);
    toldWhyAndUntouched([h, "o1", "o2"], writes);
    expect(isDirty("Reading list")).toBe(false);
  });

  it("keyboard move down of the properties themselves", async () => {
    const { h, writes } = await headerPage();
    expect(moveItem(h, 1)).toBe(false);
    toldWhyAndUntouched([h, "o1", "o2"], writes);
  });

  it("the journal-feed keyboard move (moveBlockFeed) of the first block", async () => {
    const { h, writes } = await headerPage();
    expect(await moveBlockFeed("o1", -1)).toBe("none");
    toldWhyAndUntouched([h, "o1", "o2"], writes);
  });

  it("a block selection moved up past the properties", async () => {
    const { h, writes } = await headerPage();
    selectBlock("o1");
    await moveSelectionItems(-1);
    toldWhyAndUntouched([h, "o1", "o2"], writes);
  });

  it("drag-and-drop of a block onto the slot above the properties (moveBlock)", async () => {
    const { h, writes } = await headerPage();
    await moveBlock("o2", null, 0, "Reading list");
    toldWhyAndUntouched([h, "o1", "o2"], writes);
  });

  it("drag-and-drop above the properties' drop indicator (moveBlocksRelative before)", async () => {
    const { h, writes } = await headerPage();
    expect(await moveBlocksRelative(["o2"], h, "before")).toBe(false);
    toldWhyAndUntouched([h, "o1", "o2"], writes);
  });

  it("dragging the properties themselves below another block", async () => {
    const { h, writes } = await headerPage();
    expect(await moveBlocksRelative([h], "o1", "after")).toBe(false);
    toldWhyAndUntouched([h, "o1", "o2"], writes);
  });

  it("a block inserted above the properties (paste / block-ref above)", async () => {
    const { h, writes } = await headerPage();
    expect(insertOutlineBefore(h, [{ raw: "x", children: [] }])).toBeNull();
    toldWhyAndUntouched([h, "o1", "o2"], writes);
  });

  it("Tab on the block right under the properties (GH #638, the same refusal)", async () => {
    const { h, writes } = await headerPage();
    expect(indentBlock("o1", 0)).toBeUndefined();
    toldWhyAndUntouched([h, "o1", "o2"], writes, "Reading list", "A block can't be indented under the page properties");
    expect(doc.byId[h].children).toEqual([]);
  });

  it("indenting a selection under the properties (swipe / Tab on a selection)", async () => {
    const { h, writes } = await headerPage();
    selectBlock("o1");
    indentSelection();
    toldWhyAndUntouched([h, "o1", "o2"], writes, "Reading list", "A block can't be indented under the page properties");
    expect(doc.byId[h].children).toEqual([]);
  });

  it("a block moved down from the last block of a journal day onto the top of a day whose properties lead it", async () => {
    loadFeed([
      { name: "2026_10_19", kind: "journal", title: "Oct 19th, 2026", pre_block: null, format: "md", blocks: [block("a1", "first day")] },
      { name: "2026_10_18", kind: "journal", title: "Oct 18th, 2026", pre_block: HEADER, format: "md", blocks: [block("b1", "second day")] },
    ]);
    const writes = await host();
    const h = beginPageHeaderEdit("2026_10_18")!;
    expect(doc.feed).toEqual(["2026_10_19", "2026_10_18"]);
    expect(await moveBlockFeed("a1", 1)).toBe("none");
    toldWhyAndUntouched(["a1"], writes, "2026_10_19");
    expect(page("2026_10_18").roots).toEqual([h, "b1"]);
  });
});

describe("the toast is only for the page header", () => {
  it("moves below the properties still work, silently, with an undo step", async () => {
    const { h, writes } = await headerPage();
    expect(moveItem("o2", -1)).toBe(true);
    expect(page("Reading list").roots).toEqual([h, "o2", "o1"]);
    expect(toasts()).toEqual([]);
    expect(isDirty("Reading list")).toBe(true);
    undo();
    expect(page("Reading list").roots).toEqual([h, "o1", "o2"]);
    expect(writes).toEqual([]); // nothing was written by the refused attempts
    expect(pageToDto("Reading list")!.pre_block).toBe(HEADER);
  });

  it("the Tab preflight (canIndentBlock, the swipe affordance) stays silent", async () => {
    await headerPage();
    expect(canIndentBlock("o1")).toBe(false);
    expect(toasts()).toEqual([]);
  });

  it("a held key (key repeat) shows the toast once", async () => {
    await headerPage();
    for (let i = 0; i < 4; i++) moveItem("o1", -1);
    expect(toasts()).toHaveLength(1);
  });

  it("an ordinary first-block move on a page without properties stays a silent no-op", async () => {
    loadSingle({ name: "Plain", kind: "page", title: "Plain", pre_block: null, format: "md", blocks: [block("p1", "one"), block("p2", "two")] });
    await host();
    expect(moveItem("p1", -1)).toBe(false);
    expect(toasts()).toEqual([]);
  });
});
