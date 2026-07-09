import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { collectInPageFindMatches, findTextOccurrences, scopedInPageFindMatchesForQuery, type InPageFindBlock } from "./inpageFind";
import { focusPane, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import { resetStore, setDoc } from "./store";
import type { PaneSnapshot } from "./router";

beforeAll(async () => {
  await initParser();
});

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

afterEach(() => {
  resetStore();
  resetPaneLayoutToSingle(journalsSnapshot());
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

  it("searches the focused page pane instead of the journals feed", () => {
    setDoc({
      loaded: true,
      feed: ["Feed"],
      pages: [
        { name: "Feed", kind: "journal", title: "Feed", preBlock: null, roots: ["feed-block"], format: "md", readOnly: false, guide: false },
        { name: "Pane", kind: "page", title: "Pane", preBlock: null, roots: ["pane-block"], format: "md", readOnly: false, guide: false },
      ],
      byId: {
        "feed-block": { id: "feed-block", raw: "needle in feed", collapsed: false, parent: null, page: "Feed", children: [] },
        "pane-block": { id: "pane-block", raw: "needle in pane", collapsed: false, parent: null, page: "Pane", children: [] },
      },
    });
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
      new Map([["main", journalsSnapshot()], ["pane-2", pageSnapshot("Pane")]]),
      "pane-2"
    );
    focusPane("pane-2");

    expect(scopedInPageFindMatchesForQuery("needle").map((m) => m.blockId)).toEqual(["pane-block"]);
  });
});
