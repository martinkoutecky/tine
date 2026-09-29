// Master eba7c56b2 H3: a cross-day move re-plans after its await. A keyboard
// nudge that runs off the last loaded feed day awaits the feed extender; a second
// nudge issued meanwhile must not move blocks that the first already moved, or the
// target day holds the same block twice (duplicate content, duplicate `id::`).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../../render/parse";
import { backend } from "../../backend";
import { appendFeed, loadFeed, moveBlockFeed, moveSelectionItems, pageByName, resetStore, selectBlock, setFeedExtender } from "../index";
import { pageToDto } from "../convert";
import { doc } from "../model";
import type { BlockDto, PageDto } from "../../types";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `cd-${++serial}`, raw, collapsed: false, children: [] });
const day = (name: string, raws: string[]): PageDto => ({ name, title: name, kind: "journal", pre_block: null, blocks: raws.map(block) });
const roots = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];

function deferredExtender() {
  const waiters: Array<() => void> = [];
  setFeedExtender(() => new Promise<boolean>((resolve) => {
    waiters.push(() => { appendFeed([day("Older", ["o"])]); resolve(true); });
  }));
  return () => { for (const release of waiters.splice(0)) release(); };
}

beforeAll(() => initParser());
beforeEach(() => {
  serial = 0;
  resetStore();
  vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => ({ ok: entries.map((_, i) => `s-${i}`) }));
});
afterEach(() => { setFeedExtender(null); vi.restoreAllMocks(); });

describe("a cross-day move re-plans after its await (master H3)", () => {
  it("two quick selection nudges past the loaded feed move the blocks once", async () => {
    loadFeed([day("Today", ["a", "b"])]);
    const [, last] = pageByName("Today")!.roots;
    selectBlock(last);
    const release = deferredExtender();
    const first = moveSelectionItems(1);
    const second = moveSelectionItems(1);
    release();
    await Promise.all([first, second]);
    expect(roots("Today")).toEqual(["a"]);
    expect(roots("Older")).toEqual(["b", "o"]);
    expect(pageToDto("Older")!.blocks.map((b) => b.raw)).toEqual(["b", "o"]);
  });

  it("two quick single-block nudges past the loaded feed move the block once", async () => {
    loadFeed([day("Today", ["a", "b"])]);
    const [, last] = pageByName("Today")!.roots;
    const release = deferredExtender();
    const first = moveBlockFeed(last, 1);
    const second = moveBlockFeed(last, 1);
    release();
    const outcomes = await Promise.all([first, second]);
    expect(outcomes.filter((o) => o === "crossed")).toHaveLength(1);
    expect(roots("Today")).toEqual(["a"]);
    expect(roots("Older")).toEqual(["b", "o"]);
  });
});
