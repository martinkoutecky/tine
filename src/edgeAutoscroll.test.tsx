import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  EDGE_ZONE_PX,
  HIT_INSET_PX,
  MAX_SPEED_PX_PER_S,
  MIN_SPEED_PX_PER_S,
  PRESS_ZONE_PX,
  scrollHome,
  startEdgeAutoscroll,
  viewportPoint,
} from "./edgeAutoscroll";

// jsdom has no layout: a scroller is a div whose geometry and scrollTop are
// stubbed, and `elementFromPoint` answers from that geometry. Frames are driven
// by hand so the speed is a number, not a timing.
const VIEW_W = 800;
const VIEW_H = 400;

interface Scroller {
  el: HTMLDivElement;
  top: () => number;
}

function scroller(opts: { parent?: HTMLElement; box?: { top: number; bottom: number }; content: number; at?: number }): Scroller {
  const el = document.createElement("div");
  el.style.overflowY = "auto";
  const box = opts.box ?? { top: 0, bottom: VIEW_H };
  let top = opts.at ?? 0;
  Object.defineProperty(el, "scrollTop", {
    configurable: true,
    get: () => top,
    set: (v: number) => {
      top = Math.max(0, Math.min(v, opts.content - (box.bottom - box.top)));
    },
  });
  Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => opts.content });
  Object.defineProperty(el, "clientHeight", { configurable: true, get: () => box.bottom - box.top });
  el.getBoundingClientRect = () =>
    ({ top: box.top, bottom: box.bottom, left: 0, right: VIEW_W, width: VIEW_W, height: box.bottom - box.top, x: 0, y: box.top, toJSON: () => ({}) }) as DOMRect;
  (opts.parent ?? document.body).appendChild(el);
  return { el, top: () => top };
}

function frames() {
  const queue = new Map<number, FrameRequestCallback>();
  let id = 0;
  let now = 1000;
  window.requestAnimationFrame = (cb) => {
    queue.set(++id, cb);
    return id;
  };
  window.cancelAnimationFrame = (n) => {
    queue.delete(n);
  };
  return {
    /** Run one frame `ms` after the previous one. */
    step(ms = 16) {
      now += ms;
      const due = [...queue.values()];
      queue.clear();
      for (const cb of due) cb(now);
    },
    run(n: number, ms = 16) {
      for (let i = 0; i < n; i++) this.step(ms);
    },
    pending: () => queue.size,
  };
}

let f: ReturnType<typeof frames>;
let under: Element | null;

beforeEach(() => {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: VIEW_W });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: VIEW_H });
  f = frames();
  under = null;
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => under });
});

afterEach(() => {
  Reflect.deleteProperty(document, "elementFromPoint");
  document.body.innerHTML = "";
});

