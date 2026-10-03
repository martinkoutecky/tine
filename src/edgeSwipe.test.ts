import { describe, expect, it, vi } from "vitest";
import {
  BACK_COMMIT_MIN_PX, BACK_FLICK_MIN_PX, DECIDE_PX, DRAWER_OPEN_PX, EDGE_PX,
  createEdgeSwipe, type EdgeSwipeHost,
} from "./edgeSwipe";

function host(over: Partial<EdgeSwipeHost> = {}) {
  const h = {
    platform: "ios" as const,
    backAvailable: vi.fn(() => true),
    drawerOpenable: vi.fn(() => true),
    viewportWidth: () => 400,
    progress: vi.fn(),
    settle: vi.fn(),
    openDrawer: vi.fn(),
    back: vi.fn(),
    ...over,
  } satisfies EdgeSwipeHost;
  return h;
}

/** Drive one swipe: start at (x0,y0), move through `moves`, end at the last. */
function drag(h: EdgeSwipeHost, x0: number, y0: number, moves: Array<[number, number]>, dt = 300) {
  const s = createEdgeSwipe(h);
  const started = s.start(x0, y0, 0);
  let last: [number, number] = [x0, y0];
  const claims: string[] = [];
  moves.forEach(([x, y], i) => {
    claims.push(s.move(x, y, ((i + 1) * dt) / moves.length));
    last = [x, y];
  });
  s.end(last[0], last[1], dt);
  return { s, started, claims };
}

describe("left-edge swipe zone (one recognizer for iOS Back and the drawer pull)", () => {
  it("starts only inside the edge strip", () => {
    expect(createEdgeSwipe(host()).start(EDGE_PX, 100, 0)).toBe(true);
    expect(createEdgeSwipe(host()).start(EDGE_PX + 1, 100, 0)).toBe(false);
  });

  it("ignores a start when there is nothing to do", () => {
    expect(createEdgeSwipe(host({ backAvailable: () => false, drawerOpenable: () => false })).start(5, 100, 0)).toBe(false);
  });
});

describe("axis lock (OG has no vertical check; Tine decides in the first few px)", () => {
  it("a vertical-dominant start is never ours and never claimed", () => {
    const h = host();
    const { claims, s } = drag(h, 5, 100, [[5 + DECIDE_PX - 1, 100 + DECIDE_PX + 20], [60, 200]]);
    expect(claims).toEqual(["pass", "pass"]);
    expect(h.progress).not.toHaveBeenCalled();
    expect(h.back).not.toHaveBeenCalled();
    expect(h.openDrawer).not.toHaveBeenCalled();
    expect(s.state).toBe("idle");
  });

  it("a leftward start is not ours", () => {
    const h = host();
    const { claims } = drag(h, 5, 100, [[5 - DECIDE_PX - 1, 100]]);
    expect(claims[0]).toBe("pass");
    expect(h.progress).not.toHaveBeenCalled();
  });

  it("stays undecided just under the decide distance, claims just over", () => {
    const s = createEdgeSwipe(host());
    s.start(5, 100, 0);
    expect(s.move(5 + DECIDE_PX - 1, 100, 10)).toBe("pass");
    expect(s.state).toBe("undecided");
    expect(s.move(5 + DECIDE_PX, 100, 20)).toBe("claim");
    expect(s.state).toBe("tracking");
  });

  it("a later vertical drift does not release an already-locked horizontal swipe", () => {
    const h = host();
    const s = createEdgeSwipe(h);
    s.start(5, 100, 0);
    s.move(5 + DECIDE_PX + 4, 100, 10);
    expect(s.move(5 + DECIDE_PX + 8, 100 + 60, 20)).toBe("claim");
    expect(h.progress).toHaveBeenCalledTimes(2);
  });
});

