import { describe, expect, it, vi } from "vitest";
import {
  ACTIONS_PX,
  AXIS_LOCK_PX,
  EDITING_WINDOW_MS,
  INDENT_PX,
  OUTDENT_PX,
  RELEASE_MIN_PX,
  REVERSAL_PX,
  SWIPE_RECOGNIZE_PX,
  VERTICAL_LIMIT_PX,
  actionFor,
  createBlockSwipe,
  type BlockSwipeAction,
} from "./blockSwipe";

function harness(over: { editing?: boolean; range?: boolean } = {}) {
  const commits: Array<{ action: BlockSwipeAction; x: number; y: number }> = [];
  const progress = vi.fn();
  const host = {
    editing: () => !!over.editing,
    rangeSelected: () => !!over.range,
    progress,
    commit: (action: BlockSwipeAction, x: number, y: number) => commits.push({ action, x, y }),
  };
  const swipe = createBlockSwipe(host);
  return { swipe, commits, progress };
}

/** One horizontal swipe from (100,200) by `dx`, ending there; stepped so the axis locks. */
function swipeBy(h: ReturnType<typeof harness>, dx: number, dy = 0, t = 100) {
  h.swipe.start(100, 200, 0, 1, false);
  const steps = 8;
  for (let i = 1; i <= steps; i++) h.swipe.move(100 + (dx * i) / steps, 200 + (dy * i) / steps, (t * i) / steps, 1);
  h.swipe.end(100 + dx, 200 + dy, t);
}

describe("block swipe thresholds (OG block.cljs on-touch-move/end)", () => {
  it("pins the OG constants", () => {
    expect([SWIPE_RECOGNIZE_PX, VERTICAL_LIMIT_PX, INDENT_PX, OUTDENT_PX, ACTIONS_PX, RELEASE_MIN_PX, EDITING_WINDOW_MS]).toEqual([30, 30, 40, 40, 80, 10, 600]);
  });

  it("indent: just under 40px right does nothing, 40px right indents", () => {
    const a = harness(); swipeBy(a, INDENT_PX - 1); expect(a.commits).toEqual([]);
    const b = harness(); swipeBy(b, INDENT_PX); expect(b.commits.map((c) => c.action)).toEqual(["indent"]);
    const c = harness(); swipeBy(c, 200); expect(c.commits.map((x) => x.action)).toEqual(["indent"]);
  });

  it("outdent: just under 40px left nothing, 40..79 outdent, 80+ the action menu", () => {
    const a = harness(); swipeBy(a, -(OUTDENT_PX - 1)); expect(a.commits).toEqual([]);
    const b = harness(); swipeBy(b, -OUTDENT_PX); expect(b.commits.map((c) => c.action)).toEqual(["outdent"]);
    const c = harness(); swipeBy(c, -(ACTIONS_PX - 1)); expect(c.commits.map((x) => x.action)).toEqual(["outdent"]);
    const d = harness(); swipeBy(d, -ACTIONS_PX); expect(d.commits.map((x) => x.action)).toEqual(["actions"]);
  });

  it("actionFor is the single decision table", () => {
    expect(actionFor(1, 39)).toBeNull();
    expect(actionFor(1, 40)).toBe("indent");
    expect(actionFor(-1, -39)).toBeNull();
    expect(actionFor(-1, -40)).toBe("outdent");
    expect(actionFor(-1, -80)).toBe("actions");
    // A right-swipe never yields a left action and vice versa.
    expect(actionFor(1, -100)).toBeNull();
    expect(actionFor(-1, 100)).toBeNull();
  });

  it("the commit carries the release point (the action menu anchors there)", () => {
    const h = harness(); swipeBy(h, -90); expect(h.commits[0]).toMatchObject({ x: 10, y: 200 });
  });

  it("recognition: feedback starts past 30px, not at 30px", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    h.swipe.move(100 + AXIS_LOCK_PX + 2, 200, 10, 1);
    h.swipe.move(100 + SWIPE_RECOGNIZE_PX, 200, 20, 1);
    expect(h.progress).toHaveBeenLastCalledWith(expect.objectContaining({ recognized: false }));
    h.swipe.move(100 + SWIPE_RECOGNIZE_PX + 1, 200, 30, 1);
    expect(h.progress).toHaveBeenLastCalledWith(expect.objectContaining({ recognized: true, action: null }));
    h.swipe.move(100 + INDENT_PX, 200, 40, 1);
    expect(h.progress).toHaveBeenLastCalledWith(expect.objectContaining({ recognized: true, action: "indent" }));
  });

  it("the release gate: a swipe that ends within 10px of its origin does nothing", () => {
    // Walk out to 60px then come back to the start: net travel 0 <= 10.
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    for (const x of [110, 130, 160]) h.swipe.move(x, 200, 10, 1);
    for (const x of [140, 120, 105]) h.swipe.move(x, 200, 20, 1);
    h.swipe.end(105, 200, 30);
    expect(h.commits).toEqual([]);
    expect(RELEASE_MIN_PX).toBe(10);
  });
});

