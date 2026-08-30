import { beforeEach, describe, expect, it } from "vitest";
import {
  adjustPaneSize,
  closeLayoutPane,
  closePane,
  focusPane,
  openRouteInOtherPane,
  openPdf,
  layoutPaneIds,
  layoutRoot,
  focusedPaneId,
  lastFocusedLayoutPaneId,
  maximizedPaneId,
  moveActiveTabInDirection,
  moveActiveTabToPane,
  moveTabToSplitPane,
  togglePaneMaximize,
  visibleLayoutNode,
  paneRouter,
  resetPaneLayoutToSingle,
  restorePaneLayout,
  setFocusedPaneId,
  setSplitRatio,
  splitLayoutNode,
  splitPane,
  type LayoutNode,
} from "./panes";
import { hasSelection, selectBlock, setDoc } from "./store";
import { cellSel, setCellSel } from "./sheet/selection";
import type { PaneSnapshot } from "./router";
import { clearRecent, recentPages } from "./ui";
import { journalTitle } from "./journal";
import { exitPaneSelect, rememberBlockSelectionForPaneReturn } from "./paneSelect";

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

beforeEach(() => {
  clearRecent();
  exitPaneSelect();
  resetPaneLayoutToSingle(journalsSnapshot());
  paneRouter("main").setScrollerElement(null);
});

function setJournalFeed(entries: { name: string; blockId?: string }[]) {
  setDoc({
    byId: Object.fromEntries(entries.flatMap(({ name, blockId }) => blockId ? [[blockId, {
      id: blockId, raw: "Journal block", collapsed: false, parent: null, page: name, children: [],
    }]] : [])),
    pages: entries.map(({ name, blockId }) => ({
      name, kind: "journal" as const, title: name, preBlock: null,
      roots: blockId ? [blockId] : [], format: "md" as const, readOnly: false, guide: false,
    })),
    feed: entries.map(({ name }) => name),
    loaded: true,
  });
}

