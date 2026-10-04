// jsdom: the swipe on a real row element, wired to the real outline store
// (GH #501). Layout / OS scroll arbitration is NOT provable here - see the
// native E2E (scripts/e2e-touch-gestures.mjs) and the device-only list in the
// changelog receipt.
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { attachBlockSwipe, blockSwipeDisabledTarget, swipeDisabledByTags } from "./blockSwipe";
import { setGraphMeta } from "./graphSession";
import type { GraphMeta } from "./types";
import { wireBlockSwipe } from "./components/blockSwipeWiring";
import { OUTLINE_MAX_DEPTH } from "./editor/outline";
import { initParser } from "./render/parse";
import { clearSeededFacets } from "./render/facets";
import { doc } from "./document/model";
import { loadSingle } from "./document/workingSet";
import { resetStore, undo, selectedIds } from "./document";
import { contextMenu, setContextMenu } from "./ui";
import type { BlockDto } from "./types";

let counter = 0;
const blk = (raw: string, children: BlockDto[] = []): BlockDto => ({ id: `s${counter++}`, raw, collapsed: false, children });

function loadPage(blocks: BlockDto[]) {
  loadSingle({ name: "Swipe", kind: "page", title: "Swipe", pre_block: null, blocks });
  clearSeededFacets();
}
const shape = (ids: string[] = doc.pages[0].roots): any[] =>
  ids.map((id) => (doc.byId[id].children.length ? [doc.byId[id].raw, shape(doc.byId[id].children)] : [doc.byId[id].raw]));

function fire(el: EventTarget, type: string, x: number, y: number, ts: number, fingers = 1) {
  const t = { clientX: x, clientY: y, identifier: 1, target: el };
  const e = new Event(type, { bubbles: true, cancelable: true }) as any;
  Object.defineProperty(e, "touches", { value: type === "touchend" || type === "touchcancel" ? [] : Array(fingers).fill(t) });
  Object.defineProperty(e, "changedTouches", { value: [t] });
  Object.defineProperty(e, "timeStamp", { value: ts });
  el.dispatchEvent(e);
  return e as Event;
}
/** touchstart at (200,300), steps of dx/8 to (200+dx, 300+dy), touchend. Returns the moves' events. */
function swipe(el: HTMLElement, dx: number, dy = 0, target: EventTarget = el) {
  fire(target, "touchstart", 200, 300, 0);
  const moves: Event[] = [];
  for (let i = 1; i <= 8; i++) moves.push(fire(target, "touchmove", 200 + (dx * i) / 8, 300 + (dy * i) / 8, i * 12));
  fire(target, "touchend", 200 + dx, 300 + dy, 120);
  return moves;
}

beforeAll(() => initParser());
let row: HTMLElement;
let cleanups: Array<() => void> = [];
beforeEach(() => {
  counter = 0;
  resetStore();
  setContextMenu(null as never);
  (globalThis as any).__TINE_E2E_TOUCH_GESTURES__ = "android";
  row = document.createElement("div");
  row.className = "block-main";
  document.body.appendChild(row);
});
afterEach(() => {
  cleanups.forEach((c) => c());
  cleanups = [];
  document.body.innerHTML = "";
  delete (globalThis as any).__TINE_E2E_TOUCH_GESTURES__;
});
const wire = (id: string, over: Partial<Parameters<typeof wireBlockSwipe>[1]> = {}) => {
  const c = wireBlockSwipe(row, { id, scope: null, editing: () => false, readOnly: () => false, ...over });
  cleanups.push(c);
  return c;
};

