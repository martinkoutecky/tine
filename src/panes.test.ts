import { beforeEach, describe, expect, it } from "vitest";
import {
  closeLayoutPane,
  focusPane,
  openRouteInOtherPane,
  layoutPaneIds,
  layoutRoot,
  focusedPaneId,
  moveActiveTabToPane,
  paneRouter,
  resetPaneLayoutToSingle,
  splitLayoutNode,
  splitPane,
  type LayoutNode,
} from "./panes";
import { hasSelection, selectBlock } from "./store";
import { cellSel, setCellSel } from "./sheet/selection";
import type { PaneSnapshot } from "./router";

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

beforeEach(() => {
  resetPaneLayoutToSingle(journalsSnapshot());
  paneRouter("main").setScrollerElement(null);
});

describe("pane layout mutations", () => {
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