describe("pane layout mutations", () => {
  it("redirects a journals split to the selected feed day's plain, unpinned page", () => {
    setJournalFeed([{ name: "Selected journal day", blockId: "selected-feed-block" }]);
    resetPaneLayoutToSingle({
      tabs: [{
        history: [
          { kind: "page", name: "Different journal day", pageKind: "journal", block: "old-zoomed-block" },
          { kind: "journals" },
        ],
        pos: 1,
        pinned: true,
      }],
      activeIndex: 0,
    });
    rememberBlockSelectionForPaneReturn("selected-feed-block");

    const split = splitPane("main", "row")!;
    const tab = paneRouter(split).snapshot().tabs[0];

    expect(tab).toEqual({
      history: [{ kind: "page", name: "Selected journal day", pageKind: "journal" }],
      pos: 0,
      pinned: false,
    });
  });

  it("falls back to today's visible journal, then the first visible feed day", () => {
    const today = journalTitle(new Date());
    setJournalFeed([{ name: "Older journal day" }, { name: today }]);
    resetPaneLayoutToSingle({
      tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: true }],
      activeIndex: 0,
    });

    const todaySplit = splitPane("main", "row")!;
    expect(paneRouter(todaySplit).snapshot().tabs[0]).toEqual({
      history: [{ kind: "page", name: today, pageKind: "journal" }],
      pos: 0,
      pinned: false,
    });

    resetPaneLayoutToSingle({
      tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
      activeIndex: 0,
    });
    setJournalFeed([{ name: "First visible feed day" }]);

    const firstVisibleSplit = splitPane("main", "row")!;
    expect(paneRouter(firstVisibleSplit).snapshot().tabs[0]).toEqual({
      history: [{ kind: "page", name: "First visible feed day", pageKind: "journal" }],
      pos: 0,
      pinned: false,
    });
  });

  it("keeps a non-journals split as an exact mirror", () => {
    const history = [
      { kind: "page" as const, name: "Source", pageKind: "page" as const },
      { kind: "page" as const, name: "Zoomed", pageKind: "page" as const, block: "zoomed-block" },
    ];
    resetPaneLayoutToSingle({
      tabs: [{ history, pos: 1, pinned: true }],
      activeIndex: 0,
    });

    const split = splitPane("main", "row")!;

    expect(paneRouter(split).snapshot().tabs[0]).toEqual({ history, pos: 1, pinned: true });
  });

  it("promotes the route exposed by creating and focusing a split, but not a background split", () => {
    resetPaneLayoutToSingle(pageSnapshot("Left"));
    clearRecent();

    const background = splitPane("main", "row", {
      focusNew: false,
      snapshot: pageSnapshot("Background"),
    })!;
    expect(focusedPaneId()).toBe("main");
    expect(recentPages()).toEqual([]);

    const foreground = splitPane(background, "col", {
      snapshot: pageSnapshot("Foreground"),
    })!;
    expect(focusedPaneId()).toBe(foreground);
    expect(recentPages().map((item) => item.name)).toEqual(["Foreground"]);
  });

  it("promotes the sibling route exposed by closing the focused pane only", () => {
    resetPaneLayoutToSingle(pageSnapshot("Left"));
    const right = splitPane("main", "row", {
      focusNew: false,
      snapshot: pageSnapshot("Right"),
    })!;

    focusPane(right);
    clearRecent();
    expect(closePane(right)).toBe(true);
    expect(focusedPaneId()).toBe("main");
    expect(recentPages().map((item) => item.name)).toEqual(["Left"]);

    const background = splitPane("main", "row", {
      focusNew: false,
      snapshot: pageSnapshot("Background"),
    })!;
    clearRecent();
    expect(closePane(background)).toBe(true);
    expect(focusedPaneId()).toBe("main");
    expect(recentPages()).toEqual([]);
  });

  it("promotes the moved route when a real tab move creates and focuses a split", () => {
    resetPaneLayoutToSingle({
      tabs: [
        { history: [{ kind: "page", name: "Moved", pageKind: "page" }], pos: 0, pinned: false },
        { history: [{ kind: "page", name: "Spare", pageKind: "page" }], pos: 0, pinned: false },
      ],
      activeIndex: 0,
    });
    const movedId = paneRouter("main").activeId();
    clearRecent();

    const target = moveTabToSplitPane("main", movedId, "main", "right");

    expect(target).not.toBeNull();
    expect(focusedPaneId()).toBe(target);
    expect(paneRouter(target!).route()).toEqual({ kind: "page", name: "Moved", pageKind: "page" });
    expect(recentPages().map((item) => item.name)).toEqual(["Moved"]);
  });

  it("clears a sheet selection when focus moves to another pane", () => {
    splitPane("main", "row");
    focusPane("main");
    setCellSel({ gridId: "sheet", row: 0, col: 0 });
    focusPane(layoutPaneIds().find((id) => id !== "main")!);
    expect(cellSel()).toBeNull();
  });
  it("splits a leaf into a binary split", () => {
    const root: LayoutNode = { kind: "pane", paneId: "main" };

    expect(splitLayoutNode(root, "main", "row", "pane-2")).toEqual({
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "main" },
        { kind: "pane", paneId: "pane-2" },
      ],
    });
  });

  it("closes a pane by collapsing its parent split", () => {
    const root: LayoutNode = {
      kind: "split",
      dir: "col",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "main" },
        { kind: "pane", paneId: "pane-2" },
      ],
    };

    expect(closeLayoutPane(root, "pane-2")).toEqual({
      node: { kind: "pane", paneId: "main" },
      focusedPaneId: "main",
      closed: true,
    });
  });

  it("does not close the last remaining pane", () => {
    const root: LayoutNode = { kind: "pane", paneId: "main" };

    expect(closeLayoutPane(root, "main")).toEqual({
      node: root,
      focusedPaneId: "main",
      closed: false,
    });
  });

  it("closing the last tab of a page pane closes that pane", async () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const newPaneId = splitPane("main", "row")!;

    await paneRouter(newPaneId).closeTab(paneRouter(newPaneId).activeId());

    expect(layoutPaneIds(layoutRoot())).toEqual(["main"]);
  });

  it("the feed pane keeps its last tab", async () => {
    resetPaneLayoutToSingle(journalsSnapshot());

    await paneRouter("main").closeTab(paneRouter("main").activeId());

    expect(layoutPaneIds(layoutRoot())).toEqual(["main"]);
    expect(paneRouter("main").tabs()).toHaveLength(1);
  });

  it("adopts the active tab into another pane preserving history, pin, and scroll", () => {
    resetPaneLayoutToSingle({
      tabs: [
        {
          history: [
            { kind: "page", name: "One", pageKind: "page" },
            { kind: "page", name: "Two", pageKind: "page" },
          ],
          pos: 1,
          pinned: true,
        },
        { history: [{ kind: "page", name: "Spare", pageKind: "page" }], pos: 0, pinned: false },
      ],
      activeIndex: 0,
    });
    const target = splitPane("main", "row")!;
    paneRouter("main").setScrollerElement({ scrollTop: 73, isConnected: true } as HTMLElement);

    expect(moveActiveTabToPane("main", target)).toBe(true);

    const snap = paneRouter(target).snapshot();
    const adopted = snap.tabs[snap.activeIndex];
    expect(adopted).toMatchObject({
      history: [
        { kind: "page", name: "One", pageKind: "page" },
        { kind: "page", name: "Two", pageKind: "page" },
      ],
      pos: 1,
      pinned: true,
    });
    expect(snap.scrolls?.[snap.activeIndex]).toBe(73);
  });

  it("adopts an empty query route without changing its identity/source or the intended focus", () => {
    resetPaneLayoutToSingle({
      tabs: [{ history: [{ kind: "query", id: "query-empty-adopt", sourceKind: "search", source: "", presentation: "search" }], pos: 0, pinned: false }],
      activeIndex: 0,
    });
    const target = splitPane("main", "row", { focusNew: false })!;
    focusPane("main");
    expect(moveActiveTabToPane("main", target)).toBe(true);
    expect(paneRouter(target).route()).toEqual({ kind: "query", id: "query-empty-adopt", sourceKind: "search", source: "", presentation: "search" });
    expect(focusedPaneId()).toBe(target);
  });

  it("moving the last page tab out closes the emptied pane", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const target = splitPane("main", "row")!;

    expect(moveActiveTabToPane("main", target)).toBe(true);

    expect(layoutPaneIds(layoutRoot())).toEqual([target]);
    expect(focusedPaneId()).toBe(target);
  });

  it("does not move the feed pane's last journals tab", () => {
    resetPaneLayoutToSingle(journalsSnapshot());
    const target = splitPane("main", "row")!;

    expect(moveActiveTabToPane("main", target)).toBe(false);

    expect(layoutPaneIds(layoutRoot())).toEqual(["main", target]);
    expect(paneRouter("main").route()).toEqual({ kind: "journals" });
  });

  it("clears block selection when focus moves to another pane", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const target = splitPane("main", "row")!;
    focusPane("main");
    selectBlock("selected-block");

    focusPane(target);

    expect(hasSelection()).toBe(false);
  });
});