describe("block swipe commands (real store)", () => {
  it("swipe right indents under the previous sibling, and one undo restores it", () => {
    loadPage([blk("a"), blk("b")]);
    const [, b] = doc.pages[0].roots;
    wire(b);
    swipe(row, 80);
    expect(shape()).toEqual([["a", [["b"]]]]);
    expect(selectedIds()).toEqual([]);
    expect(undo()).toBe(true);
    expect(shape()).toEqual([["a"], ["b"]]);
  });

  it("just under the indent threshold changes nothing", () => {
    loadPage([blk("a"), blk("b")]);
    wire(doc.pages[0].roots[1]);
    swipe(row, 39);
    expect(shape()).toEqual([["a"], ["b"]]);
  });

  it("short swipe left outdents; undo restores", () => {
    loadPage([blk("a", [blk("b")])]);
    wire(doc.byId[doc.pages[0].roots[0]].children[0]);
    swipe(row, -60);
    expect(shape()).toEqual([["a"], ["b"]]);
    expect(undo()).toBe(true);
    expect(shape()).toEqual([["a", [["b"]]]]);
  });

  it.each([-80, -100, -139])("a natural %ipx left swipe outdents instead of opening actions", (dx) => {
    loadPage([blk("a", [blk("b")])]);
    wire(doc.byId[doc.pages[0].roots[0]].children[0]);
    swipe(row, dx);
    expect(shape()).toEqual([["a"], ["b"]]);
    expect(contextMenu()).toBeNull();
    expect(undo()).toBe(true);
    expect(shape()).toEqual([["a", [["b"]]]]);
  });

  it.each(["android", "ios"])("%s reveals indent, outdent and more cues while the finger is down", (platform) => {
    (globalThis as any).__TINE_E2E_TOUCH_GESTURES__ = platform;
    loadPage([blk("a", [blk("b"), blk("c")])]);
    wire(doc.byId[doc.pages[0].roots[0]].children[1]);
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 260, 300, 20);
    expect(row.querySelector(".block-swipe-cue")?.getAttribute("data-action")).toBe("indent");
    fire(row, "touchcancel", 260, 300, 30);
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 100, 300, 20);
    expect(row.querySelector(".block-swipe-cue")?.getAttribute("data-action")).toBe("outdent");
    fire(row, "touchmove", 60, 300, 30);
    expect(row.querySelector(".block-swipe-cue")?.getAttribute("data-action")).toBe("actions");
    fire(row, "touchend", 60, 300, 40);
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
  });

  it("unavailable indent/outdent cues stay hidden, but the root still offers actions", () => {
    loadPage([blk("a")]);
    wire(doc.pages[0].roots[0]);
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 260, 300, 20);
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
    fire(row, "touchcancel", 260, 300, 30);
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 100, 300, 20);
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
    fire(row, "touchmove", 40, 300, 30);
    expect(row.querySelector(".block-swipe-cue")?.getAttribute("data-action")).toBe("actions");
  });

  it("a recognized but unarmed swipe has a dim cue; reversal and cleanup remove it", () => {
    loadPage([blk("a"), blk("b")]);
    const off = wire(doc.pages[0].roots[1]);
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 231, 300, 20);
    expect(row.querySelector(".block-swipe-cue")?.getAttribute("data-armed")).toBe("false");
    fire(row, "touchmove", 260, 300, 30);
    expect(row.querySelector(".block-swipe-cue")?.getAttribute("data-armed")).toBe("true");
    fire(row, "touchmove", 250, 300, 40);
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
    fire(row, "touchmove", 300, 300, 50);
    expect(row.querySelector(".block-swipe-cue")).not.toBeNull();
    off();
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
  });

  it("read-only and scoped-out moves have no misleading cues", () => {
    loadPage([blk("a", [blk("b"), blk("c")])]);
    const a = doc.pages[0].roots[0], c = doc.byId[a].children[1];
    let readOnly = true;
    wire(c, { readOnly: () => readOnly, scope: { roots: [a], forceExpandedRoot: a } });
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 260, 300, 20);
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
    fire(row, "touchcancel", 260, 300, 30);
    readOnly = false;
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 100, 300, 20);
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
    fire(row, "touchend", 100, 300, 30);
    expect(shape()).toEqual([["a", [["b"], ["c"]]]]);
  });

  it("an indent that would exceed the subtree depth cap has no cue", () => {
    let deep = blk("leaf");
    for (let i = 1; i < OUTLINE_MAX_DEPTH; i++) deep = blk("branch", [deep]);
    loadPage([blk("a"), deep]);
    wire(doc.pages[0].roots[1]);
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 260, 300, 20);
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
  });

  it("a result scope omitting the previous sibling hides indent", () => {
    loadPage([blk("a"), blk("b"), blk("c")]);
    const [a, , c] = doc.pages[0].roots;
    wire(c, { scope: { roots: [a, c], navOnly: true } });
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 260, 300, 20);
    expect(row.querySelector(".block-swipe-cue")).toBeNull();
    fire(row, "touchend", 260, 300, 30);
    expect(shape()).toEqual([["a"], ["b"], ["c"]]);
  });

  it("long swipe left selects the block and opens its action menu at the release point", () => {
    loadPage([blk("a"), blk("b")]);
    const [, b] = doc.pages[0].roots;
    wire(b);
    swipe(row, -160);
    expect(shape()).toEqual([["a"], ["b"]]);
    expect(contextMenu()).toMatchObject({ kind: "block", blockId: b, x: 40, y: 300 });
    expect(selectedIds()).toEqual([b]);
  });

  it("the first sibling cannot indent: no move, no leftover selection", () => {
    loadPage([blk("a"), blk("b")]);
    wire(doc.pages[0].roots[0]);
    swipe(row, 80);
    expect(shape()).toEqual([["a"], ["b"]]);
  });

  it("a read-only page ignores the swipe", () => {
    loadPage([blk("a"), blk("b")]);
    wire(doc.pages[0].roots[1], { readOnly: () => true });
    swipe(row, 80);
    expect(shape()).toEqual([["a"], ["b"]]);
  });

  it("desktop without the E2E override installs nothing", () => {
    delete (globalThis as any).__TINE_E2E_TOUCH_GESTURES__;
    loadPage([blk("a"), blk("b")]);
    wire(doc.pages[0].roots[1]);
    swipe(row, 80);
    expect(shape()).toEqual([["a"], ["b"]]);
  });
});

