// Margin dialogue, slice 2 (vision §3.7 "Layout"): the model side of the
// comment column — which blocks are drawn there, how threads stack, and where
// the arrow keys lead when part of the outline sits beside it.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initParser, blockRegions } from "./render/parse";
import { resetStore } from "./document";
import { loadSingle } from "./document/workingSet";
import { pageByName } from "./document/model";
import type { BlockDto } from "./types";
import { isCommentId, marginNext, marginPrev, pageHasComment, stackThreads, threadRootOf } from "./margin";

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

function load(blocks: BlockDto[]): void {
  loadSingle({ name: "M", kind: "page", title: "M", pre_block: null, blocks });
}

beforeAll(async () => {
  await initParser();
});
afterEach(() => resetStore());

describe("stacking threads in the margin", () => {
  it("keeps each thread at its anchor while there is room", () => {
    expect(stackThreads([{ anchor: 0, height: 40 }, { anchor: 100, height: 40 }], 8)).toEqual([0, 100]);
  });

  it("pushes an overlapping thread below the previous one, never overlapping", () => {
    const items = [{ anchor: 10, height: 50 }, { anchor: 20, height: 30 }, { anchor: 30, height: 10 }, { anchor: 200, height: 5 }];
    const tops = stackThreads(items, 8);
    expect(tops).toEqual([10, 68, 106, 200]);
    for (let i = 1; i < items.length; i++) expect(tops[i]).toBeGreaterThanOrEqual(tops[i - 1] + items[i - 1].height + 8);
    tops.forEach((top, i) => expect(top).toBeGreaterThanOrEqual(items[i].anchor));
  });
});

describe("which blocks belong in the margin", () => {
  it("finds comments anywhere on the page and nothing on a page without them", () => {
    load([block("p", "plain"), block("q", "parent", [block("r", "reply-less child")])]);
    expect(pageHasComment(pageByName("M")!.roots, "md")).toBe(false);
    resetStore();
    load([block("p", "plain"), block("q", "parent text", [block("c", "a comment\nquote:: parent")])]);
    expect(pageHasComment(pageByName("M")!.roots, "md")).toBe(true);
    expect(isCommentId("c", "md")).toBe(true);
    expect(isCommentId("q", "md")).toBe(false);
  });

  it("a top-level quote:: is not a comment; a reply belongs to its comment's thread", () => {
    load([
      block("top", "a root\nquote:: something"),
      block("p", "parent text", [block("c", "comment\nquote:: parent", [block("r", "reply", [block("rr", "deeper")])])]),
    ]);
    expect(pageHasComment(["top"], "md")).toBe(false);
    expect(threadRootOf("rr", "md")).toBe("c");
    expect(threadRootOf("c", "md")).toBe("c");
    expect(threadRootOf("p", "md")).toBeNull();
  });
});

describe("arrow keys with comments in the margin", () => {
  const outline = () => load([
    block("p", "parent text here", [
      block("kid", "an ordinary child"),
      block("c1", "first comment\nquote:: parent", [block("r1", "reply")]),
      block("c2", "second comment\nquote:: text"),
    ]),
    block("after", "the next block"),
  ]);

  it("the main column steps over threads in both directions", () => {
    outline();
    const main = { thread: null, format: "md" as const };
    expect(marginNext("kid", null, main)).toBe("after");
    expect(marginPrev("after", null, main)).toBe("kid");
    expect(marginNext("p", null, main)).toBe("kid");
  });

  it("a thread's top returns to the commented block and its end leads to the block after it", () => {
    outline();
    const t1 = { thread: { root: "c1", parent: "p" }, format: "md" as const };
    expect(marginPrev("c1", null, t1)).toBe("p");
    expect(marginNext("c1", null, t1)).toBe("r1");
    expect(marginPrev("r1", null, t1)).toBe("c1");
    expect(marginNext("r1", null, t1)).toBe("kid");
    const t2 = { thread: { root: "c2", parent: "p" }, format: "md" as const };
    expect(marginNext("c2", null, t2)).toBe("kid");
  });
});
