import { describe, expect, it } from "vitest";
import { placeContextMenu } from "./ContextMenu";

// Viewport-aware menu placement (the "Delete namespace" nit: a tall menu opened
// low in the sidebar spilled off the bottom and clipped its items). Margin = 6.
const VW = 1000;
const VH = 800;
const place = (x: number, y: number, w: number, h: number) => placeContextMenu(x, y, w, h, VW, VH);

describe("placeContextMenu", () => {
  it("opens down from the cursor when there is room below", () => {
    expect(place(120, 100, 180, 200)).toEqual({ left: 120, top: 100 });
  });

  it("flips up (bottom anchored at the cursor) when opening down would overflow", () => {
    // y=760, h=200 → down-edge 960 > 800 → open up: top = 760 - 200 = 560.
    expect(place(120, 760, 180, 200)).toEqual({ left: 120, top: 560 });
  });

  it("clamps to the top edge when the menu is taller than the viewport", () => {
    // Even flipped up (y - h = -100) it overflows the top → clamp to margin.
    expect(place(120, 700, 180, 900)).toEqual({ left: 120, top: 6 });
  });

  it("clamps horizontally so the right edge stays on-screen", () => {
    // x=950, w=180 → right edge 1130 > 1000 → left = 1000 - 180 - 6 = 814.
    expect(place(950, 100, 180, 200).left).toBe(814);
  });

  it("never places past the left/top margin (a near-edge click is nudged to the margin)", () => {
    expect(place(2, 2, 180, 100)).toEqual({ left: 6, top: 6 });
  });

  it("just fits below when the down-edge lands exactly on the safe boundary", () => {
    // down-edge = y + h = 794 = vh - margin → not greater → stays down.
    expect(place(50, 594, 180, 200)).toEqual({ left: 50, top: 594 });
  });
});
