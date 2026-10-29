import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { endEdit, startEditing } from "../editorController";
import { node, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";

let dispose: (() => void) | undefined;
const frames = new Map<number, FrameRequestCallback>();
let frame = 0;
beforeAll(() => initParser());
beforeEach(() => {
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(cb => { frames.set(++frame, cb); return frame; });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => { frames.delete(id); });
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  endEdit("page-navigation"); resetStore(); document.body.replaceChildren();
  frames.clear(); vi.restoreAllMocks();
});
function step(n = 1) {
  for (let i = 0; i < n; i++) {
    const pending = [...frames.values()]; frames.clear();
    pending.forEach(cb => cb(performance.now()));
  }
}
function mount(pins = 0, initiallyScrollable = true) {
  const raw = "first line\nselected line\nlast line";
  loadSingle({ name: "Sync", title: "Sync", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "sync", raw, collapsed: false, children: [] }] });
  const pane = document.createElement("div"); pane.className = "main-content"; pane.style.overflowY = "auto";
  let scrollHeight = initiallyScrollable ? 2000 : 500;
  Object.defineProperties(pane, { clientHeight: { value: 500 }, scrollHeight: { get: () => scrollHeight } });
  pane.getBoundingClientRect = () => new DOMRect(20, 100, 900, 500);
  pane.scrollTop = 1000;
  let parent = pane;
  for (let i = 0; i < pins; i++) {
    const ancestor = document.createElement("div"); ancestor.dataset.stickyParent = "";
    const row = document.createElement("div"); row.className = "block-main";
    row.getBoundingClientRect = () => new DOMRect(20, 50, 900, 20);
    ancestor.append(row); parent.append(ancestor); parent = ancestor;
  }
  const block = document.createElement("div"); block.className = "ls-block"; parent.append(block);
  document.body.append(pane);
  startEditing("sync", 12);
  dispose = render(() => <Cm6BlockEditor id="sync" surface="main" scope={null} />, block);
  const view = EditorView.findFromDOM(block.querySelector(".cm-content")!)!;
  let top = 300;
  const caretTop = () => top - (pane.scrollTop - 1000);
  view.contentDOM.getBoundingClientRect = () => new DOMRect(50, caretTop(), 700, 1000);
  vi.spyOn(view, "coordsAtPos").mockImplementation(() => ({ left: 80, right: 80, top: caretTop(), bottom: caretTop() + 20 }));
  return { pane, view, raw, caretTop, setTop: (value: number) => { top = value; }, grow: () => { scrollHeight = 2000; } };
}

it("CM6 re-reveals the caret when a late image above it loads, then yields to the user", () => {
  const f = mount();
  step(45);
  expect(f.pane.scrollTop).toBe(1000);
  f.setTop(850);
  const image = document.createElement("img"); f.pane.append(image);
  image.dispatchEvent(new Event("load")); step(2);
  expect(f.caretTop() + 20).toBeLessThanOrEqual(600);
  const after = f.pane.scrollTop;
  f.pane.dispatchEvent(new Event("wheel")); f.setTop(1200);
  image.dispatchEvent(new Event("load")); step(2);
  expect(f.pane.scrollTop).toBe(after);
  expect(document.activeElement).toBe(f.view.contentDOM);
  expect(node("sync").raw).toBe(f.raw);
});

it.each([0, 2])("CM6 reveals below a sticky seen notice and %i ancestor pins", pins => {
  const f = mount(pins);
  f.pane.style.scrollPaddingTop = "82px";
  f.pane.style.setProperty("--seen-header-height", "82px");
  f.setTop(130); step(45);
  expect(f.caretTop()).toBeGreaterThanOrEqual(100 + 82 + pins * 30);
  expect(node("sync").raw).toBe(f.raw);
});

it("CM6 follows a late image that makes an initially short pane scrollable", () => {
  const f = mount(0, false); step(45);
  f.grow(); f.setTop(850);
  const image = document.createElement("img"); f.pane.append(image);
  image.dispatchEvent(new Event("load")); step(2);
  expect(f.caretTop() + 20).toBeLessThanOrEqual(600);
  expect(node("sync").raw).toBe(f.raw);
});

it("CM6 desktop toolbar follows the selected line, flips below a notice, and hides outside its pane", async () => {
  const f = mount();
  f.view.dispatch({ selection: { anchor: 11, head: 24 } });
  await Promise.resolve();
  const toolbar = f.pane.querySelector<HTMLElement>(".sel-toolbar")!;
  toolbar.getBoundingClientRect = () => new DOMRect(0, 0, 250, 30);
  // Offset parent is the editor wrapper, which may start above the viewport.
  Object.defineProperty(toolbar, "offsetParent", { value: f.pane });
  f.pane.dispatchEvent(new Event("scroll"));
  const viewportTop = () => parseFloat(toolbar.style.top) - f.pane.scrollTop + 100;
  expect(viewportTop()).toBeLessThan(f.caretTop());
  expect(viewportTop()).toBeGreaterThanOrEqual(100);
  const notice = document.createElement("div"); notice.dataset.seenHeader = "";
  notice.getBoundingClientRect = () => new DOMRect(20, 100, 900, 82); f.pane.prepend(notice);
  f.setTop(190);
  f.view.dispatch({ selection: { anchor: 12, head: 23 } });
  await Promise.resolve(); f.pane.dispatchEvent(new Event("scroll"));
  expect(viewportTop()).toBeGreaterThanOrEqual(f.caretTop() + 20);
  f.setTop(900); f.pane.dispatchEvent(new Event("scroll"));
  expect(toolbar.style.visibility).toBe("hidden");
  f.setTop(300); f.pane.dispatchEvent(new Event("scroll"));
  expect(toolbar.style.visibility).toBe("");
  expect(node("sync").raw).toBe(f.raw);
});
