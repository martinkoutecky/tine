// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import {
  clearInPageFindRenderedTextCacheForTests,
  collectInPageFindMatches,
  findTextOccurrences,
  scopedInPageFindMatchesForQuery,
  type InPageFindBlock,
} from "./inpageFind";
import { renderedBlockTextCallCountForTests, resetRenderedBlockTextCallCountForTests } from "./render/renderedText";
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

const querySnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "query", id: "query-find", sourceKind: "search", source: "needle", presentation: "search" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

afterEach(() => {
  clearInPageFindRenderedTextCacheForTests();
  resetRenderedBlockTextCallCountForTests();
  resetStore();
  resetPaneLayoutToSingle(journalsSnapshot());
  document.body.innerHTML = "";
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

  it("keeps the same occurrence order and count across a many-block fixture", () => {
    const blocks: InPageFindBlock[] = Array.from({ length: 48 }, (_, i) => ({
      id: `root-${i}`,
      raw: i % 6 === 0 ? `needle root ${i} needle` : `plain root ${i}`,
      children: [
        {
          id: `child-${i}`,
          raw: i % 10 === 0 ? `child [[Needle Page ${i}]]` : `child ${i}`,
          children: i % 15 === 0 ? [{ id: `grand-${i}`, raw: `grand needle ${i}`, children: [] }] : [],
        },
      ],
    }));

    const matches = collectInPageFindMatches(blocks, "needle", "md");

    expect(matches.map((m) => `${m.blockId}:${m.ordinalInBlock}:${m.start}-${m.end}`)).toEqual([
      "root-0:0:0-6",
      "root-0:1:14-20",
      "child-0:0:8-14",
      "grand-0:0:6-12",
      "root-6:0:0-6",
      "root-6:1:14-20",
      "child-10:0:8-14",
      "root-12:0:0-6",
      "root-12:1:15-21",
      "grand-15:0:6-12",
      "root-18:0:0-6",
      "root-18:1:15-21",
      "child-20:0:8-14",
      "root-24:0:0-6",
      "root-24:1:15-21",
      "root-30:0:0-6",
      "root-30:1:15-21",
      "child-30:0:8-14",
      "grand-30:0:6-12",
      "root-36:0:0-6",
      "root-36:1:15-21",
      "child-40:0:8-14",
      "root-42:0:0-6",
      "root-42:1:15-21",
      "grand-45:0:6-12",
    ]);
  });

  it("reuses per-block rendered text across rapid query revisions", () => {
    const blocks: InPageFindBlock[] = Array.from({ length: 80 }, (_, i) => ({
      id: `block-${i}`,
      raw: `needle target ${i} with [[Needle Page]]`,
      children: [],
    }));

    for (const query of ["n", "ne", "nee", "need"]) collectInPageFindMatches(blocks, query, "md");

    expect(renderedBlockTextCallCountForTests()).toBe(80);
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

  it("searches visible rows in a persistent query workspace", () => {
    resetPaneLayoutToSingle(querySnapshot());
    document.body.innerHTML = `
      <main data-pane-id="main">
        <button data-inpage-find-surface="query:page:Alpha">Page Alpha needle</button>
        <button data-inpage-find-surface="query:block:block-1">Research block needle</button>
      </main>`;

    expect(scopedInPageFindMatchesForQuery("needle").map((match) => match.surfaceId)).toEqual([
      "query:page:Alpha",
      "query:block:block-1",
    ]);
  });

  it("adds visible reference rows to the page model without losing body matches", () => {
    setDoc({
      loaded: true,
      feed: ["Target"],
      pages: [{ name: "Target", kind: "page", title: "Target", preBlock: null, roots: ["body"], format: "md", readOnly: false, guide: false }],
      byId: { body: { id: "body", raw: "needle in body", collapsed: false, parent: null, page: "Target", children: [] } },
    });
    resetPaneLayoutToSingle(pageSnapshot("Target"));
    document.body.innerHTML = `
      <main data-pane-id="main">
        <div data-inpage-find-surface="linked:Source">linked reference needle</div>
        <div data-inpage-find-surface="unlinked:Other">unlinked reference needle</div>
      </main>`;

    const matches = scopedInPageFindMatchesForQuery("needle");
    expect(matches.map((match) => match.blockId)).toContain("body");
    expect(matches.map((match) => match.surfaceId).filter(Boolean)).toEqual([
      "linked:Source",
      "unlinked:Other",
    ]);
  });
});