describe("axis lock and vertical tolerance (Tine addition over OG)", () => {
  it("a vertical-dominant start is a scroll for the whole touch, even if it later drifts sideways 200px", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    h.swipe.move(102, 200 + AXIS_LOCK_PX + 2, 10, 1);
    expect(h.swipe.state()).toBe("ignored");
    // The finger then wanders far right at nearly the same y: still a scroll.
    for (let i = 1; i <= 10; i++) expect(h.swipe.move(102 + i * 20, 200 + AXIS_LOCK_PX + 2, 20 + i, 1)).toBe(false);
    h.swipe.end(300, 208, 40);
    expect(h.commits).toEqual([]);
  });

  it("movement under the axis-lock distance decides nothing (a tap)", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    expect(h.swipe.move(100 + AXIS_LOCK_PX - 1, 200, 10, 1)).toBe(false);
    expect(h.swipe.state()).toBe("undecided");
    h.swipe.end(100 + AXIS_LOCK_PX - 1, 200, 20);
    expect(h.commits).toEqual([]);
  });

  it("horizontal-dominant start: claims the touch (returns true so the caller preventDefaults)", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    expect(h.swipe.move(100 + AXIS_LOCK_PX + 1, 203, 10, 1)).toBe(true);
    expect(h.swipe.state()).toBe("tracking");
  });

  it("OG's vertical bound: 29px of drift still indents, 30px abandons the swipe", () => {
    const a = harness(); swipeBy(a, 80, VERTICAL_LIMIT_PX - 1); expect(a.commits.map((c) => c.action)).toEqual(["indent"]);
    const b = harness(); swipeBy(b, 80, VERTICAL_LIMIT_PX); expect(b.commits).toEqual([]);
  });

  it("a mostly-vertical gesture with sideways drift (a scroll) never indents", () => {
    const h = harness(); swipeBy(h, 60, 120); expect(h.commits).toEqual([]);
  });
});

describe("origin re-base on reversal (OG: pulling back disarms)", () => {
  it("indent armed at 60px, pulled back 30px (not within jitter), then released: nothing", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    for (const x of [110, 130, 160]) h.swipe.move(x, 200, 10, 1);
    expect(h.progress).toHaveBeenLastCalledWith(expect.objectContaining({ action: "indent" }));
    h.swipe.move(130, 200, 20, 1);
    h.swipe.end(130, 200, 30);
    expect(h.commits).toEqual([]);
  });

  it("jitter of <= 4px at the apex keeps the swipe armed", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    for (const x of [110, 130, 160, 160 - REVERSAL_PX, 160]) h.swipe.move(x, 200, 10, 1);
    h.swipe.end(160 - REVERSAL_PX, 200, 30);
    expect(h.commits.map((c) => c.action)).toEqual(["indent"]);
  });

  it("5px of retreat re-bases: release there is no swipe", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    for (const x of [110, 130, 145 + 15, 160 - REVERSAL_PX - 1]) h.swipe.move(x, 200, 10, 1);
    h.swipe.end(160 - REVERSAL_PX - 1, 200, 30);
    expect(h.commits).toEqual([]);
  });

  it("long left swipe pulled back into outdent range after a re-base needs a fresh 40px", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    for (const x of [90, 60, 20]) h.swipe.move(x, 200, 10, 1); // -80
    h.swipe.move(50, 200, 20, 1); // retreat 30 -> re-base at 50
    h.swipe.end(50, 200, 30);
    expect(h.commits).toEqual([]);
  });
});

