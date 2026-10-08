import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  AXIS_LOCK_PX, DOUBLE_TAP_MS, DOUBLE_TAP_SCALE, DOUBLE_TAP_SLOP_PX, MAX_SCALE, MIN_NEXT_SLIDE_SPEED,
  MIN_RATIO_TO_CLOSE, NAV_HALF_RATIO, TAP_SLOP_PX, VERTICAL_DRAG_FRICTION, clampPan,
  createImageViewerGestures, type Transform, type ViewerHost,
} from "./imageViewerGestures";

const BOX = { w: 400, h: 800 };
// The fake clock follows the event timestamps so the single-tap wait is testable.
let now = 0;
const to = (t: number) => { if (t > now) { vi.advanceTimersByTime(t - now); now = t; } };
beforeEach(() => { vi.useFakeTimers(); now = 0; });
afterEach(() => { vi.useRealTimers(); });

function rig(steps: { ok: boolean } = { ok: true }) {
  const log = { steps: [] as number[], closed: 0, taps: 0, last: { t: { scale: 1, x: 0, y: 0 } as Transform, drag: { x: 0, y: 0 } } };
  const host: ViewerHost = {
    box: () => BOX,
    image: () => ({ w: 400, h: 300 }),
    apply: (t, drag) => { log.last = { t, drag }; },
    step: (d) => { log.steps.push(d); return steps.ok; },
    close: () => { log.closed++; },
    tap: () => { log.taps++; },
  };
  return { g: createImageViewerGestures(host), log };
}
type G = ReturnType<typeof rig>["g"];
/** One-finger drag in 10 steps of `perStep` ms, then (optionally) a still pause
 *  before release so the release velocity is zero. Returns the release time. */
function drag(g: G, x0: number, y0: number, dx: number, dy: number, t0 = 0, perStep = 16, pause = false, id = 1) {
  to(t0);
  g.down(id, x0, y0, t0);
  for (let i = 1; i <= 10; i++) { to(t0 + i * perStep); g.move(id, x0 + (dx * i) / 10, y0 + (dy * i) / 10, t0 + i * perStep); }
  let t = t0 + 10 * perStep;
  if (pause) { t += 200; to(t); g.move(id, x0 + dx, y0 + dy, t); }
  g.up(id, x0 + dx, y0 + dy, t);
  return t;
}
/** A quick tap (40 ms touch). */
function tap(g: G, x: number, y: number, t: number) {
  to(t); g.down(1, x, y, t);
  to(t + 40); g.up(1, x, y, t + 40);
}
/** Slow release: more than half the width, or the speed rule. */
const HALF = BOX.w * NAV_HALF_RATIO;

describe("swipe between images (scale 1, horizontal; PhotoSwipe DragHandler.end)", () => {
  it("a slow release turns the page only past half the viewport width", () => {
    const a = rig();
    drag(a.g, 300, 400, -HALF, 0, 0, 16, true);
    expect(a.log.steps).toEqual([]);
    const b = rig();
    drag(b.g, 300, 400, -(HALF + 1), 0, 0, 16, true);
    expect(b.log.steps).toEqual([1]); // left swipe = next
  });
  it("right swipe is previous", () => {
    const r = rig();
    drag(r.g, 100, 400, HALF + 1, 0, 0, 16, true);
    expect(r.log.steps).toEqual([-1]);
  });
  it("a flick turns the page at MIN_NEXT_SLIDE_SPEED, not under it", () => {
    // 10 steps over 50 ms: speed = distance / 50.
    const under = rig();
    drag(under.g, 300, 400, -Math.floor(MIN_NEXT_SLIDE_SPEED * 50) + 1, 0, 0, 5);
    expect(under.log.steps).toEqual([]);
    const over = rig();
    drag(over.g, 300, 400, -Math.ceil(MIN_NEXT_SLIDE_SPEED * 50) - 1, 0, 0, 5);
    expect(over.log.steps).toEqual([1]);
  });
  it("a flick the other way after a long drag does not turn the page", () => {
    const r = rig();
    r.g.down(1, 300, 400, 0);
    for (let i = 1; i <= 10; i++) r.g.move(1, 300 - 30 * i, 400, i * 16);
    // reverse quickly at the end: release velocity is now to the right
    for (let i = 1; i <= 5; i++) r.g.move(1, 0 + 20 * i, 400, 160 + i * 5);
    r.g.up(1, 100, 400, 185);
    expect(r.log.steps).toEqual([]);
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
    drag(r.g, 300, 400, -250, 0, 0, 16, true);
    expect(r.log.steps).toEqual([1]);
    expect(r.log.last.drag).toEqual({ x: 0, y: 0 });
    expect(r.log.closed).toBe(0);
  });
});

