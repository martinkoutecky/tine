import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore, selectedIds } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import { installBlockSelectionDrag } from "../blockDrag";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

// GH #731: dragging a selection or a block to the top/bottom edge of the
// scrolling pane must scroll it. jsdom has no layout, so the pane is a scroller
// with stubbed geometry: 30 rows of 40 px in a 400 px viewport (10 per screen),
// `elementFromPoint` answers from the live scrollTop, and animation frames are
// stepped by hand.

const ROW = 40;
const VIEW = 400;
const COUNT = 30;
const ids = Array.from({ length: COUNT }, (_, i) => `b${i}`);

beforeAll(async () => {
  await initParser();
});

let scrollerEl: HTMLDivElement;
let scrollTop = 0;
let queue: Map<number, FrameRequestCallback>;
let uninstall: () => void;
let dispose: () => void;

const block = (id: string): BlockDto => ({ id, raw: `row ${id}`, collapsed: false, children: [] });
const page = (): PageDto => ({ name: "Long", kind: "page", title: "Long", pre_block: null, blocks: ids.map(block) });

function frames(n: number) {
  let t = 5000;
  for (let i = 0; i < n; i++) {
    t += 16;
    const due = [...queue.values()];
    queue.clear();
    for (const cb of due) cb(t);
  }
}

const rowAt = (y: number) => ids[Math.floor((y + scrollTop) / ROW)];
const el = (id: string) => scrollerEl.querySelector<HTMLElement>(`[data-block-id="${id}"]`)!;

function mount() {
  loadSingle(page());
  scrollerEl = document.createElement("div");
  scrollerEl.className = "main-content";
  scrollerEl.style.overflowY = "auto";
  document.body.appendChild(scrollerEl);
  scrollTop = 0;
  Object.defineProperty(scrollerEl, "scrollTop", {
    configurable: true,
    get: () => scrollTop,
    set: (v: number) => {
      scrollTop = Math.max(0, Math.min(v, COUNT * ROW - VIEW));
    },
  });
  Object.defineProperty(scrollerEl, "scrollHeight", { configurable: true, value: COUNT * ROW });
  Object.defineProperty(scrollerEl, "clientHeight", { configurable: true, value: VIEW });
  scrollerEl.getBoundingClientRect = () =>
    ({ top: 0, bottom: VIEW, left: 0, right: 800, width: 800, height: VIEW, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
  dispose = render(() => <For each={pageByName("Long")?.roots ?? []}>{(id) => <Block id={id} />}</For>, scrollerEl);
  // Each row's box follows the scroll position.
  for (const id of ids) {
    const i = ids.indexOf(id);
    const rect = () => {
      const top = i * ROW - scrollTop;
      return { top, bottom: top + ROW, left: 0, right: 800, width: 800, height: ROW, x: 0, y: top, toJSON: () => ({}) } as DOMRect;
    };
    el(id).getBoundingClientRect = rect;
    el(id).querySelector<HTMLElement>(".block-main")!.getBoundingClientRect = rect;
  }
  // Like the window's resize grips (.grip-s): the outermost 4 px hit chrome, not a block.
  const grip = document.createElement("div");
  document.body.appendChild(grip);
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: (_x: number, y: number) => (y >= VIEW - 4 ? grip : y >= 0 ? el(rowAt(y)) : document.body),
  });
}

beforeEach(() => {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 800 });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: VIEW });
  queue = new Map();
  let id = 0;
  window.requestAnimationFrame = (cb) => {
    queue.set(++id, cb);
    return id;
  };
  window.cancelAnimationFrame = (n) => {
    queue.delete(n);
  };
});

afterEach(() => {
  uninstall?.();
  dispose?.();
  vi.restoreAllMocks();
  resetStore();
  Reflect.deleteProperty(document, "elementFromPoint");
  document.documentElement.classList.remove("drag-selection-suppressed");
  document.body.innerHTML = "";
});

const move = (x: number, y: number) =>
  document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, cancelable: true, clientX: x, clientY: y, buttons: 1 }));
const up = () => document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));

