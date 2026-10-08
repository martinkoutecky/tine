// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { initParser } from "./render/parse";
import {
  clearInPageFindRenderedTextCacheForTests,
  collectInPageFindMatches,
  findTextOccurrences,
  revealInPageFindMatch,
  scopedInPageFindMatchesForQuery,
  type InPageFindBlock,
  openInPageFind,
  closeInPageFind,
  setInPageFindQuery,
} from "./inpageFind";
import { renderedBlockTextCallCountForTests, resetRenderedBlockTextCallCountForTests } from "./render/renderedText";
import { focusPane, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import { resetStore } from "./document";
import { setDoc } from "./document/model";
import type { PaneSnapshot } from "./router";
import { setGraphMeta } from "./graphSession";

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
  setGraphMeta(null);
  clearInPageFindRenderedTextCacheForTests();
  resetRenderedBlockTextCallCountForTests();
  resetStore();
  resetPaneLayoutToSingle(journalsSnapshot());
  document.body.innerHTML = "";
});

describe("in-page find model", () => {
  it("does not scroll a stale result after Find closes during an animation frame", async () => {
    resetPaneLayoutToSingle(querySnapshot());
    document.body.innerHTML = '<main data-pane-id="main"><button data-inpage-find-surface="query:page:Alpha">needle</button></main>';
    const element = document.querySelector<HTMLElement>("[data-inpage-find-surface]")!;
    const scroll = vi.fn();
    element.scrollIntoView = scroll;
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
    try {
      openInPageFind();
      setInPageFindQuery("needle");
      expect(frames.length).toBeGreaterThan(0);
      closeInPageFind({ restoreFocus: false });
      frames.shift()!(0);
      await Promise.resolve();
      expect(scroll).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
      closeInPageFind({ restoreFocus: false });
    }
  });

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
    expect(findTextOccurrences("aaaa", "aa", true)).toEqual([
      { start: 0, end: 2 },
      { start: 2, end: 4 },
    ]);
  });

  it("finds folded accents and keeps highlights on the original graphemes", () => {
    expect(findTextOccurrences("🧠 cafe\u0301 café", "cafe", true)).toEqual([
      { start: 3, end: 8 }, { start: 9, end: 13 },
    ]);
  });

  it("honors the graph's accent-removal opt-out", () => {
    setGraphMeta({ enable_search_remove_accents: false } as never);
    expect(findTextOccurrences("café", "cafe", false)).toEqual([]);
    expect(findTextOccurrences("café", "café", false)).toEqual([{ start: 0, end: 4 }]);
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

  it("searches both the journals feed and the routed split page", () => {
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

    expect(scopedInPageFindMatchesForQuery("needle").map((m) => m.blockId)).toEqual(["feed-block", "pane-block"]);
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

  it("finds a reference mention whose mark is a jump control, without finding the chrome around it", () => {
    setDoc({
      loaded: true,
      feed: ["Target"],
      pages: [{ name: "Target", kind: "page", title: "Target", preBlock: null, roots: [], format: "md", readOnly: false, guide: false }],
      byId: {},
    });
    resetPaneLayoutToSingle(pageSnapshot("Target"));
    // An unlinked-reference excerpt renders each marked mention as a button that
    // opens the source page there (master GH #200). The words inside it are the
    // ones the reader searched for, so they are content; the row's own actions
    // are chrome and must stay out of the searchable text.
    document.body.innerHTML = `
      <main data-pane-id="main">
        <div class="reference-blocks" data-inpage-find-surface="unlinked:Source">
          <div class="reference-excerpt-text">names <button type="button" class="reference-excerpt-mark" data-inpage-find-text>Query parity</button> near the start</div>
          <button type="button" class="reference-show-full">Show full block</button>
        </div>
      </main>`;

    expect(scopedInPageFindMatchesForQuery("names Query parity near the start").map((match) => match.surfaceId))
      .toEqual(["unlinked:Source"]);
    expect(scopedInPageFindMatchesForQuery("Show full block")).toEqual([]);
  });

  it("counts code text once, not again in the hidden line-gutter mirror", () => {
    setDoc({
      loaded: true,
      feed: ["Target"],
      pages: [{ name: "Target", kind: "page", title: "Target", preBlock: null, roots: [], format: "md", readOnly: false, guide: false }],
      byId: {},
    });
    resetPaneLayoutToSingle(pageSnapshot("Target"));
    // A code block's gutter repeats each line invisibly to mirror soft wraps; that
    // copy is aria-hidden layout, not content, so find must not count it.
    document.body.innerHTML = `
      <main data-pane-id="main">
        <div class="reference-blocks" data-inpage-find-surface="unlinked:Source">
          <pre class="code-block"><div class="calc-gutter code-gutter" aria-hidden="true"><div class="gutter-row"><span class="calc-lineno">1</span><span class="gutter-mirror">let needle = 1;</span></div></div><code class="hljs">let needle = 1;</code></pre>
        </div>
      </main>`;

    expect(scopedInPageFindMatchesForQuery("needle")).toHaveLength(1);
  });

  it("reveals the exact occurrence, not just the block, in a viewport-tall block (GH #253)", async () => {
    setDoc({
      loaded: true,
      feed: ["Long"],
      pages: [{ name: "Long", kind: "page", title: "Long", preBlock: null, roots: ["b1"], format: "md", readOnly: false, guide: false }],
      byId: { b1: { id: "b1", raw: "needle top … needle bottom", collapsed: false, parent: null, page: "Long", children: [] } },
    });
    resetPaneLayoutToSingle(pageSnapshot("Long"));
    document.body.innerHTML = `
      <main data-pane-id="main">
        <div class="scroller" style="overflow-y: auto">
          <div class="ls-block" data-block-id="b1">
            <div class="block-content">needle top … needle bottom</div>
          </div>
        </div>
      </main>`;
    const scroller = document.querySelector(".scroller") as HTMLElement;
    Object.defineProperty(scroller, "clientHeight", { value: 800, configurable: true });
    // A pane that does not overflow scrolls nowhere, and the reveal deliberately
    // refuses to claim success on one (it would skip the block-level fallback and
    // leave the occurrence off-screen). jsdom reports scrollHeight 0, so the
    // fixture has to say that this scroller really does have somewhere to go.
    Object.defineProperty(scroller, "scrollHeight", { value: 4000, configurable: true });
    let st = 0;
    // Mirror the DOM clamp (scrollTop never goes negative) so sequential
    // reveals accumulate exactly like a real scroller.
    Object.defineProperty(scroller, "scrollTop", {
      get: () => st,
      set: (v: number) => { st = Math.max(0, v); },
      configurable: true,
    });
    // jsdom has no layout: make each occurrence's Range report a distinct rect.
    // Occurrence 0 sits at the viewport top, occurrence 1 far below it.
    const rectByStart = new Map<number, DOMRect>([
      [0, { top: 100, bottom: 120, height: 20, width: 40, left: 0, right: 40 } as DOMRect],
      [13, { top: 2600, bottom: 2620, height: 20, width: 40, left: 0, right: 40 } as DOMRect],
    ]);
    (Range.prototype as unknown as { getClientRects: () => unknown }).getClientRects = function (this: Range) {
      const rect = rectByStart.get(this.startOffset);
      return rect ? [rect] : [];
    };
    const scrollIntoViewCalls: Element[] = [];
    (Element.prototype as unknown as { scrollIntoView: () => void }).scrollIntoView = function (this: Element) {
      scrollIntoViewCalls.push(this);
    };
    try {
      // og reveals only while Find is open (stale-reveal guard); opening it
      // reveals occurrence 0 on its own, so let that settle first.
      openInPageFind();
      setInPageFindQuery("needle");
      for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
      st = 0;

      await revealInPageFindMatch({ blockId: "b1", ordinalInBlock: 1, start: 13, end: 19 });

      // Scroller center is 400 (clientHeight 800, top 0); occurrence center is 2610.
      expect(scroller.scrollTop).toBeCloseTo(2610 - 400, 5);
      expect(scrollIntoViewCalls).toEqual([]);
    } finally {
      closeInPageFind({ restoreFocus: false });
      delete (Range.prototype as unknown as { getClientRects?: unknown }).getClientRects;
      delete (Element.prototype as unknown as { scrollIntoView?: unknown }).scrollIntoView;
    }
  });
});
