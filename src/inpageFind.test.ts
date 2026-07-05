import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { collectInPageFindMatches, findTextOccurrences, type InPageFindBlock } from "./inpageFind";

beforeAll(async () => {
  await initParser();
});

describe("in-page find model", () => {
  it("counts matches in collapsed descendants because it searches the block model", () => {
    const blocks: InPageFindBlock[] = [
      {
        id: "parent",
        raw: "parent without the term\ncollapsed:: true",
        children: [
          { id: "hidden-child", raw: "needle inside a collapsed branch", children: [] },
          { id: "hidden-grandchild", raw: "another Needle under the same folded parent", children: [] },
        ],
      },
    ];

    expect(collectInPageFindMatches(blocks, "needle", "md").map((m) => m.blockId)).toEqual([
      "hidden-child",
      "hidden-grandchild",
    ]);
  });

  it("does not count hidden collapsed/id properties as page text", () => {
    const blocks: InPageFindBlock[] = [
      { id: "a", raw: "visible body\ncollapsed:: true\nid:: abc", children: [] },
    ];

    expect(collectInPageFindMatches(blocks, "collapsed", "md")).toEqual([]);
    expect(collectInPageFindMatches(blocks, "visible", "md")).toHaveLength(1);
  });

  it("uses non-overlapping browser-style text occurrences", () => {
    expect(findTextOccurrences("aaaa", "aa")).toEqual([
      { start: 0, end: 2 },
      { start: 2, end: 4 },
    ]);
  });
});

