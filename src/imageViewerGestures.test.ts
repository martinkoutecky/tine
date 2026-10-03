import { describe, expect, it } from "vitest";
import {
  AXIS_LOCK_PX, CLOSE_SWIPE_PX, DOUBLE_TAP_MS, DOUBLE_TAP_SCALE, DOUBLE_TAP_SLOP_PX, FLICK_MIN_PX,
  MAX_SCALE, NAV_SWIPE_PX, TAP_SLOP_PX, clampPan, createImageViewerGestures, type Transform, type ViewerHost,
} from "./imageViewerGestures";

function rig(steps: { ok: boolean } = { ok: true }) {
  const log = { steps: [] as number[], closed: 0, last: { t: { scale: 1, x: 0, y: 0 } as Transform, drag: { x: 0, y: 0 } } };
  const host: ViewerHost = {
    box: () => ({ w: 400, h: 800 }),
    image: () => ({ w: 400, h: 300 }),
    apply: (t, drag) => { log.last = { t, drag }; },
    step: (d) => { log.steps.push(d); return steps.ok; },
    close: () => { log.closed++; },
  };
  return { g: createImageViewerGestures(host), log };
}
/** One-finger drag in 10 steps of 16 ms, returning the release time. */
function drag(g: ReturnType<typeof rig>["g"], x0: number, y0: number, dx: number, dy: number, t0 = 0, perStep = 16, id = 1) {
  g.down(id, x0, y0, t0);
  for (let i = 1; i <= 10; i++) g.move(id, x0 + (dx * i) / 10, y0 + (dy * i) / 10, t0 + i * perStep);
  g.up(id, x0 + dx, y0 + dy, t0 + 10 * perStep);
  return t0 + 10 * perStep;
}

describe("swipe between images (scale 1, horizontal)", () => {
  it("just under the page-turn distance (slow) snaps back; just over turns", () => {
    const a = rig();
    drag(a.g, 200, 400, -(NAV_SWIPE_PX - 1), 0, 0, 100); // slow: no flick
    expect(a.log.steps).toEqual([]);
    const b = rig();
    drag(b.g, 200, 400, -NAV_SWIPE_PX, 0, 0, 100);
    expect(b.log.steps).toEqual([1]); // left swipe = next
  });
  it("right swipe is previous", () => {
    const r = rig();
    drag(r.g, 100, 400, 80, 0, 0, 100);
    expect(r.log.steps).toEqual([-1]);
  });
  it("a fast short flick turns the page once past FLICK_MIN_PX, not under it", () => {
    const under = rig();
    drag(under.g, 200, 400, -(FLICK_MIN_PX - 1), 0, 0, 5);
    expect(under.log.steps).toEqual([]);
    const over = rig();
    drag(over.g, 200, 400, -FLICK_MIN_PX - 1, 0, 0, 5);
    expect(over.log.steps).toEqual([1]);
  });
  it("the image follows the finger while dragging and returns to rest on release", () => {
    const r = rig();
    r.g.down(1, 200, 400, 0);
    r.g.move(1, 150, 400, 50);
    expect(r.log.last.drag).toEqual({ x: -50, y: 0 });
    r.g.up(1, 150, 400, 400);
    expect(r.log.last.drag).toEqual({ x: 0, y: 0 });
  });
  it("at the edge (no next image) it snaps back", () => {
    const r = rig({ ok: false });
    drag(r.g, 200, 400, -120, 0, 0, 100);
    expect(r.log.steps).toEqual([1]);
    expect(r.log.last.drag).toEqual({ x: 0, y: 0 });
    expect(r.log.closed).toBe(0);
  });
});

