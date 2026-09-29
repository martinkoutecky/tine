import { beforeEach, describe, expect, it } from "vitest";
import {
  focusPane,
  focusedPaneId,
  layoutPaneIds,
  layoutRoot,
  moveActiveTabInDirection,
  paneRouter,
  resetPaneLayoutToSingle,
  splitPane,
} from "./panes";
import { runGlobalCommand } from "./keybindings";
import type { PaneSnapshot } from "./router";
import { exitPaneSelect } from "./paneSelect";

// Master b653e6768 (GH #282): directional "Move tab to pane" grows the layout
// when no pane lies that way, instead of silently doing nothing.

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

beforeEach(() => {
  exitPaneSelect();
  resetPaneLayoutToSingle(journalsSnapshot());
});

describe("Move tab to pane commands (GH #282)", () => {
  it("the keyboard command spawns the missing pane from a single pane", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));

    expect(runGlobalCommand("pane/move-tab-right")).toBe(true);

    const ids = layoutPaneIds(layoutRoot());
    expect(ids).toHaveLength(2);
    expect(ids[0]).toBe("main");
    expect(paneRouter(ids[1]).route()).toEqual({ kind: "page", name: "Source", pageKind: "page" });
    expect(focusedPaneId()).toBe(ids[1]);
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