describe("vertical drag closes in either direction (PhotoSwipe closeOnVerticalDrag)", () => {
  // The image follows the finger at VERTICAL_DRAG_FRICTION; a still release
  // closes once offset / (viewport height / 3) passes MIN_RATIO_TO_CLOSE.
  const need = (BOX.h / 3) * MIN_RATIO_TO_CLOSE / VERTICAL_DRAG_FRICTION; // 177.8 px of finger travel
  it("downward: just under the still-release distance stays open; just over closes", () => {
    const a = rig();
    drag(a.g, 200, 100, 0, Math.floor(need), 0, 16, true);
    expect(a.log.closed).toBe(0);
    const b = rig();
    drag(b.g, 200, 100, 0, Math.ceil(need), 0, 16, true);
    expect(b.log.closed).toBe(1);
  });
  it("upward closes too, at the same distance", () => {
    const a = rig();
    drag(a.g, 200, 700, 0, -Math.floor(need), 0, 16, true);
    expect(a.log.closed).toBe(0);
    const b = rig();
    drag(b.g, 200, 700, 0, -Math.ceil(need), 0, 16, true);
    expect(b.log.closed).toBe(1);
  });
  it("a fast short flick closes in either direction, a slower one does not", () => {
    // 10 steps over 50 ms; offset 0.6*d plus 199 * (d / 50) must pass 0.4 * 800/3.
    for (const sign of [1, -1]) {
      const fast = rig();
      drag(fast.g, 200, 400, 0, sign * 30, 0, 5);
      expect(fast.log.closed).toBe(1);
      const slow = rig();
      drag(slow.g, 200, 400, 0, sign * 20, 0, 5);
      expect(slow.log.closed).toBe(0);
    }
  });
  it("a drag that reverses past its start does not close in the original direction", () => {
    const r = rig();
    r.g.down(1, 200, 400, 0);
    for (let i = 1; i <= 10; i++) r.g.move(1, 200, 400 + 30 * i, i * 16);
    for (let i = 1; i <= 10; i++) r.g.move(1, 200, 700 - 30 * i, 160 + i * 16); // back to start
    r.g.up(1, 200, 400, 340);
    expect(r.log.closed).toBe(0);
  });
  it("the image follows the finger with friction in both directions", () => {
    const r = rig();
    r.g.down(1, 200, 400, 0);
    r.g.move(1, 200, 400 + 100, 50);
    expect(r.log.last.drag.y).toBeCloseTo(100 * VERTICAL_DRAG_FRICTION, 5);
    r.g.move(1, 200, 400 - 100, 100);
    expect(r.log.last.drag.y).toBeCloseTo(-100 * VERTICAL_DRAG_FRICTION, 5);
  });
  it("a diagonal drag locks to the dominant axis", () => {
    const r = rig();
    drag(r.g, 200, 200, -60, 300, 0, 16, true); // dy dominates -> closes
    expect(r.log.closed).toBe(1);
    const h = rig();
    drag(h.g, 300, 200, -260, 120, 0, 16, true); // dx dominates -> page turn
    expect(h.log.closed).toBe(0);
    expect(h.log.steps).toEqual([1]);
  });
  it("axis lock waits for AXIS_LOCK_PX of travel", () => {
    const r = rig();
    r.g.down(1, 200, 200, 0);
    r.g.move(1, 200, 200 + AXIS_LOCK_PX, 20);
    expect(r.log.last.drag).toEqual({ x: 0, y: 0 });
    r.g.move(1, 200, 200 + AXIS_LOCK_PX + 1, 40);
    expect(r.log.last.drag.y).toBeCloseTo((AXIS_LOCK_PX + 1) * VERTICAL_DRAG_FRICTION, 5);
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

describe("tap toggles the controls (PhotoSwipe tapAction), double-tap zooms", () => {
  it("a single tap fires only after the double-tap wait", () => {
    const r = rig();
    tap(r.g, 300, 400, 0);
    expect(r.log.taps).toBe(0);
    to(40 + DOUBLE_TAP_MS - 1);
    expect(r.log.taps).toBe(0);
    to(40 + DOUBLE_TAP_MS);
    expect(r.log.taps).toBe(1);
    expect(r.g.transform().scale).toBe(1);
  });
  it("two quick taps zoom to DOUBLE_TAP_SCALE about the tap point and fire no tap; another pair resets", () => {
    const r = rig();
    tap(r.g, 300, 400, 0);
    tap(r.g, 300, 400, 120);
    to(2000);
    expect(r.g.transform().scale).toBe(DOUBLE_TAP_SCALE);
    expect(r.g.transform().x).toBeLessThanOrEqual(0);
    expect(r.log.taps).toBe(0);
    tap(r.g, 300, 400, 3000);
    tap(r.g, 300, 400, 3120);
    expect(r.g.transform()).toEqual({ scale: 1, x: 0, y: 0 });
    expect(r.log.taps).toBe(0);
  });
  it("just outside the double-tap window it is two single taps", () => {
    const r = rig();
    tap(r.g, 300, 400, 0);
    tap(r.g, 300, 400, 40 + DOUBLE_TAP_MS + 1); // the first already fired
    to(2000);
    expect(r.g.transform().scale).toBe(1);
    expect(r.log.taps).toBe(2);
  });
  it("a second tap at DOUBLE_TAP_SLOP_PX is two single taps; just inside it zooms", () => {
    const far = rig();
    tap(far.g, 100, 400, 0);
    tap(far.g, 100 + DOUBLE_TAP_SLOP_PX, 400, 100);
    to(2000);
    expect(far.g.transform().scale).toBe(1);
    expect(far.log.taps).toBe(2);
    const near = rig();
    tap(near.g, 100, 400, 0);
    tap(near.g, 100 + DOUBLE_TAP_SLOP_PX - 1, 400, 100);
    to(2000);
    expect(near.g.transform().scale).toBe(DOUBLE_TAP_SCALE);
    expect(near.log.taps).toBe(0);
  });
  it("a touch that moves just over TAP_SLOP_PX is a drag, not a tap", () => {
    const r = rig();
    to(0); r.g.down(1, 300, 400, 0);
    to(20); r.g.move(1, 300 + TAP_SLOP_PX + 1, 400, 20);
    to(50); r.g.up(1, 300 + TAP_SLOP_PX + 1, 400, 50);
    to(2000);
    expect(r.log.taps).toBe(0);
    tap(r.g, 300, 400, 2100);
    to(3000);
    expect(r.log.taps).toBe(1);
  });
  it("just under TAP_SLOP_PX of movement is still a tap", () => {
    const r = rig();
    to(0); r.g.down(1, 300, 400, 0);
    to(20); r.g.move(1, 300 + TAP_SLOP_PX, 400, 20);
    to(50); r.g.up(1, 300 + TAP_SLOP_PX, 400, 50);
    to(2000);
    expect(r.log.taps).toBe(1);
  });
  it("a tap on a zoomed image toggles the controls too", () => {
    const r = rig();
    tap(r.g, 200, 400, 0); tap(r.g, 200, 400, 120); // zoom
    to(1000);
    tap(r.g, 200, 400, 1100);
    to(2000);
    expect(r.log.taps).toBe(1);
    expect(r.g.transform().scale).toBe(DOUBLE_TAP_SCALE);
  });
  it("reset (a new image) drops a pending single tap", () => {
    const r = rig();
    tap(r.g, 200, 400, 0);
    r.g.reset();
    to(2000);
    expect(r.log.taps).toBe(0);
  });
  it("the click after the second tap is swallowed; after a plain single tap it is not", () => {
    const r = rig();
    tap(r.g, 300, 400, 0);
    expect(r.g.swallowClick(60)).toBe(false);
    tap(r.g, 300, 400, 100);
    expect(r.g.swallowClick(200)).toBe(true);
    expect(r.g.swallowClick(140 + 1000)).toBe(false);
  });
});

describe("pan while zoomed", () => {
  function zoomed() {
    const r = rig();
    tap(r.g, 200, 400, 0); tap(r.g, 200, 400, 120); // centre double-tap: x stays 0
    to(1000);
    return r;
  }
  it("a one-finger drag inside the image pans (never turns the page or closes)", () => {
    const r = zoomed();
    expect(r.g.transform().scale).toBe(DOUBLE_TAP_SCALE);
    drag(r.g, 200, 400, -100, 0, 1500, 20);
    expect(r.g.transform().x).toBe(-100);
    expect(r.log.steps).toEqual([]);
    drag(r.g, 200, 200, 0, 300, 2500, 20);
    expect(r.log.closed).toBe(0);
  });
  it("is clamped to the edges of the zoomed image", () => {
    const r = zoomed();
    drag(r.g, 200, 400, -5000, 0, 1500, 20);
    // image 400 wide * 2.5 = 1000; box 400 -> max pan 300
    expect(r.g.transform().x).toBe(-300);
    expect(clampPan({ scale: 2.5, x: 0, y: 9999 }, BOX, { w: 400, h: 300 }).y).toBe(0); // 750 < 800: no vertical room
  });
});

describe("pan to the next image at a zoom edge (PhotoSwipe allowPanToNext)", () => {
  /** Zoomed to 2.5x and panned to the far right edge (x = -300). */
  function atRightEdge(ok = true) {
    const r = rig({ ok });
    tap(r.g, 200, 400, 0); tap(r.g, 200, 400, 120);
    drag(r.g, 200, 400, -5000, 0, 1500, 20);
    expect(r.g.transform().x).toBe(-300);
    expect(r.log.steps).toEqual([]); // that drag started inside the image: friction, no page turn
    return r;
  }
  it("a drag that started inside never turns the page, however far past the edge", () => {
    const r = atRightEdge();
    expect(r.log.last.drag).toEqual({ x: 0, y: 0 });
  });
  it("a new drag from the right edge moves the strip and a slow release past half the width turns the page", () => {
    const r = atRightEdge();
    drag(r.g, 300, 400, -HALF, 0, 4000, 20, true);
    expect(r.log.steps).toEqual([]);
    expect(r.g.transform().x).toBe(-300);
    drag(r.g, 300, 400, -(HALF + 1), 0, 6000, 20, true);
    expect(r.log.steps).toEqual([1]);
    expect(r.g.transform()).toEqual({ scale: 1, x: 0, y: 0 }); // new image at rest
  });
  it("the strip follows the finger past the edge, then returns to rest on a short release", () => {
    const r = atRightEdge();
    r.g.down(1, 300, 400, 4000);
    to(4050); r.g.move(1, 250, 400, 4050);
    expect(r.log.last.drag.x).toBe(-50);
    expect(r.log.last.t.x).toBe(-300);
    to(4500); r.g.move(1, 250, 400, 4500);
    r.g.up(1, 250, 400, 4500);
    expect(r.log.steps).toEqual([]);
    expect(r.log.last.drag).toEqual({ x: 0, y: 0 });
  });
  it("a flick from the edge turns the page at MIN_NEXT_SLIDE_SPEED", () => {
    const r = atRightEdge();
    drag(r.g, 300, 400, -(Math.ceil(MIN_NEXT_SLIDE_SPEED * 50) + 1), 0, 4000, 5);
    expect(r.log.steps).toEqual([1]);
  });
  it("from the left edge a rightward drag goes to the previous image", () => {
    const r = rig();
    tap(r.g, 200, 400, 0); tap(r.g, 200, 400, 120);
    drag(r.g, 100, 400, 5000, 0, 1500, 20);
    expect(r.g.transform().x).toBe(300);
    expect(r.log.steps).toEqual([]);
    drag(r.g, 100, 400, HALF + 1, 0, 4000, 20, true);
    expect(r.log.steps).toEqual([-1]);
  });
  it("dragging back from the edge pans the image, not the strip", () => {
    const r = atRightEdge();
    drag(r.g, 100, 400, 100, 0, 4000, 20, true);
    expect(r.g.transform().x).toBe(-200);
    expect(r.log.steps).toEqual([]);
  });
  it("with no next image the strip snaps back", () => {
    const r = atRightEdge(false);
    drag(r.g, 300, 400, -(HALF + 1), 0, 4000, 20, true);
    expect(r.log.steps).toEqual([1]);
    expect(r.log.last.drag).toEqual({ x: 0, y: 0 });
    expect(r.g.transform().scale).toBe(DOUBLE_TAP_SCALE);
  });
  it("a mostly vertical drag at the edge never turns the page", () => {
    const r = atRightEdge();
    drag(r.g, 300, 400, -(HALF + 1), 3 * HALF, 4000, 20, true);
    expect(r.log.steps).toEqual([]);
    expect(r.log.closed).toBe(0);
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