describe("block swipe and the browser", () => {
  it("claims the touch (preventDefault) once the axis is horizontal, never for a vertical scroll", () => {
    const h = attachBlockSwipe(row, { platform: "ios", editing: () => false, run() {} });
    cleanups.push(h);
    const horiz = swipe(row, 80);
    expect(horiz.slice(2).every((e) => e.defaultPrevented)).toBe(true);
    const vert = swipe(row, 10, 120);
    expect(vert.some((e) => e.defaultPrevented)).toBe(false);
  });

  it("a vertical scroll that wanders sideways never indents", () => {
    loadPage([blk("a"), blk("b")]);
    wire(doc.pages[0].roots[1]);
    swipe(row, 70, 150);
    expect(shape()).toEqual([["a"], ["b"]]);
  });

  it("swallows the click a committed swipe would otherwise produce, but not a later tap", () => {
    const seen: number[] = [];
    const btn = document.createElement("button");
    row.appendChild(btn);
    btn.addEventListener("click", () => seen.push(1));
    cleanups.push(attachBlockSwipe(row, { platform: "android", editing: () => false, run() {} }));
    swipe(row, 80, 0, btn);
    btn.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(seen).toEqual([]);
    // A tap after the swallow window is a normal click.
    const realNow = Date.now;
    Date.now = () => realNow() + 1000;
    try {
      btn.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    } finally { Date.now = realNow; }
    expect(seen).toEqual([1]);
  });

  it("the native context menu is refused while a swipe is tracking (it must not arm a long-press menu)", () => {
    cleanups.push(attachBlockSwipe(row, { platform: "android", editing: () => false, run() {} }));
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 220, 300, 10);
    const ctx = new Event("contextmenu", { bubbles: true, cancelable: true });
    row.dispatchEvent(ctx);
    expect(ctx.defaultPrevented).toBe(true);
    fire(row, "touchend", 220, 300, 20);
    const after = new Event("contextmenu", { bubbles: true, cancelable: true });
    row.dispatchEvent(after);
    expect(after.defaultPrevented).toBe(false);
  });

  it("progress feedback is set while tracking and removed on release", () => {
    cleanups.push(attachBlockSwipe(row, { platform: "android", editing: () => false, run() {} }));
    fire(row, "touchstart", 200, 300, 0);
    fire(row, "touchmove", 210, 300, 10);
    fire(row, "touchmove", 260, 300, 20);
    expect(row.getAttribute("data-swipe")).toBe("indent");
    fire(row, "touchend", 260, 300, 30);
    expect(row.hasAttribute("data-swipe")).toBe(false);
  });

  it("cleanup removes every listener", () => {
    const runs: string[] = [];
    const off = attachBlockSwipe(row, { platform: "android", editing: () => false, run: (a) => runs.push(a) });
    off();
    swipe(row, 80);
    expect(runs).toEqual([]);
  });

  it("a range text selection suppresses the swipe", () => {
    const runs: string[] = [];
    cleanups.push(attachBlockSwipe(row, { platform: "android", editing: () => false, run: (a) => runs.push(a) }));
    const p = document.createElement("p"); p.textContent = "some text"; row.appendChild(p);
    const r = document.createRange(); r.selectNodeContents(p);
    const s = document.getSelection()!; s.removeAllRanges(); s.addRange(r);
    swipe(row, 80);
    expect(runs).toEqual([]);
    s.removeAllRanges();
  });
});