describe("moveActiveTabInDirection (GH #282)", () => {
  it("moves the active tab into the pane that already lies in the direction", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row")!;
    focusPane("main");

    expect(moveActiveTabInDirection("main", "right")).toBe(right);

    expect(layoutPaneIds(layoutRoot())).toEqual([right]);
    expect(focusedPaneId()).toBe(right);
  });

  it("spawns a right-hand mirror pane from a single one-tab pane", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));

    const created = moveActiveTabInDirection("main", "right")!;

    expect(created).toBeTruthy();
    expect(layoutRoot()).toEqual({
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "main" },
        { kind: "pane", paneId: created },
      ],
    });
    // One-tab source has no empty-pane route: the original tab stays and the
    // new pane opens as a mirror of the same tab history.
    expect(paneRouter("main").route()).toEqual({ kind: "page", name: "Source", pageKind: "page" });
    expect(paneRouter(created).route()).toEqual({ kind: "page", name: "Source", pageKind: "page" });
    expect(focusedPaneId()).toBe(created);
  });

  it("places the spawned pane before the source for left/up and after it for down", () => {
    for (const [dir, axis, first] of [
      ["left", "row", true],
      ["up", "col", true],
      ["down", "col", false],
    ] as const) {
      resetPaneLayoutToSingle(pageSnapshot("Source"));

      const created = moveActiveTabInDirection("main", dir)!;

      const layout = layoutRoot();
      expect(layout.kind).toBe("split");
      if (layout.kind !== "split") throw new Error("expected a split layout");
      expect(layout.dir).toBe(axis);
      expect(layout.children).toEqual(
        first
          ? [{ kind: "pane", paneId: created }, { kind: "pane", paneId: "main" }]
          : [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: created }]
      );
      expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Source" });
      expect(paneRouter(created).route()).toMatchObject({ kind: "page", name: "Source" });
      resetPaneLayoutToSingle(journalsSnapshot());
    }
  });

  it("donates only the active tab when a multi-tab source grows a missing neighbor", () => {
    resetPaneLayoutToSingle({
      tabs: [
        { history: [{ kind: "page", name: "One", pageKind: "page" }], pos: 0, pinned: false },
        { history: [{ kind: "page", name: "Two", pageKind: "page" }], pos: 0, pinned: false },
      ],
      activeIndex: 0,
    });

    const created = moveActiveTabInDirection("main", "down")!;

    expect(layoutRoot()).toMatchObject({ kind: "split", dir: "col" });
    expect(paneRouter("main").tabs().map((t) => t.history[0])).toEqual([
      { kind: "page", name: "Two", pageKind: "page" },
    ]);
    expect(paneRouter(created).route()).toEqual({ kind: "page", name: "One", pageKind: "page" });
    expect(focusedPaneId()).toBe(created);
  });

  it("splits the other axis when the layout extends only horizontally", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row")!;
    focusPane("main");

    const created = moveActiveTabInDirection("main", "down")!;

    expect(layoutRoot()).toEqual({
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        {
          kind: "split",
          dir: "col",
          ratio: 0.5,
          children: [
            { kind: "pane", paneId: "main" },
            { kind: "pane", paneId: created },
          ],
        },
        { kind: "pane", paneId: right },
      ],
    });
    expect(paneRouter(created).route()).toMatchObject({ kind: "page", name: "Source" });
    expect(focusedPaneId()).toBe(created);
  });

  it("mirrors a lone journals tab instead of refusing when there is no neighbor", () => {
    const created = moveActiveTabInDirection("main", "right")!;

    expect(created).toBeTruthy();
    expect(layoutPaneIds(layoutRoot())).toEqual(["main", created]);
    expect(paneRouter("main").route()).toEqual({ kind: "journals" });
  });

  it("keeps the existing refusal for a lone journals tab that already has a neighbor", () => {
    const right = splitPane("main", "row", { focusNew: false })!;
    focusPane("main");

    expect(moveActiveTabInDirection("main", "right")).toBe(null);
    expect(layoutPaneIds(layoutRoot())).toEqual(["main", right]);
    expect(paneRouter("main").route()).toEqual({ kind: "journals" });
  });
});