describe("edge autoscroll while selecting or moving blocks (GH #731)", () => {
  it("drag-select from outside the rendered text scrolls the pane and extends the selection past the first screen", () => {
    uninstall = installBlockSelectionDrag();
    mount();
    el("b1").dispatchEvent(new MouseEvent("mousedown", { button: 0, buttons: 1, bubbles: true, clientX: 200, clientY: 60 }));
    move(200, 140); // crosses into b3: block selection starts
    expect(selectedIds()).toEqual(["b1", "b2", "b3"]);

    move(200, VIEW - 3); // the pointer rests in the bottom zone
    frames(60);
    expect(scrollTop).toBeGreaterThan(300);
    const reached = selectedIds().at(-1)!;
    expect(ids.indexOf(reached)).toBeGreaterThan(COUNT / 3); // well past the first screen (b0..b9)
    expect(selectedIds()[0]).toBe("b1");
    expect(selectedIds()).toEqual(ids.slice(1, ids.indexOf(reached) + 1));

    // Releasing ends the scrolling and keeps the selection.
    up();
    const stopped = scrollTop;
    frames(20);
    expect(scrollTop).toBe(stopped);
    expect(selectedIds().at(-1)).toBe(reached);
  });

  it("drag-select upwards scrolls back toward the start of the page", () => {
    uninstall = installBlockSelectionDrag();
    mount();
    scrollTop = 600;
    el("b20").dispatchEvent(new MouseEvent("mousedown", { button: 0, buttons: 1, bubbles: true, clientX: 200, clientY: 20 * ROW - 600 + 5 }));
    move(200, 20 * ROW - 600 - 30); // into b19
    expect(selectedIds()).toEqual(["b19", "b20"]);
    move(200, 2); // top zone
    frames(60);
    expect(scrollTop).toBeLessThan(300);
    expect(ids.indexOf(selectedIds()[0])).toBeLessThan(ids.indexOf("b20") - 8);
    expect(selectedIds().at(-1)).toBe("b20");
    up();
  });

  it("a pointer held beyond the bottom of the window keeps scrolling the pane under it", () => {
    uninstall = installBlockSelectionDrag();
    mount();
    el("b1").dispatchEvent(new MouseEvent("mousedown", { button: 0, buttons: 1, bubbles: true, clientX: 200, clientY: 60 }));
    move(200, 140);
    move(200, VIEW + 200);
    frames(40);
    expect(scrollTop).toBeGreaterThan(200);
    expect(ids.indexOf(selectedIds().at(-1)!)).toBeGreaterThan(14);
    up();
  });

  it("does not scroll while the pointer is away from the edges, nor after Escape", () => {
    uninstall = installBlockSelectionDrag();
    mount();
    el("b1").dispatchEvent(new MouseEvent("mousedown", { button: 0, buttons: 1, bubbles: true, clientX: 200, clientY: 60 }));
    move(200, 140);
    move(200, 220);
    frames(30);
    expect(scrollTop).toBe(0);
    move(200, VIEW - 3);
    frames(10);
    const moving = scrollTop;
    expect(moving).toBeGreaterThan(0);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    frames(10);
    expect(scrollTop).toBe(moving);
    up();
  });

  it("a press on rendered text that crosses blocks scrolls too (the edit gesture, not the document drag)", () => {
    mount(); // no installBlockSelectionDrag: the press itself owns the gesture
    const text = (id: string) => el(id).querySelector<HTMLElement>(".block-content")!;
    text("b1").dispatchEvent(new MouseEvent("mousedown", { button: 0, buttons: 1, bubbles: true, cancelable: true, clientX: 50, clientY: 60 }));
    move(50, 140);
    expect(selectedIds()).toEqual(["b1", "b2", "b3"]);
    move(50, VIEW - 3);
    frames(60);
    expect(scrollTop).toBeGreaterThan(300);
    expect(ids.indexOf(selectedIds().at(-1)!)).toBeGreaterThan(COUNT / 3);
    up();
    const stopped = scrollTop;
    frames(20);
    expect(scrollTop).toBe(stopped);
  });

  it("a jittery click near the bottom edge does not scroll the page", () => {
    mount();
    const text = el("b9").querySelector<HTMLElement>(".block-content")!;
    const y = 9 * ROW + 20; // inside the bottom zone
    text.dispatchEvent(new MouseEvent("mousedown", { button: 0, buttons: 1, bubbles: true, cancelable: true, clientX: 50, clientY: y }));
    move(58, y - 2);
    move(50, y + 3);
    frames(30);
    expect(scrollTop).toBe(0);
    up();
  });

  it("dragging a block by its bullet to the bottom edge scrolls and drops it beyond the first screen", async () => {
    mount();
    const bullet = el("b0").querySelector<HTMLElement>(".bullet-container")!;
    bullet.dispatchEvent(new MouseEvent("mousedown", { button: 0, buttons: 1, bubbles: true, clientX: 10, clientY: 20 }));
    move(30, 100); // past the 4 px threshold, mid-page
    frames(10);
    expect(scrollTop).toBe(0);
    move(30, VIEW - 3);
    frames(60);
    expect(scrollTop).toBeGreaterThan(300);
    up();
    const stopped = scrollTop;
    frames(20);
    expect(scrollTop).toBe(stopped);
    await vi.waitFor(() => expect(pageByName("Long")!.roots[0]).not.toBe("b0"));
    const order = pageByName("Long")!.roots;
    expect(order.indexOf("b0")).toBeGreaterThan(COUNT / 3);
    expect(order.length).toBe(COUNT);
  });

  it("releasing a bullet drag outside the window with nothing to scroll still cancels the move", async () => {
    mount();
    // A pane that cannot scroll: the pointer past the window has no scroller to push on.
    Object.defineProperty(scrollerEl, "scrollHeight", { configurable: true, value: VIEW });
    const bullet = el("b0").querySelector<HTMLElement>(".bullet-container")!;
    bullet.dispatchEvent(new MouseEvent("mousedown", { button: 0, buttons: 1, bubbles: true, clientX: 10, clientY: 20 }));
    move(30, 100);
    move(30, VIEW + 120);
    up();
    await new Promise((r) => setTimeout(r, 20));
    expect(pageByName("Long")!.roots).toEqual(ids);
  });
});