describe("swipe down to close", () => {
  it("just under the close distance (slow) stays open; just over closes", () => {
    const a = rig();
    drag(a.g, 200, 200, 0, CLOSE_SWIPE_PX - 1, 0, 100);
    expect(a.log.closed).toBe(0);
    const b = rig();
    drag(b.g, 200, 200, 0, CLOSE_SWIPE_PX, 0, 100);
    expect(b.log.closed).toBe(1);
  });
  it("swiping UP does not close", () => {
    const r = rig();
    drag(r.g, 200, 600, 0, -200, 0, 100);
    expect(r.log.closed).toBe(0);
  });
  it("a fast short downward flick closes", () => {
    const r = rig();
    drag(r.g, 200, 200, 0, 40, 0, 5);
    expect(r.log.closed).toBe(1);
  });
  it("a diagonal drag locks to the dominant axis: mostly horizontal never closes", () => {
    const r = rig();
    drag(r.g, 200, 200, -60, 120, 0, 100); // dy dominates -> v
    expect(r.log.closed).toBe(1);
    const h = rig();
    drag(h.g, 200, 200, -130, 120, 0, 100); // dx dominates -> h
    expect(h.log.closed).toBe(0);
    expect(h.log.steps).toEqual([1]);
  });
  it("axis lock waits for AXIS_LOCK_PX of travel", () => {
    const r = rig();
    r.g.down(1, 200, 200, 0);
    r.g.move(1, 200, 200 + AXIS_LOCK_PX, 20);
    expect(r.log.last.drag).toEqual({ x: 0, y: 0 });
    r.g.move(1, 200, 200 + AXIS_LOCK_PX + 1, 40);
    expect(r.log.last.drag.y).toBe(AXIS_LOCK_PX + 1);
  });
});

describe("pinch zoom", () => {
  const pinch = (r: ReturnType<typeof rig>, d0: number, d1: number) => {
    r.g.down(1, 200 - d0 / 2, 400, 0);
    r.g.down(2, 200 + d0 / 2, 400, 0);
    r.g.move(1, 200 - d1 / 2, 400, 50);
    r.g.move(2, 200 + d1 / 2, 400, 50);
  };
  it("spreading fingers scales proportionally about the midpoint", () => {
    const r = rig();
    pinch(r, 100, 250);
    expect(r.log.last.t.scale).toBeCloseTo(2.5, 5);
    expect(r.log.last.t.x).toBeCloseTo(0, 5); // midpoint at centre: no shift
  });
  it("is clamped at MAX_SCALE on release", () => {
    const r = rig();
    pinch(r, 50, 600);
    r.g.up(1, 0, 400, 100);
    r.g.up(2, 400, 400, 100);
    expect(r.log.last.t.scale).toBe(MAX_SCALE);
  });
  it("pinching in past 1 settles back to scale 1 at rest", () => {
    const r = rig();
    pinch(r, 200, 120);
    expect(r.log.last.t.scale).toBeCloseTo(0.8, 5); // rubber-band floor
    r.g.up(1, 0, 400, 100);
    r.g.up(2, 400, 400, 100);
    expect(r.log.last.t).toEqual({ scale: 1, x: 0, y: 0 });
  });
  it("a pinch released is not a tap or a swipe, and the click after it is swallowed", () => {
    const r = rig();
    pinch(r, 100, 200);
    r.g.up(1, 150, 400, 100);
    r.g.up(2, 250, 400, 100);
    expect(r.log.steps).toEqual([]);
    expect(r.log.closed).toBe(0);
    expect(r.g.swallowClick(150)).toBe(true);
  });
  it("keeps the content point under the fingers (off-centre pinch pans the image)", () => {
    const r = rig();
    // midpoint (300, 400) is 100 px right of centre (200,400)
    r.g.down(1, 250, 400, 0);
    r.g.down(2, 350, 400, 0);
    r.g.move(1, 200, 400, 50);
    r.g.move(2, 400, 400, 50);
    const t = r.log.last.t;
    expect(t.scale).toBeCloseTo(2, 5);
    expect(t.x).toBeCloseTo(100 - 100 * 2, 5); // mid.x - c*s with c = 100
  });
});