describe("pane maximize (GH #285)", () => {
  it("is a no-op on a single-pane window", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));

    expect(togglePaneMaximize("main")).toBe(false);
    expect(maximizedPaneId()).toBe(null);
    expect(visibleLayoutNode()).toEqual({ kind: "pane", paneId: "main" });
  });

  it("shows only the maximized pane while keeping the real tree and ratios untouched", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row")!;
    focusPane("main");
    const treeBefore = layoutRoot();

    expect(togglePaneMaximize("main")).toBe(true);

    expect(maximizedPaneId()).toBe("main");
    expect(visibleLayoutNode()).toEqual({ kind: "pane", paneId: "main" });
    // Session/workspace persistence reads layoutRoot(): the full tree (with
    // its ratio) survives maximization, so nothing transient is serialized.
    expect(layoutRoot()).toEqual(treeBefore);
    expect(layoutRoot()).toEqual({
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "main" },
        { kind: "pane", paneId: right },
      ],
    });

    expect(togglePaneMaximize("main")).toBe(true);
    expect(maximizedPaneId()).toBe(null);
    expect(visibleLayoutNode()).toEqual(treeBefore);
  });

  it("keeps mutations made while maximized, restoring the evolved tree exactly", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row", { focusNew: false })!;
    togglePaneMaximize("main");

    const below = splitPane(right, "col", { focusNew: false })!;

    expect(visibleLayoutNode()).toEqual({ kind: "pane", paneId: "main" });
    expect(togglePaneMaximize("main")).toBe(true);
    expect(visibleLayoutNode()).toEqual({
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "main" },
        {
          kind: "split",
          dir: "col",
          ratio: 0.5,
          children: [
            { kind: "pane", paneId: right },
            { kind: "pane", paneId: below },
          ],
        },
      ],
    });
  });

  it("clears when the maximized pane disappears, showing the surviving tree", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row")!;
    togglePaneMaximize("main");

    expect(closePane("main")).toBe(true);

    expect(maximizedPaneId()).toBe(null);
    expect(visibleLayoutNode()).toEqual({ kind: "pane", paneId: right });
    expect(layoutRoot()).toEqual({ kind: "pane", paneId: right });
  });

  it("stays engaged when a sibling pane closes, and collapses gracefully to one pane", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row")!;
    focusPane("main");
    togglePaneMaximize("main");

    expect(closePane(right)).toBe(true);

    // Only the maximized pane is left; the visible surface is still just it.
    expect(visibleLayoutNode()).toEqual({ kind: "pane", paneId: "main" });
    // Toggling again from this degenerate state still unmaximizes cleanly.
    expect(togglePaneMaximize("main")).toBe(true);
    expect(maximizedPaneId()).toBe(null);
    expect(visibleLayoutNode()).toEqual({ kind: "pane", paneId: "main" });
  });

  it("restores the full layout when focus escapes to another pane", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row")!;
    focusPane("main");
    togglePaneMaximize("main");

    focusPane(right);

    expect(maximizedPaneId()).toBe(null);
    expect(visibleLayoutNode()).toEqual(layoutRoot());
    expect(focusedPaneId()).toBe(right);
  });

  it("restores the full layout through the shared focus-state boundary", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row")!;
    focusPane("main");
    togglePaneMaximize("main");

    // Session/history adapters use this lower-level boundary directly.
    setFocusedPaneId(right);

    expect(maximizedPaneId()).toBeNull();
    expect(visibleLayoutNode()).toEqual(layoutRoot());
    expect(focusedPaneId()).toBe(right);
  });
});

