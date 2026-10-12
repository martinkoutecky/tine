import { afterEach, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import {
  MAIN_PANE_MIN,
  RIGHT_MAX_WINDOW_FRACTION,
  RIGHT_MIN,
  adjustSidebarWidth,
  resizeSidebar,
  rightSidebarMaxWidth,
  rightSidebarWidth,
  setLeftSidebarReserve,
  setRightSidebarWidth,
} from "./sidebarSizing";

afterEach(() => {
  setLeftSidebarReserve(0);
  setRightSidebarWidth(360);
});

// GH #521: OG caps its right sidebar at 70% of the window (og-1.0.0
// components/right_sidebar.cljs sidebar-resizer, max-ratio 0.7). Tine's old fixed 800px
// cap left a 2560px-wide window mostly unused. The new bound is 70% of the window,
// never leaving the main pane (beside an open left sidebar) under 360px.
describe("right sidebar maximum width (GH #521)", () => {
  it("is 70% of a wide window, far beyond the old 800px cap", () => {
    expect(RIGHT_MAX_WINDOW_FRACTION).toBe(0.7);
    expect(rightSidebarMaxWidth(2560, 0)).toBe(1792);
    expect(rightSidebarMaxWidth(1920, 0)).toBe(1344);
  });

  it("keeps the main pane at least 360px wide beside the left sidebar", () => {
    expect(MAIN_PANE_MIN).toBe(360);
    for (const [windowWidth, reserved] of [[1280, 246], [1100, 0], [900, 300], [1920, 500]]) {
      const max = rightSidebarMaxWidth(windowWidth, reserved);
      expect(windowWidth - reserved - max).toBeGreaterThanOrEqual(MAIN_PANE_MIN);
    }
    expect(rightSidebarMaxWidth(1280, 246)).toBe(674);
    expect(rightSidebarMaxWidth(1280, 0)).toBe(896);
  });

  it("never drops below the minimum on a small window", () => {
    expect(rightSidebarMaxWidth(500, 246)).toBe(RIGHT_MIN);
  });

  it("pointer resizing clamps to it and keyboard resizing follows", () => {
    setLeftSidebarReserve(246);
    resizeSidebar("right", 5000, 1280);
    expect(rightSidebarWidth()).toBe(674);
    resizeSidebar("right", 100, 1280);
    expect(rightSidebarWidth()).toBe(RIGHT_MIN);
    setRightSidebarWidth(660);
    // 660 + 48 would pass 674 on a 1280 window; the shared clamp stops it at the bound.
    resizeSidebar("right", rightSidebarWidth() + 48, 1280);
    expect(rightSidebarWidth()).toBe(674);
    setRightSidebarWidth(1000);
    adjustSidebarWidth("right", true); // default test window: no throw, still a number >= min
    expect(rightSidebarWidth()).toBeGreaterThanOrEqual(RIGHT_MIN);
  });

  it("the stylesheet guard for a stale wide stored width uses the same bounds", () => {
    const css = readFileSync("src/styles/app.css", "utf8");
    const rule = /\n\.right-sidebar \{([^}]*)\}/.exec(css)?.[1] ?? "";
    expect(rule).toContain(`max-width: min(${RIGHT_MAX_WINDOW_FRACTION * 100}vw, calc(100vw - ${MAIN_PANE_MIN}px))`);
  });
});