describe("edge autoscroll (GH #731)", () => {
  it("scrolls down while the pointer is in the bottom zone, faster the deeper it is", () => {
    const s = scroller({ content: 4000 });
    under = s.el;
    const seen: number[] = [];
    const a = startEdgeAutoscroll(document, () => seen.push(s.top()));
    a.update(100, VIEW_H - 1); // deepest point in the window
    f.run(60); // 60 frames x 16 ms
    const full = s.top();
    expect(full).toBeGreaterThan(0.9 * MAX_SPEED_PX_PER_S * 0.96 * 0.99);
    expect(full).toBeLessThan(MAX_SPEED_PX_PER_S * 0.96 * 1.01 + 1);
    a.stop();

    const s2 = scroller({ content: 4000 });
    under = s2.el;
    const b = startEdgeAutoscroll(document, () => {});
    b.update(100, VIEW_H - EDGE_ZONE_PX + 4); // just inside the zone
    f.run(60);
    expect(s2.top()).toBeGreaterThan(0);
    expect(s2.top()).toBeLessThan(full / 3);
    expect(s2.top()).toBeGreaterThanOrEqual(MIN_SPEED_PX_PER_S * 0.96 * 0.9);
    b.stop();
    expect(seen.length).toBeGreaterThan(0);
  });

  it("scrolls up in the top zone and not at all in the middle", () => {
    const s = scroller({ content: 4000, at: 1000 });
    under = s.el;
    const a = startEdgeAutoscroll(document, () => {});
    a.update(100, VIEW_H / 2);
    f.run(30);
    expect(s.top()).toBe(1000);
    a.update(100, 2);
    f.run(30);
    expect(s.top()).toBeLessThan(1000);
    const after = s.top();
    a.update(100, VIEW_H / 2); // left the zone
    f.run(30);
    expect(s.top()).toBe(after);
    a.stop();
  });

  it("keeps scrolling with the pointer beyond the window (button held outside)", () => {
    const s = scroller({ content: 4000 });
    under = s.el;
    const a = startEdgeAutoscroll(document, () => {});
    const hit = a.update(100, VIEW_H + 300);
    expect(hit.edge).toBe(true);
    expect(hit.y).toBeLessThan(VIEW_H); // hit test lands on the scroller's last row
    f.run(10);
    expect(s.top()).toBeGreaterThan(0);
    a.stop();
  });

  it("tells the gesture where to hit-test after the content moved under a still pointer", () => {
    const s = scroller({ content: 4000 });
    under = s.el;
    const points: Array<[number, number]> = [];
    const a = startEdgeAutoscroll(document, (x, y) => points.push([x, y]));
    a.update(120, VIEW_H + 50);
    f.run(5);
    expect(points.length).toBeGreaterThan(0);
    for (const [x, y] of points) {
      expect(x).toBe(120);
      expect(y).toBe(VIEW_H - HIT_INSET_PX); // clear of the resize grips along the edge
    }
    a.stop();
  });

  it("stops on stop(), mouseup, pointercancel, Escape, blur and a move with no button", () => {
    const ends: Array<[string, () => void]> = [
      ["stop", () => {}],
      ["mouseup", () => document.dispatchEvent(new MouseEvent("mouseup"))],
      ["pointercancel", () => document.dispatchEvent(new Event("pointercancel"))],
      ["escape", () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }))],
      ["blur", () => window.dispatchEvent(new Event("blur"))],
    ];
    for (const [name, end] of ends) {
      const s = scroller({ content: 4000 });
      under = s.el;
      const a = startEdgeAutoscroll(document, () => {});
      a.update(100, VIEW_H - 2);
      f.run(5);
      const moving = s.top();
      expect(moving, name).toBeGreaterThan(0);
      if (name === "stop") a.stop();
      else end();
      f.run(10);
      expect(s.top(), name).toBe(moving);
      expect(f.pending(), name).toBe(0);
      // A later move of the finished gesture neither scrolls nor re-arms.
      a.update(100, VIEW_H - 2);
      f.run(10);
      expect(s.top(), name).toBe(moving);
      s.el.remove();
    }
    const s = scroller({ content: 4000 });
    under = s.el;
    const a = startEdgeAutoscroll(document, () => {});
    a.update(100, VIEW_H - 2, 1);
    f.run(3);
    const moving = s.top();
    a.update(100, VIEW_H - 2, 0);
    f.run(10);
    expect(s.top()).toBe(moving);
  });

  it("is one loop for several gestures: speed does not add up, each hears the scroll", () => {
    const one = scroller({ content: 4000 });
    under = one.el;
    const a = startEdgeAutoscroll(document, () => {});
    a.update(100, VIEW_H - 1);
    f.run(30);
    const single = one.top();
    a.stop();
    one.el.remove();

    const two = scroller({ content: 4000 });
    under = two.el;
    let heardA = 0;
    let heardB = 0;
    const b = startEdgeAutoscroll(document, () => heardA++);
    const c = startEdgeAutoscroll(document, () => heardB++);
    b.update(100, VIEW_H - 1);
    c.update(100, VIEW_H - 1);
    f.run(30);
    expect(two.top()).toBe(single);
    expect(heardA).toBeGreaterThan(0);
    expect(heardB).toBe(heardA);
    b.stop();
    f.run(3);
    const afterOne = heardB;
    expect(two.top()).toBeGreaterThan(single); // c still pushes
    c.stop();
    f.run(5);
    expect(heardB).toBeGreaterThanOrEqual(afterOne);
    const done = two.top();
    f.run(5);
    expect(two.top()).toBe(done);
  });

  it("a press that may still be a click only scrolls when pushed against the edge", () => {
    const s = scroller({ content: 4000 });
    under = s.el;
    const a = startEdgeAutoscroll(document, () => {});
    a.update(100, VIEW_H - PRESS_ZONE_PX - 6, undefined, PRESS_ZONE_PX);
    f.run(20);
    expect(s.top()).toBe(0);
    a.update(100, VIEW_H - 3, undefined, PRESS_ZONE_PX);
    f.run(20);
    expect(s.top()).toBeGreaterThan(0);
    a.stop();
  });

  it("scrolls the scroller that contains the pointer: a split pane, not the one beside it", () => {
    const left = scroller({ content: 4000, box: { top: 0, bottom: VIEW_H } });
    const right = scroller({ content: 4000, box: { top: 0, bottom: VIEW_H } });
    under = right.el;
    const a = startEdgeAutoscroll(document, () => {});
    a.update(700, VIEW_H - 2);
    f.run(10);
    expect(right.top()).toBeGreaterThan(0);
    expect(left.top()).toBe(0);
    a.stop();
  });

  it("uses a scroller whose edge is mid-window (stacked panes), not the window edge", () => {
    const upper = scroller({ content: 4000, box: { top: 0, bottom: 200 } });
    const lower = scroller({ content: 4000, box: { top: 200, bottom: 400 } });
    under = upper.el;
    const a = startEdgeAutoscroll(document, () => {});
    a.update(100, 198);
    f.run(10);
    expect(upper.top()).toBeGreaterThan(0);
    expect(lower.top()).toBe(0);
    a.stop();
  });

  it("lets an inner scroller that is out of room hand over to the outer one", () => {
    const outer = scroller({ content: 4000 });
    const inner = scroller({ parent: outer.el, content: 500, box: { top: 300, bottom: 400 }, at: 400 }); // already at its end
    under = inner.el;
    const a = startEdgeAutoscroll(document, () => {});
    a.update(100, VIEW_H - 2);
    f.run(10);
    expect(inner.top()).toBe(400);
    expect(outer.top()).toBeGreaterThan(0);
    a.stop();
  });

  it("prefers an inner scroller that can still move", () => {
    const outer = scroller({ content: 4000 });
    const inner = scroller({ parent: outer.el, content: 900, box: { top: 300, bottom: 400 } });
    under = inner.el;
    const a = startEdgeAutoscroll(document, () => {});
    a.update(100, VIEW_H - 2);
    f.run(10);
    expect(inner.top()).toBeGreaterThan(0);
    expect(outer.top()).toBe(0);
    a.stop();
  });

  it("over window chrome outside any scroller (resize grip, bar above), the pane the drag started in still scrolls", () => {
    const outer = scroller({ content: 4000, box: { top: 48, bottom: VIEW_H } });
    const row = document.createElement("div");
    outer.el.appendChild(row);
    const home = scrollHome(row);
    expect(home).toBe(outer.el);
    const grip = document.createElement("div"); // like .grip-s along the window's bottom edge
    document.body.appendChild(grip);
    under = grip;
    const a = startEdgeAutoscroll(document, () => {}, home);
    const hit = a.update(100, VIEW_H - 2);
    expect(hit.edge).toBe(true);
    f.run(10);
    expect(outer.top()).toBeGreaterThan(0);
    // Above the pane (title bar): scrolls up once it is below the top.
    const before = outer.top();
    a.update(100, 20);
    f.run(10);
    expect(outer.top()).toBeLessThan(before);
    // Beside the pane (outside its width) the pane does not claim the pointer.
    outer.el.getBoundingClientRect = () => ({ top: 48, bottom: VIEW_H, left: 0, right: 300, width: 300, height: VIEW_H - 48, x: 0, y: 48, toJSON: () => ({}) }) as DOMRect;
    const stay = outer.top();
    a.update(700, VIEW_H - 2);
    f.run(10);
    expect(outer.top()).toBe(stay);
    a.stop();
  });

  it("does nothing where nothing scrolls, and clamps the hit point into the viewport", () => {
    under = document.body;
    const a = startEdgeAutoscroll(document, () => {});
    const hit = a.update(-20, VIEW_H + 20);
    expect(hit.edge).toBe(false);
    expect(hit.x).toBe(0);
    expect(hit.y).toBe(VIEW_H - 1);
    expect(f.pending()).toBe(0);
    a.stop();
    expect(viewportPoint(document, 5000, -5)).toEqual({ x: VIEW_W - 1, y: 0 });
  });

  it("reports the edge row even when the scroller cannot move further", () => {
    const s = scroller({ content: 4000, at: 3600 }); // at the end
    under = s.el;
    const a = startEdgeAutoscroll(document, () => {});
    const hit = a.update(100, VIEW_H + 40);
    expect(hit.edge).toBe(true);
    f.run(10);
    expect(s.top()).toBe(3600);
    a.stop();
  });
});