describe("adjustPaneSize (GH #286)", () => {
  it("is a no-op for a sole pane", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));

    expect(adjustPaneSize("main", "width", true)).toBe(false);
    expect(layoutRoot()).toEqual({ kind: "pane", paneId: "main" });
  });

  it("grows and shrinks the first child through its row split by five points", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    splitPane("main", "row");

    expect(adjustPaneSize("main", "width", true)).toBe(true);
    expect(layoutRoot()).toMatchObject({ kind: "split", dir: "row", ratio: 0.55 });

    expect(adjustPaneSize("main", "width", false)).toBe(true);
    expect(layoutRoot()).toMatchObject({ kind: "split", dir: "row", ratio: 0.5 });
  });

  it("moves the same ratio the other way for the second child", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row")!;

    expect(adjustPaneSize(right, "width", true)).toBe(true);
    expect(layoutRoot()).toMatchObject({ kind: "split", ratio: 0.45 });

    expect(adjustPaneSize(right, "width", false)).toBe(true);
    expect(layoutRoot()).toMatchObject({ kind: "split", ratio: 0.5 });
  });

  it("adjusts height only through a column split, never through a row split", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    splitPane("main", "row");

    expect(adjustPaneSize("main", "height", true)).toBe(false);
    expect(layoutRoot()).toMatchObject({ kind: "split", ratio: 0.5 });
  });

  it("walks past a nearer wrong-axis ancestor to the nearest matching one", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const right = splitPane("main", "row")!;
    const below = splitPane(right, "col")!;

    // `below` sits under a col split inside the second row child: a width
    // change must skip the nearer col ancestor and adjust the root row split.
    expect(adjustPaneSize(below, "width", true)).toBe(true);

    expect(layoutRoot()).toEqual({
      kind: "split",
      dir: "row",
      ratio: 0.45, // second child holds `below`; growing it shrinks the ratio
      children: [
        { kind: "pane", paneId: "main" },
        {
          kind: "split",
          dir: "col",
          ratio: 0.5, // untouched — wrong axis for a width command
          children: [
            { kind: "pane", paneId: right },
            { kind: "pane", paneId: below },
          ],
        },
      ],
    });
  });

  it("respects the existing 15–85% clamps", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    splitPane("main", "row");
    setSplitRatio([], 0.85);

    expect(adjustPaneSize("main", "width", true)).toBe(true);
    expect(layoutRoot()).toMatchObject({ kind: "split", ratio: 0.85 });

    setSplitRatio([], 0.15);
    expect(adjustPaneSize("main", "width", false)).toBe(true);
    expect(layoutRoot()).toMatchObject({ kind: "split", ratio: 0.15 });
  });
});

