import { describe, expect, it } from "vitest";
import { createWheelZoomAccumulator, highestPriorityPage } from "./pdfRenderQueue";

const notDone = () => false;

describe("highestPriorityPage", () => {
  it("renders visible pages first, in priority order", () => {
    expect(highestPriorityPage({ visible: [5, 4], numPages: 20, scrollingDown: true, prefetch: 2, done: notDone })).toBe(5);
    expect(highestPriorityPage({ visible: [5, 4], numPages: 20, scrollingDown: true, prefetch: 2,
      done: (page) => page === 5 })).toBe(4);
  });

  it("fills holes between visible pages before prefetching", () => {
    expect(highestPriorityPage({ visible: [3, 6], numPages: 20, scrollingDown: true, prefetch: 2,
      done: (page) => page === 3 || page === 6 })).toBe(4);
  });

  it("prefetches ahead in the scroll direction, up to the prefetch count", () => {
    const visibleDone = (page: number) => page === 5;
    expect(highestPriorityPage({ visible: [5], numPages: 20, scrollingDown: true, prefetch: 2, done: visibleDone })).toBe(6);
    expect(highestPriorityPage({ visible: [5], numPages: 20, scrollingDown: false, prefetch: 2, done: visibleDone })).toBe(4);
    expect(highestPriorityPage({ visible: [5], numPages: 20, scrollingDown: true, prefetch: 2,
      done: (page) => page === 5 || page === 6 })).toBe(7);
    expect(highestPriorityPage({ visible: [5], numPages: 20, scrollingDown: true, prefetch: 2,
      done: (page) => page >= 5 && page <= 7 })).toBeNull();
  });

  it("prefetches away from the document's ends whatever the last direction", () => {
    expect(highestPriorityPage({ visible: [1], numPages: 20, scrollingDown: false, prefetch: 1,
      done: (page) => page === 1 })).toBe(2);
    expect(highestPriorityPage({ visible: [20], numPages: 20, scrollingDown: true, prefetch: 1,
      done: (page) => page === 20 })).toBe(19);
  });

  it("has nothing to do without visible pages", () => {
    expect(highestPriorityPage({ visible: [], numPages: 20, scrollingDown: true, prefetch: 2, done: notDone })).toBeNull();
  });
});

describe("createWheelZoomAccumulator", () => {
  const wheel = (deltaY: number, deltaMode = 0) => ({ deltaY, deltaX: 0, deltaMode });

  it("turns a wheel notch into one step and accumulates pixel deltas 30 px per step", () => {
    const zoom = createWheelZoomAccumulator();
    expect(zoom.steps(wheel(-3, 1))).toBe(1);
    expect(zoom.steps(wheel(3, 1))).toBe(-1);
    // A high-resolution wheel or touchpad: ten 3 px events are one step, not ten.
    let steps = 0;
    for (let i = 0; i < 10; i++) steps += zoom.steps(wheel(-3));
    expect(steps).toBe(1);
  });

  it("drops the remainder when the direction reverses", () => {
    const zoom = createWheelZoomAccumulator();
    expect(zoom.steps(wheel(-20))).toBe(0);
    expect(zoom.steps(wheel(20))).toBe(0); // not +20-20 = a completed step either way
    expect(zoom.steps(wheel(15))).toBe(-1);
  });

  it("recognises a trackpad pinch and applies a continuous factor without losing slow pinches", () => {
    const zoom = createWheelZoomAccumulator();
    expect(zoom.isPinch({ ctrlKey: true, deltaMode: 0, deltaX: 0, deltaY: -2, deltaZ: 0 }, false)).toBe(true);
    expect(zoom.isPinch({ ctrlKey: true, deltaMode: 0, deltaX: 0, deltaY: -2, deltaZ: 0 }, true)).toBe(false);
    expect(zoom.isPinch({ ctrlKey: true, deltaMode: 1, deltaX: 0, deltaY: -3, deltaZ: 0 }, false)).toBe(false);
    let scale = 1;
    for (let i = 0; i < 20; i++) scale = Math.round(scale * zoom.pinchFactor(scale, -0.4) * 100) / 100;
    // 20 × exp(0.004) ≈ ×1.083: rounding to 1/100 per event must not swallow it.
    expect(scale).toBeGreaterThanOrEqual(1.07);
    expect(scale).toBeLessThanOrEqual(1.09);
  });
});
