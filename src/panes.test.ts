import { beforeEach, describe, expect, it } from "vitest";
import {
  closeLayoutPane,
  closePane,
  focusPane,
  openRouteInOtherPane,
  layoutPaneIds,
  layoutRoot,
  focusedPaneId,
  moveActiveTabToPane,
  moveTabToSplitPane,
  paneRouter,
  resetPaneLayoutToSingle,
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
});
