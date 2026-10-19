import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { node as docNode, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import { Block } from "./Block";
import { installTouchMedia, setTouchInput } from "../tests/touchMediaTestkit";

// Review r1 of D-20261018-W1: the block swipe was attached once at mount, so a
// touch laptop or convertible that gained or lost its touch input kept the old
// wiring until the row remounted. The swipe now follows the capability.
beforeAll(() => { initParser(); installTouchMedia(); });
beforeEach(() => setTouchInput(false));
afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
  setTouchInput(false);
});

function fire(el: EventTarget, type: string, x: number, y: number, ts: number) {
  const t = { clientX: x, clientY: y, identifier: 1, target: el };
  const e = new Event(type, { bubbles: true, cancelable: true }) as any;
  Object.defineProperty(e, "touches", { value: type === "touchend" || type === "touchcancel" ? [] : [t] });
  Object.defineProperty(e, "changedTouches", { value: [t] });
  Object.defineProperty(e, "timeStamp", { value: ts });
  el.dispatchEvent(e);
}
function swipeRight(el: Element) {
  fire(el, "touchstart", 200, 300, 0);
  for (let i = 1; i <= 8; i++) fire(el, "touchmove", 200 + (80 * i) / 8, 300, i * 12);
  fire(el, "touchend", 280, 300, 120);
}

function mountPage() {
  loadSingle({
    name: "Swipe", kind: "page", title: "Swipe", format: "md", pre_block: null,
    blocks: [
      { id: "first", raw: "first block", collapsed: false, children: [] },
      { id: "second", raw: "second block", collapsed: false, children: [] },
    ],
  });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <For each={pageByName("Swipe")?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
  const main = root.querySelector('[data-block-id="second"] > .block-main') as HTMLElement;
  return { root, main, dispose };
}

const indented = () => docNode("second").parent === "first";
const SWIPE_EVENTS = ["touchstart", "touchmove", "touchend", "touchcancel", "contextmenu", "click"];

describe("block swipe follows the input capability (D-20261018-W1 review r1)", () => {
  it("starts working when a touch input appears after the row mounted", () => {
    const page = mountPage();
    try {
      swipeRight(page.main);
      expect(indented()).toBe(false);
      setTouchInput(false, true);
      swipeRight(page.main);
      expect(indented()).toBe(true);
    } finally { page.dispose(); }
  });

  it("stops working when the touch input goes away", () => {
    setTouchInput(false, true);
    const page = mountPage();
    try {
      setTouchInput(false);
      swipeRight(page.main);
      expect(indented()).toBe(false);
    } finally { page.dispose(); }
  });

  it("balances its listeners across flips and unmount", () => {
    setTouchInput(false, true);
    const page = mountPage();
    const add = vi.spyOn(page.main, "addEventListener");
    const remove = vi.spyOn(page.main, "removeEventListener");
    try {
      for (let i = 0; i < 3; i++) {
        setTouchInput(false);
        setTouchInput(false, true);
      }
      page.dispose();
      const count = (spy: typeof add, type: string) => spy.mock.calls.filter(([t]) => t === type).length;
      for (const type of SWIPE_EVENTS) {
        // The spies start after the first attach: three re-attaches, and three
        // detaches for the flips plus one for the unmount.
        expect(count(add, type), `added ${type}`).toBe(3);
        expect(count(remove, type), `removed ${type}`).toBe(4);
      }
      const removed = new Set(remove.mock.calls.map(([, fn]) => fn));
      for (const [, fn] of add.mock.calls) expect(removed.has(fn)).toBe(true);
    } finally { add.mockRestore(); remove.mockRestore(); }
  });
});