describe("double-tap zoom", () => {
  it("two quick taps zoom to DOUBLE_TAP_SCALE about the tap point; another pair resets", () => {
    const r = rig();
    r.g.down(1, 300, 400, 0); r.g.up(1, 300, 400, 50);
    expect(r.g.transform().scale).toBe(1);
    r.g.down(1, 300, 400, 120); r.g.up(1, 300, 400, 160);
    expect(r.g.transform().scale).toBe(DOUBLE_TAP_SCALE);
    // tap point 100px right of centre stays put: x = r*(1-s), clamped to the pan range
    expect(r.g.transform().x).toBeLessThanOrEqual(0);
    r.g.down(1, 300, 400, 300); r.g.up(1, 300, 400, 340);
    r.g.down(1, 300, 400, 380); r.g.up(1, 300, 400, 420);
    expect(r.g.transform()).toEqual({ scale: 1, x: 0, y: 0 });
  });
  it("just outside the double-tap window it is two single taps", () => {
    const r = rig();
    r.g.down(1, 300, 400, 0); r.g.up(1, 300, 400, 50);
    r.g.down(1, 300, 400, 50 + DOUBLE_TAP_MS + 1); r.g.up(1, 300, 400, 50 + DOUBLE_TAP_MS + 40);
    expect(r.g.transform().scale).toBe(1);
  });
  it("just outside the double-tap radius it is two single taps; inside it zooms", () => {
    const far = rig();
    far.g.down(1, 100, 400, 0); far.g.up(1, 100, 400, 50);
    far.g.down(1, 100 + DOUBLE_TAP_SLOP_PX + 1, 400, 100); far.g.up(1, 100 + DOUBLE_TAP_SLOP_PX + 1, 400, 140);
    expect(far.g.transform().scale).toBe(1);
    const near = rig();
    near.g.down(1, 100, 400, 0); near.g.up(1, 100, 400, 50);
    near.g.down(1, 100 + DOUBLE_TAP_SLOP_PX, 400, 100); near.g.up(1, 100 + DOUBLE_TAP_SLOP_PX, 400, 140);
    expect(near.g.transform().scale).toBe(DOUBLE_TAP_SCALE);
  });
  it("a touch that moves just over TAP_SLOP_PX is a drag, not a tap", () => {
    const r = rig();
    r.g.down(1, 300, 400, 0); r.g.move(1, 300 + TAP_SLOP_PX + 1, 400, 20); r.g.up(1, 300 + TAP_SLOP_PX + 1, 400, 50);
    r.g.down(1, 300, 400, 100); r.g.up(1, 300, 400, 130);
    expect(r.g.transform().scale).toBe(1);
  });
  it("the click after the second tap is swallowed (it must not close the viewer)", () => {
    const r = rig();
    r.g.down(1, 300, 400, 0); r.g.up(1, 300, 400, 50);
    r.g.down(1, 300, 400, 100); r.g.up(1, 300, 400, 140);
    expect(r.g.swallowClick(200)).toBe(true);
    expect(r.g.swallowClick(140 + 1000)).toBe(false);
  });
  it("a plain single tap does not swallow the click", () => {
    const r = rig();
    r.g.down(1, 300, 400, 0); r.g.up(1, 300, 400, 50);
    expect(r.g.swallowClick(60)).toBe(false);
  });
});

describe("pan while zoomed", () => {
  function zoomed() {
    const r = rig();
    r.g.down(1, 200, 400, 0); r.g.up(1, 200, 400, 50);
    r.g.down(1, 200, 400, 100); r.g.up(1, 200, 400, 140); // centre tap: x stays 0
    return r;
  }
  it("a one-finger drag pans (never turns the page or closes)", () => {
    const r = zoomed();
    expect(r.g.transform().scale).toBe(DOUBLE_TAP_SCALE);
    drag(r.g, 200, 400, -100, 0, 500, 20);
    expect(r.g.transform().x).toBe(-100);
    expect(r.log.steps).toEqual([]);
    drag(r.g, 200, 200, 0, 300, 900, 20);
    expect(r.log.closed).toBe(0);
  });
  it("is clamped to the edges of the zoomed image", () => {
    const r = zoomed();
    drag(r.g, 200, 400, -5000, 0, 500, 20);
    // image 400 wide * 2.5 = 1000; box 400 -> max pan 300
    expect(r.g.transform().x).toBe(-300);
    expect(clampPan({ scale: 2.5, x: 0, y: 9999 }, { w: 400, h: 800 }, { w: 400, h: 300 }).y).toBe(0); // 750 < 800: no vertical room
  });
});

describe("reset / cancel", () => {
  it("reset returns to rest", () => {
    const r = rig();
    r.g.down(1, 300, 400, 0); r.g.up(1, 300, 400, 50);
    r.g.down(1, 300, 400, 100); r.g.up(1, 300, 400, 140);
    r.g.reset();
    expect(r.g.transform()).toEqual({ scale: 1, x: 0, y: 0 });
  });
  it("a cancelled drag neither navigates nor closes", () => {
    const r = rig();
    r.g.down(1, 200, 200, 0);
    r.g.move(1, 200, 400, 50);
    r.g.cancel(1);
    expect(r.log.closed).toBe(0);
    expect(r.log.last.drag).toEqual({ x: 0, y: 0 });
  });
});
