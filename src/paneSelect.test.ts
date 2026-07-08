import { describe, expect, it } from "vitest";
import {
  computePaneGeometry,
  nearestPane,
  nearestPaneInDirection,
  readingOrderPanes,
  stepPaneTarget,
  type PaneTarget,
} from "./paneSelect";
import type { LayoutNode } from "./panes";

const roundRect = (r: { x: number; y: number; w: number; h: number }) => ({
  x: Number(r.x.toFixed(4)),
  y: Number(r.y.toFixed(4)),
  w: Number(r.w.toFixed(4)),
  h: Number(r.h.toFixed(4)),
});

describe("pane geometry", () => {
  it("computes pane and seam rects over nested split ratios", () => {
    const root: LayoutNode = {
      kind: "split",
      dir: "row",
      ratio: 0.6,
      children: [
        { kind: "pane", paneId: "a" },
        {
          kind: "split",
          dir: "col",
          ratio: 0.25,
          children: [
            { kind: "pane", paneId: "b" },
            { kind: "pane", paneId: "c" },
          ],
        },
      ],
    };

    const geom = computePaneGeometry(root);

    expect(Object.fromEntries(geom.panes.map((p) => [p.paneId, roundRect(p.rect)]))).toEqual({
      a: { x: 0, y: 0, w: 0.6, h: 1 },
      b: { x: 0.6, y: 0, w: 0.4, h: 0.25 },
      c: { x: 0.6, y: 0.25, w: 0.4, h: 0.75 },
    });
    expect(geom.seams.map((s) => ({ path: s.path, dir: s.dir, rect: roundRect(s.rect) }))).toEqual([
      { path: [], dir: "row", rect: { x: 0.6, y: 0, w: 0, h: 1 } },
      { path: [1], dir: "col", rect: { x: 0.6, y: 0.25, w: 0.4, h: 0 } },
    ]);
    expect(geom.edges.map((e) => e.side)).toEqual(["left", "right", "top", "bottom"]);
  });

  it("steps pane selection through seams before panes and then to edges", () => {
    const root: LayoutNode = {
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "left" },
        { kind: "pane", paneId: "right" },
      ],
    };

    const seam: PaneTarget = { kind: "seam", path: [] };
    expect(stepPaneTarget(root, { kind: "pane", paneId: "left" }, "right")).toEqual(seam);
    expect(stepPaneTarget(root, seam, "right")).toEqual({ kind: "pane", paneId: "right" });
    // The pane's own edge segment outranks the (same-position) global edge.
    expect(stepPaneTarget(root, { kind: "pane", paneId: "right" }, "right")).toEqual({
      kind: "pane-edge",
      paneId: "right",
      side: "right",
    });
    expect(stepPaneTarget(root, { kind: "edge", side: "right" }, "left")).toEqual({
      kind: "pane",
      paneId: "right",
    });
  });

  it("widens a FULL-span segment to the global edge too — nesting in the pane vs splitting the root differ (Martin's Jul 8 follow-up)", () => {
    // row [left, right]: left's LEFT segment spans the full window height,
    // yet Enter there nests a quarter-width column inside the left half while
    // the global edge makes a half-width root column. Both must be reachable.
    const root: LayoutNode = {
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "left" },
        { kind: "pane", paneId: "right" },
      ],
    };

    const seg = stepPaneTarget(root, { kind: "pane", paneId: "left" }, "left");
    expect(seg).toEqual({ kind: "pane-edge", paneId: "left", side: "left" });
    expect(stepPaneTarget(root, seg, "left")).toEqual({ kind: "edge", side: "left" });
  });

  it("keeps the ghost rung skipped for a true solo pane (both splits are identical there)", () => {
    const root: LayoutNode = { kind: "pane", paneId: "main" };
    const seg = stepPaneTarget(root, { kind: "pane", paneId: "main" }, "right");
    expect(seg).toEqual({ kind: "pane-edge", paneId: "main", side: "right" });
    expect(stepPaneTarget(root, seg, "right")).toEqual(seg);
  });

  it("ArrowUp from one pane of a row split targets THAT pane's top segment, then the whole edge (Martin's Jul 8 nit)", () => {
    const root: LayoutNode = {
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "left" },
        { kind: "pane", paneId: "right" },
      ],
    };

    const seg = stepPaneTarget(root, { kind: "pane", paneId: "left" }, "up");
    expect(seg).toEqual({ kind: "pane-edge", paneId: "left", side: "top" });
    // pressing outward again widens to the whole-window edge (root split)
    expect(stepPaneTarget(root, seg, "up")).toEqual({ kind: "edge", side: "top" });
  });

  it("requires directional overlap: down from a tall right pane hits its own bottom segment, never the left panes' seam", () => {
    // Martin's Jul 8 screenshot layout: two panes stacked on the left, one
    // tall pane on the right. ArrowDown from the right pane used to select
    // the seam BETWEEN THE LEFT PANES (center-distance artifact).
    const root: LayoutNode = {
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        {
          kind: "split",
          dir: "col",
          ratio: 0.5,
          children: [
            { kind: "pane", paneId: "a" },
            { kind: "pane", paneId: "b" },
          ],
        },
        { kind: "pane", paneId: "c" },
      ],
    };

    const down = stepPaneTarget(root, { kind: "pane", paneId: "c" }, "down");
    expect(down).toEqual({ kind: "pane-edge", paneId: "c", side: "bottom" });
    expect(stepPaneTarget(root, down, "down")).toEqual({ kind: "edge", side: "bottom" });

    const up = stepPaneTarget(root, { kind: "pane", paneId: "c" }, "up");
    expect(up).toEqual({ kind: "pane-edge", paneId: "c", side: "top" });
    expect(stepPaneTarget(root, up, "up")).toEqual({ kind: "edge", side: "top" });
  });

  it("emits pane-edge segments for ALL four sides of every pane (internal sides too)", () => {
    const root: LayoutNode = {
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        {
          kind: "split",
          dir: "col",
          ratio: 0.5,
          children: [
            { kind: "pane", paneId: "a" },
            { kind: "pane", paneId: "b" },
          ],
        },
        { kind: "pane", paneId: "c" },
      ],
    };

    const segs = computePaneGeometry(root).paneEdges.map((e) => `${e.paneId}:${e.side}`).sort();
    const all = ["a", "b", "c"].flatMap((p) => ["left", "right", "top", "bottom"].map((s) => `${p}:${s}`)).sort();
    expect(segs).toEqual(all);
  });

  it("ArrowRight from an inner pane targets ITS right side, widening to the covering seam (Martin's 3-column report)", () => {
    // Martin's Jul 8 layout: left guide, middle column split top/bottom,
    // right guide — with the middle column slightly LEFT of window center.
    // Bug 1: the global TOP edge's center was "ahead-right" of the pane's
    // center and won with a near-zero distance (all panes tinted). Bug 2:
    // the pane's right side is internal, so "split just this pane" had no
    // target at all.
    const root: LayoutNode = {
      kind: "split",
      dir: "row",
      ratio: 0.32,
      children: [
        { kind: "pane", paneId: "a" },
        {
          kind: "split",
          dir: "row",
          ratio: 0.5,
          children: [
            {
              kind: "split",
              dir: "col",
              ratio: 0.7,
              children: [
                { kind: "pane", paneId: "r" },
                { kind: "pane", paneId: "d" },
              ],
            },
            { kind: "pane", paneId: "b" },
          ],
        },
      ],
    };

    // r's own right side first — Enter here splits JUST r (its height).
    const first = stepPaneTarget(root, { kind: "pane", paneId: "r" }, "right");
    expect(first).toEqual({ kind: "pane-edge", paneId: "r", side: "right" });
    // outward again widens to the seam at the same coordinate (full-height
    // column insert)...
    const second = stepPaneTarget(root, first, "right");
    expect(second).toEqual({ kind: "seam", path: [1] });
    // ...and onward steps normally into the right pane.
    expect(stepPaneTarget(root, second, "right")).toEqual({ kind: "pane", paneId: "b" });
  });

  it("numbers panes in spatial reading order", () => {
    const root: LayoutNode = {
      kind: "split",
      dir: "col",
      ratio: 0.5,
      children: [
        {
          kind: "split",
          dir: "row",
          ratio: 0.5,
          children: [
            { kind: "pane", paneId: "top-left" },
            { kind: "pane", paneId: "top-right" },
          ],
        },
        { kind: "pane", paneId: "bottom" },
      ],
    };

    expect(readingOrderPanes(root).map((p) => p.paneId)).toEqual(["top-left", "top-right", "bottom"]);
  });

  it("finds nearest panes by direction and center distance", () => {
    const root: LayoutNode = {
      kind: "split",
      dir: "col",
      ratio: 0.5,
      children: [
        {
          kind: "split",
          dir: "row",
          ratio: 0.5,
          children: [
            { kind: "pane", paneId: "a" },
            { kind: "pane", paneId: "b" },
          ],
        },
        {
          kind: "split",
          dir: "row",
          ratio: 0.5,
          children: [
            { kind: "pane", paneId: "c" },
            { kind: "pane", paneId: "d" },
          ],
        },
      ],
    };

    expect(nearestPaneInDirection(root, "a", "right")).toBe("b");
    expect(nearestPaneInDirection(root, "a", "down")).toBe("c");
    expect(nearestPane(root, "a")).toBe("b");
  });
});