describe("openRouteInOtherPane", () => {
  it("a freshly-created pane ends with a SINGLE tab (target replaces the split duplicate)", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));

    const target = openRouteInOtherPane({ kind: "page", name: "Dest", pageKind: "page" }, "main");

    expect(target).not.toBeNull();
    const tabs = paneRouter(target!).tabs();
    expect(tabs).toHaveLength(1);
    expect(tabs[0].history[tabs[0].pos]).toMatchObject({ kind: "page", name: "Dest" });
    // Back-history keeps the source context (the duplicated entry).
    expect(tabs[0].history.length).toBeGreaterThan(1);
    expect(focusedPaneId()).toBe("main");
  });

  it("an EXISTING other pane gets the route as a new foreground tab", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const other = splitPane("main", "row", { focusNew: false })!;
    const before = paneRouter(other).tabs().length;

    openRouteInOtherPane({ kind: "page", name: "Dest", pageKind: "page" }, "main");

    expect(paneRouter(other).tabs().length).toBe(before + 1);
  });

  it("uses the nearest-ancestor sibling rather than a globally-nearest tie", () => {
    const root: LayoutNode = {
      kind: "split",
      dir: "col",
      ratio: 0.1,
      children: [
        {
          kind: "split", dir: "row", ratio: 0.5,
          children: [
            { kind: "pane", paneId: "main" },
            { kind: "pane", paneId: "z-structural" },
          ],
        },
        {
          kind: "split", dir: "row", ratio: 0.5,
          children: [
            { kind: "pane", paneId: "a-global-tie" },
            { kind: "pane", paneId: "far" },
          ],
        },
      ],
    };
    const snapshots = new Map([
      ["main", pageSnapshot("Source")],
      ["z-structural", pageSnapshot("Structural")],
      ["a-global-tie", pageSnapshot("Global")],
      ["far", pageSnapshot("Far")],
    ]);
    restorePaneLayout(root, snapshots, "main");

    expect(openRouteInOtherPane({ kind: "page", name: "Dest", pageKind: "page" }, "main"))
      .toBe("z-structural");
  });

  it("does not escape the structural selector for a missing source leaf", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const before = paneRouter("main").snapshot();

    expect(openRouteInOtherPane({ kind: "page", name: "Dest", pageKind: "page" }, "missing"))
      .toBeNull();
    expect(layoutPaneIds()).toEqual(["main"]);
    expect(paneRouter("main").snapshot()).toEqual(before);
  });
});

describe("last focused layout pane", () => {
  it("ignores satellite and legacy PDF focus while retaining current focus behavior", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const other = splitPane("main", "row")!;
    setFocusedPaneId(other);
    expect(lastFocusedLayoutPaneId()).toBe(other);

    // The dedicated PDF representation may still publish its pseudo id during
    // the migration. It must not become a future satellite action's source.
    setFocusedPaneId("pdf", false);
    expect(focusedPaneId()).toBe("pdf");
    expect(lastFocusedLayoutPaneId()).toBe(other);

    // Current outside-pane pointer behavior still retargets current focus to
    // main, but does not erase the last focus backed by a real pane surface.
    setFocusedPaneId("main", false);
    expect(focusedPaneId()).toBe("main");
    expect(lastFocusedLayoutPaneId()).toBe(other);
  });
});

describe("PDF pane routes", () => {
  it("opens beside the source in one exact PDF tab without cloned source history", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));

    const opened = openPdf("assets/alpha.pdf", "Alpha", 3, undefined, { sourcePaneId: "main" });

    expect(opened).toMatchObject({ kind: "pdf", filename: "assets/alpha.pdf", page: 3 });
    expect(layoutPaneIds()).toHaveLength(2);
    const pdfPane = layoutPaneIds().find((id) => id !== "main")!;
    expect(paneRouter(pdfPane).snapshot().tabs).toEqual([{
      history: [opened], pos: 0, pinned: false,
    }]);
    expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Source" });
  });

  it("reuses one companion pane for different PDFs and focuses an existing document tab", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const alpha = openPdf("assets/alpha.pdf", "Alpha", 1, undefined, { sourcePaneId: "main" })!;
    const pdfPane = focusedPaneId();
    focusPane("main");
    openPdf("assets/beta.pdf", "Beta", 2, undefined, { sourcePaneId: "main" });
    expect(layoutPaneIds()).toHaveLength(2);
    expect(paneRouter(pdfPane).tabs()).toHaveLength(2);

    focusPane("main");
    const reused = openPdf("assets/alpha.pdf", "Alpha", 9, "hl-9", { sourcePaneId: "main" });
    expect(reused?.viewId).toBe(alpha.viewId);
    expect(layoutPaneIds()).toHaveLength(2);
    expect(paneRouter(pdfPane).route()).toMatchObject({ kind: "pdf", viewId: alpha.viewId, page: 9 });
  });

  it("does not expose deliberate duplicate views before shared annotation ownership lands", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    openPdf("assets/alpha.pdf", "Alpha", undefined, undefined, { sourcePaneId: "main" });
    expect(openPdf("assets/alpha.pdf", "Alpha", undefined, undefined, {
      sourcePaneId: "main", anotherView: true,
    })).toBeNull();
  });
});