describe("disabled situations", () => {
  it("a second finger never starts a swipe and cancels a running one", () => {
    const a = harness();
    expect(a.swipe.start(100, 200, 0, 2, false)).toBe(false);
    const b = harness();
    b.swipe.start(100, 200, 0, 1, false);
    b.swipe.move(160, 200, 10, 1);
    expect(b.swipe.move(170, 200, 20, 2)).toBe(false);
    b.swipe.end(170, 200, 30);
    expect(b.commits).toEqual([]);
  });

  it("a disabled target (query, drawer, drawing, code, media) never starts one", () => {
    const h = harness();
    expect(h.swipe.start(100, 200, 0, 1, true)).toBe(false);
    h.swipe.move(200, 200, 10, 1); h.swipe.end(200, 200, 20);
    expect(h.commits).toEqual([]);
  });

  it("a range text selection never starts one and aborts a running one", () => {
    const a = harness({ range: true });
    expect(a.swipe.start(100, 200, 0, 1, false)).toBe(false);
    let range = false;
    const commits: string[] = [];
    const swipe = createBlockSwipe({ editing: () => false, rangeSelected: () => range, progress() {}, commit: (a) => commits.push(a) });
    swipe.start(100, 200, 0, 1, false);
    swipe.move(150, 200, 10, 1);
    range = true;
    swipe.move(200, 200, 20, 1);
    swipe.end(200, 200, 30);
    expect(commits).toEqual([]);
  });

  it("editing window: a swipe on a row being edited is honoured under 600ms, dropped from 600ms", () => {
    const a = harness({ editing: true });
    a.swipe.start(100, 200, 0, 1, false);
    a.swipe.move(160, 200, EDITING_WINDOW_MS - 1, 1);
    a.swipe.end(160, 200, EDITING_WINDOW_MS - 1);
    expect(a.commits.map((c) => c.action)).toEqual(["indent"]);
    const b = harness({ editing: true });
    b.swipe.start(100, 200, 0, 1, false);
    b.swipe.move(115, 200, 100, 1);
    b.swipe.move(160, 200, EDITING_WINDOW_MS, 1);
    b.swipe.end(160, 200, EDITING_WINDOW_MS);
    expect(b.commits).toEqual([]);
  });

  it("a resting (not editing) row has no time limit", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    h.swipe.move(115, 200, 100, 1);
    h.swipe.move(160, 200, 5000, 1);
    h.swipe.end(160, 200, 5000);
    expect(h.commits.map((c) => c.action)).toEqual(["indent"]);
  });

  it("cancel (touchcancel) clears feedback and commits nothing", () => {
    const h = harness();
    h.swipe.start(100, 200, 0, 1, false);
    h.swipe.move(110, 200, 10, 1); h.swipe.move(170, 200, 20, 1);
    h.swipe.cancel();
    expect(h.progress).toHaveBeenLastCalledWith({ action: null, recognized: false, dx: 0 });
    expect(h.swipe.state()).toBe("idle");
    expect(h.commits).toEqual([]);
  });

  it("each touch is independent: a committed swipe does not leak into the next tap", () => {
    const h = harness(); swipeBy(h, 80);
    h.swipe.start(100, 200, 0, 1, false); h.swipe.end(100, 200, 10);
    expect(h.commits).toHaveLength(1);
  });
});