describe("disabled zones (OG target-disable-swipe? + Tine equivalents)", () => {
  const zones = [
    ".query-block", ".query-result-sections", ".block-properties", ".drawio", ".draw-wrap",
    "pre.code-block", ".audio-panel", "video", "iframe", "textarea", ".md-table-wrap", ".sheet-scroll",
  ];
  for (const sel of zones) {
    it(`a touch inside ${sel} never starts a swipe`, () => {
      const make = (s: string) => {
        const m = s.match(/^([a-z]+)?\.?([\w-]+)?$/)!;
        const tag = s.startsWith(".") ? "div" : s.split(".")[0];
        const el = document.createElement(tag);
        if (s.includes(".")) el.className = s.slice(s.indexOf(".") + 1);
        void m;
        return el;
      };
      const zone = make(sel);
      const inner = document.createElement("span");
      zone.appendChild(inner);
      row.appendChild(zone);
      expect(blockSwipeDisabledTarget(inner, row)).toBe(true);
      const runs: string[] = [];
      cleanups.push(attachBlockSwipe(row, { platform: "android", editing: () => false, run: (a) => runs.push(a) }));
      swipe(row, 80, 0, inner);
      expect(runs).toEqual([]);
    });
  }

  it("plain text, the bullet column and empty row space are not disabled", () => {
    const text = document.createElement("span");
    const bullet = document.createElement("div"); bullet.className = "bullet-container";
    row.append(text, bullet);
    expect(blockSwipeDisabledTarget(text, row)).toBe(false);
    expect(blockSwipeDisabledTarget(bullet, row)).toBe(false);
    expect(blockSwipeDisabledTarget(row, row)).toBe(false);
  });

  it("a nested block row (embed / ref) is not this row's swipe", () => {
    const inner = document.createElement("div"); inner.className = "block-main";
    const leaf = document.createElement("span"); inner.appendChild(leaf);
    row.appendChild(inner);
    expect(blockSwipeDisabledTarget(leaf, row)).toBe(true);
  });

  it("an element that scrolls sideways owns its pan", () => {
    const wide = document.createElement("div");
    wide.style.overflowX = "auto";
    Object.defineProperty(wide, "scrollWidth", { value: 600 });
    Object.defineProperty(wide, "clientWidth", { value: 200 });
    const leaf = document.createElement("span"); wide.appendChild(leaf);
    row.appendChild(wide);
    expect(blockSwipeDisabledTarget(leaf, row)).toBe(true);
  });
});

