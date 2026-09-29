// Pane-select keyboard navigation from an off-centre seam (master 91d102fba4; og-G2 row 45).
import { describe, expect, it } from "vitest";
import { stepPaneTarget, type PaneTarget } from "./paneSelect";
import type { LayoutNode } from "./panes";

describe("og-G2 91d102fb pane seam navigation", () => {
  it("steps from an off-centre seam into its adjacent pane, not a perpendicular window edge", () => {
    const root: LayoutNode = {
      kind: "split",
      dir: "row",
      ratio: 0.4,
      children: [
        { kind: "pane", paneId: "left" },
        { kind: "pane", paneId: "wide-right" },
      ],
    } as LayoutNode;
    const seam: PaneTarget = { kind: "seam", path: [] };
    expect(stepPaneTarget(root, { kind: "pane", paneId: "left" }, "right")).toEqual(seam);
    expect(stepPaneTarget(root, seam, "right")).toEqual({ kind: "pane", paneId: "wide-right" });
  });
});
