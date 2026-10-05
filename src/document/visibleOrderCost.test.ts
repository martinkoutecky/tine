import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { BlockDto, PageDto } from "../types";
import { initParser } from "../render/parse";
import { loadFeed, pageByName, resetStore, setRaw } from ".";
import { visibleData } from "./tree";

// og C (I-25 / I-11): typing in an expanded parent block must not rebuild the
// feed-wide visible order. The order depends on each parent's raw only through
// the one fact "is this block an opaque sheet view?", so per-keystroke work is
// that one block's facet read, not a walk of every loaded feed page.

beforeAll(() => initParser());
afterEach(() => resetStore());

let next = 1;
const uid = () => `00000000-0000-4000-8000-${(next++).toString(16).padStart(12, "0")}`;
const leaf = (raw: string): BlockDto => ({ id: uid(), raw, collapsed: false, children: [] });
const page = (name: string, blocks: BlockDto[]): PageDto => ({ name, kind: "journal", title: name, pre_block: null, blocks, format: "md" });

function feed(pages: number): PageDto[] {
  return Array.from({ length: pages }, (_, p) =>
    page(`day ${p}`, [{ id: uid(), raw: `parent ${p}`, collapsed: false, children: [leaf("a"), leaf("b")] }]));
}

describe("visible order cost while typing", () => {
  it("keeps the feed-wide order when a keystroke lands in an expanded parent", () => {
    loadFeed(feed(20));
    const parent = pageByName("day 7")!.roots[0];
    const before = visibleData();
    expect(before.order.length).toBe(60);
    setRaw(parent, "parent 7 typed more");
    expect(visibleData()).toBe(before);
  });

  it("still rebuilds when an edit turns the parent into an opaque sheet view, and back", () => {
    loadFeed(feed(3));
    const parent = pageByName("day 1")!.roots[0];
    expect(visibleData().order).toContain(pageByName("day 1")!.roots[0]);
    expect(visibleData().order.length).toBe(9);
    setRaw(parent, "parent 1\ntine.view:: grid");
    expect(visibleData().order.length).toBe(7);
    setRaw(parent, "parent 1");
    expect(visibleData().order.length).toBe(9);
  });
});