describe("iOS Back: the page follows the finger, release decides", () => {
  it("reports progress to the finger, rightward only", () => {
    const h = host();
    const s = createEdgeSwipe(h);
    s.start(5, 100, 0);
    s.move(5 + 30, 100, 10);
    expect(h.progress).toHaveBeenLastCalledWith(30);
  });

  it("snaps back just under the commit distance (slow)", () => {
    const h = host(); // width 400: 30% = 120
    drag(h, 5, 100, [[5 + 119, 100]], 1000);
    expect(h.settle).toHaveBeenCalledWith(false, 119);
    expect(h.back).not.toHaveBeenCalled();
  });

  it("goes back at the commit distance (slow)", () => {
    const h = host();
    drag(h, 5, 100, [[5 + 120, 100]], 1000);
    expect(h.settle).toHaveBeenCalledWith(true, 120);
    expect(h.back).toHaveBeenCalledOnce();
  });

  it("the floor wins on a narrow viewport", () => {
    const h = host({ viewportWidth: () => 100 }); // 30% = 30 < floor
    drag(h, 5, 100, [[5 + BACK_COMMIT_MIN_PX - 1, 100]], 2000);
    expect(h.back).not.toHaveBeenCalled();
    drag(h, 5, 100, [[5 + BACK_COMMIT_MIN_PX, 100]], 2000);
    expect(h.back).toHaveBeenCalledOnce();
  });

  it("a flick commits over a short distance; a short slow drag does not", () => {
    const fast = host();
    drag(fast, 5, 100, [[5 + BACK_FLICK_MIN_PX, 100]], 40); // 0.8 px/ms
    expect(fast.back).toHaveBeenCalledOnce();

    const tooShort = host();
    drag(tooShort, 5, 100, [[5 + BACK_FLICK_MIN_PX - 1, 100]], 10); // fast but under distance
    expect(tooShort.back).not.toHaveBeenCalled();

    const slow = host();
    drag(slow, 5, 100, [[5 + BACK_FLICK_MIN_PX + 20, 100]], 1000);
    expect(slow.back).not.toHaveBeenCalled();
  });

  it("a cancelled touch snaps back and never goes back", () => {
    const h = host();
    const s = createEdgeSwipe(h);
    s.start(5, 100, 0);
    s.move(5 + 80, 100, 10);
    s.cancel();
    expect(h.settle).toHaveBeenCalledWith(false, 80);
    expect(h.back).not.toHaveBeenCalled();
    expect(s.state).toBe("idle");
  });

  it("when Back has nothing to pop, iOS pulls the drawer instead (single owner of the edge)", () => {
    const h = host({ backAvailable: () => false });
    const s = createEdgeSwipe(h);
    s.start(5, 100, 0);
    expect(s.kind).toBe("drawer");
    s.move(5 + DECIDE_PX, 100, 10);
    expect(h.progress).not.toHaveBeenCalled();
  });

  it("with Back available the edge is Back even when the drawer could open", () => {
    const s = createEdgeSwipe(host());
    s.start(5, 100, 0);
    expect(s.kind).toBe("back");
  });
});

describe("left drawer pull (OG container.cljs: opens past 40px; Tine adds the vertical check)", () => {
  const android = () => host({ platform: "android" });

  it("does not open just under the OG distance, opens just over", () => {
    const under = android();
    drag(under, 5, 100, [[5 + DRAWER_OPEN_PX, 100]]);
    expect(under.openDrawer).not.toHaveBeenCalled();
    const over = android();
    drag(over, 5, 100, [[5 + DRAWER_OPEN_PX + 1, 100]]);
    expect(over.openDrawer).toHaveBeenCalledOnce();
  });

  it("Android never runs Back from the edge (the OS owns it)", () => {
    const h = android();
    const s = createEdgeSwipe(h);
    s.start(5, 100, 0);
    expect(s.kind).toBe("drawer");
    s.move(5 + 200, 100, 10);
    expect(h.back).not.toHaveBeenCalled();
  });

  it("opens exactly once per gesture", () => {
    const h = android();
    const s = createEdgeSwipe(h);
    s.start(5, 100, 0);
    s.move(5 + 60, 100, 10);
    s.move(5 + 90, 100, 20);
    s.end(5 + 90, 100, 30);
    expect(h.openDrawer).toHaveBeenCalledOnce();
  });

  it("does not open from a diagonal scroll", () => {
    const h = android();
    drag(h, 5, 100, [[5 + 12, 100 + 40], [5 + 90, 100 + 90]]);
    expect(h.openDrawer).not.toHaveBeenCalled();
  });

  it("does nothing when the drawer cannot open (already open / wide layout)", () => {
    const h = android();
    h.drawerOpenable = () => false;
    expect(createEdgeSwipe(h).start(5, 100, 0)).toBe(false);
  });
});