// OG `:mobile {:gestures/disabled-in-block-with-tags [...]}` (`target-disable-swipe?`).
describe("user tag opt-out (swipeDisabledByTags)", () => {
  /** `.ls-block[data-block-id=outer] > .block-children-container > .ls-block[inner] > row` */
  function nest(outer: string, inner: string): HTMLElement {
    const a = document.createElement("div"); a.className = "ls-block"; a.setAttribute("data-block-id", outer);
    const kids = document.createElement("div"); kids.className = "block-children-container";
    const b = document.createElement("div"); b.className = "ls-block"; b.setAttribute("data-block-id", inner);
    const r = document.createElement("div"); r.className = "block-main";
    b.appendChild(r); kids.appendChild(b); a.appendChild(kids); document.body.appendChild(a);
    return r;
  }
  const refs = (table: Record<string, string[]>) => (id: string) => table[id] ?? [];

  it("a block carrying a listed tag disables the swipe; one without it does not", () => {
    const r = nest("o", "i");
    expect(swipeDisabledByTags(r, ["kanban"], refs({ i: ["kanban"] }))).toBe(true);
    expect(swipeDisabledByTags(r, ["kanban"], refs({ i: ["other"] }))).toBe(false);
  });

  it("a descendant of a tagged block is disabled too (the DOM nests children, as in OG)", () => {
    const r = nest("o", "i");
    expect(swipeDisabledByTags(r, ["kanban"], refs({ o: ["kanban"], i: [] }))).toBe(true);
  });

  it("matches as OG does: a substring of the refs string (kan matches kanban)", () => {
    const r = nest("o", "i");
    expect(swipeDisabledByTags(r, ["kan"], refs({ i: ["kanban"] }))).toBe(true);
    expect(swipeDisabledByTags(r, ["kanban"], refs({ i: ["kan"] }))).toBe(false);
  });

  it("an empty entry is ignored, and with nothing listed no block is even looked up", () => {
    const r = nest("o", "i");
    let asked = 0;
    const counting = (id: string) => { asked++; return id === "i" ? ["x"] : []; };
    expect(swipeDisabledByTags(r, [], counting)).toBe(false);
    expect(swipeDisabledByTags(r, [""], counting)).toBe(false);
    expect(asked).toBe(0);
    expect(swipeDisabledByTags(r, ["", "x"], counting)).toBe(true);
  });

  it("a row outside any block is not disabled", () => {
    expect(swipeDisabledByTags(row, ["kanban"], () => ["kanban"])).toBe(false);
  });
});

describe("user tag opt-out wired to the real store and graph meta", () => {
  afterEach(() => setGraphMeta(null));
  const meta = (tags: string[]) => ({ mobile_gestures_disabled_in_block_with_tags: tags }) as unknown as GraphMeta;
  function inBlock(id: string) {
    const b = document.createElement("div"); b.className = "ls-block"; b.setAttribute("data-block-id", id);
    document.body.appendChild(b); b.appendChild(row);
  }

  it("a swipe on a block tagged with a listed page does nothing; an untagged one still indents", () => {
    loadPage([blk("a"), blk("b #kanban"), blk("c")]);
    const [, b, c] = doc.pages[0].roots;
    setGraphMeta(meta(["Kanban"]));
    inBlock(b);
    wire(b);
    swipe(row, 80);
    expect(shape()).toEqual([["a"], ["b #kanban"], ["c"]]);
    // the next block carries no listed tag: unaffected
    const row2 = document.createElement("div"); row2.className = "block-main";
    const b2 = document.createElement("div"); b2.className = "ls-block"; b2.setAttribute("data-block-id", c);
    b2.appendChild(row2); document.body.appendChild(b2);
    cleanups.push(wireBlockSwipe(row2, { id: c, scope: null, editing: () => false, readOnly: () => false }));
    swipe(row2, 80, 0, row2);
    expect(shape()).toEqual([["a"], ["b #kanban", [["c"]]]]);
  });

  it("the same swipe works when nothing is listed", () => {
    loadPage([blk("a"), blk("b #kanban")]);
    const [, b] = doc.pages[0].roots;
    setGraphMeta(meta([]));
    inBlock(b);
    wire(b);
    swipe(row, 80);
    expect(shape()).toEqual([["a", [["b #kanban"]]]]);
  });
});
