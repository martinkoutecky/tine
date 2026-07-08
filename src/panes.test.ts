import { beforeEach, describe, expect, it } from "vitest";
import {
  closeLayoutPane,
  layoutPaneIds,
  layoutRoot,
  paneRouter,
  resetPaneLayoutToSingle,
  splitLayoutNode,
  splitPane,
  type LayoutNode,
} from "./panes";
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
});

describe("pane layout mutations", () => {
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
});
