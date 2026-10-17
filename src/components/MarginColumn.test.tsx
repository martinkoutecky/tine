// Margin dialogue, slice 2 (vision §3.7 "Layout"): on a wide main pane, a page
// that holds comments draws them in a right-hand column instead of the outline,
// with a count marker on the commented block; the threads are ordinary Blocks,
// edited in place; arrows lead in and out of them; hovering a thread emphasises
// its passage and vice versa; and a keystroke costs at most one measurement pass.
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { editingId, endEdit, startEditing } from "../editorController";
import { blockRegions, initParser } from "../render/parse";
import { node as docNode, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import type { BlockDto } from "../types";
import { installKeybindings } from "../keybindings";
import { marginMeasurePasses } from "../margin";
import { BlockList } from "./BlockList";
import { SurfaceContext } from "./Block";
import { createMarginSurface } from "./MarginColumn";
import { MarginContext } from "./marginContext";

const PAGE = "MarginColumn";

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

// A frame queue and ResizeObservers the test drives, standing in for the
// browser's rendering steps.
let frames: FrameRequestCallback[] = [];
const observers: { cb: ResizeObserverCallback; targets: Set<Element> }[] = [];
const realRaf = window.requestAnimationFrame;
const realRO = (window as { ResizeObserver?: unknown }).ResizeObserver;
function flushFrames(): void {
  for (let guard = 0; frames.length && guard < 10; guard++) {
    const run = frames;
    frames = [];
    for (const cb of run) cb(0);
  }
}
/** The browser noticed `el` resized: every observer watching it is told once. */
function resized(el: Element): void {
  for (const o of observers) if (o.targets.has(el)) o.cb([{ target: el } as ResizeObserverEntry], {} as ResizeObserver);
}

function Host(props: { eligible?: boolean }): JSX.Element {
  let outline: HTMLDivElement | undefined;
  const margin = createMarginSurface({ page: () => pageByName(PAGE), eligible: () => props.eligible ?? true, outline: () => outline });
  return (
    <div class="page-section">
      <MarginContext.Provider value={margin.placement}>
        <div class="page-blocks" ref={outline} classList={{ "margin-active": margin.active() }}>
          <BlockList ids={pageByName(PAGE)?.roots ?? []} />
        </div>
      </MarginContext.Provider>
      <margin.Column />
    </div>
  );
}

function mount(blocks: BlockDto[], width = 1400, host: () => JSX.Element = () => <Host />): { pane: HTMLElement; dispose: () => void } {
  loadSingle({ name: PAGE, kind: "page", title: PAGE, pre_block: null, blocks });
  const pane = document.createElement("main");
  pane.className = "main-content";
  Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => width });
  document.body.appendChild(pane);
  const dispose = render(host, pane);
  flushFrames();
  return { pane, dispose };
}

const outline = (pane: HTMLElement) => pane.querySelector(".page-blocks") as HTMLElement;
const column = (pane: HTMLElement) => pane.querySelector(".margin-column") as HTMLElement | null;
const thread = (pane: HTMLElement, id: string) => pane.querySelector(`.margin-thread[data-thread-id="${id}"]`) as HTMLElement | null;
const blockEl = (root: ParentNode, id: string) => root.querySelector(`.ls-block[data-block-id="${id}"]`) as HTMLElement | null;
const editor = (root: ParentNode) => root.querySelector("textarea.block-editor") as HTMLTextAreaElement | null;
const key = (ta: HTMLTextAreaElement, k: string) =>
  ta.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));

const commented = () => [
  block("p", "Gradient descent converges here because the step size shrinks.", [
    block("kid", "an ordinary child"),
    block("c1", "Only if the loss is convex.\nquote:: converges here because", [block("r1", "Fair, I will add it.\nauthor:: claude")]),
    block("c2", "Which step size?\nquote:: the step size"),
  ]),
  block("after", "The next block."),
];

let disposeKeys: (() => void) | null = null;
beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});
afterAll(() => disposeKeys?.());
beforeEach(() => {
  frames = [];
  observers.length = 0;
  window.requestAnimationFrame = (cb) => { frames.push(cb); return frames.length; };
  (window as { ResizeObserver?: unknown }).ResizeObserver = class {
    entry: { cb: ResizeObserverCallback; targets: Set<Element> };
    constructor(cb: ResizeObserverCallback) { observers.push(this.entry = { cb, targets: new Set() }); }
    observe(el: Element) { this.entry.targets.add(el); }
    unobserve(el: Element) { this.entry.targets.delete(el); }
    disconnect() { this.entry.targets.clear(); }
  };
});
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
  window.requestAnimationFrame = realRaf;
  (window as { ResizeObserver?: unknown }).ResizeObserver = realRO;
});

describe("the margin column on a wide main pane", () => {
  it("draws each comment thread in the column, not in the outline, with a count on the commented block", () => {
    const { pane, dispose } = mount(commented());
    try {
      expect(outline(pane).classList.contains("margin-active")).toBe(true);
      // The outline keeps the parent, its ordinary child and the next block only.
      expect(blockEl(outline(pane), "p")).not.toBeNull();
      expect(blockEl(outline(pane), "kid")).not.toBeNull();
      expect(blockEl(outline(pane), "c1")).toBeNull();
      expect(blockEl(outline(pane), "r1")).toBeNull();
      expect(blockEl(outline(pane), "c2")).toBeNull();
      // Each thread is an ordinary Block (its replies included) in the column.
      expect(blockEl(thread(pane, "c1")!, "c1")!.classList.contains("comment-card")).toBe(true);
      expect(blockEl(thread(pane, "c1")!, "r1")).not.toBeNull();
      expect(blockEl(thread(pane, "c2")!, "c2")).not.toBeNull();
      const marker = blockEl(outline(pane), "p")!.querySelector(":scope > .block-main > .margin-marker")!;
      expect(marker.textContent).toBe("2");
      // The quoted passages stay marked in the parent, named by their comment.
      const marks = [...blockEl(outline(pane), "p")!.querySelectorAll<HTMLElement>(".comment-quote-anchor")];
      expect(marks.map((m) => [m.dataset.commentId, m.textContent])).toEqual([["c1", "converges here because"], ["c2", "the step size"]]);
      // Both threads were positioned by a measurement pass.
      expect(thread(pane, "c1")!.style.top).not.toBe("");
      expect(thread(pane, "c2")!.style.top).not.toBe("");
    } finally {
      dispose();
    }
  });

  it("leaves a page without comments exactly as the plain outline renders it", () => {
    const blocks = () => [block("a", "one", [block("b", "two")]), block("c", "three\nauthor:: claude")];
    const plain = mount(blocks(), 1400, () => <div class="page-section"><div class="page-blocks"><BlockList ids={pageByName(PAGE)?.roots ?? []} /></div></div>);
    const plainHtml = plain.pane.innerHTML;
    plain.dispose();
    document.body.innerHTML = "";
    resetStore();
    const { pane, dispose } = mount(blocks());
    try {
      expect(column(pane)).toBeNull();
      expect(outline(pane).classList.contains("margin-active")).toBe(false);
      expect(pane.querySelector(".margin-marker")).toBeNull();
      // Instance ids differ between mounts; the structure and text do not.
      const normal = (html: string) => html.replace(/ data-[a-z-]+="[^"]*"/g, "");
      expect(normal(pane.innerHTML)).toBe(normal(plainHtml));
    } finally {
      dispose();
    }
  });

  it("keeps slice 1's inline threads on a narrow pane and off the main pane's single-page view", () => {
    for (const [width, eligible] of [[800, true], [1400, false]] as const) {
      const { pane, dispose } = mount(commented(), width, () => <Host eligible={eligible} />);
      try {
        expect(column(pane)).toBeNull();
        expect(blockEl(outline(pane), "c1")).not.toBeNull();
        expect(blockEl(outline(pane), "r1")).not.toBeNull();
        expect(pane.querySelector(".margin-marker")).toBeNull();
      } finally {
        dispose();
        document.body.innerHTML = "";
        resetStore();
      }
    }
  });

  it("an embed of the commented block inside a margin page keeps its comments inline", () => {
    const { pane, dispose } = mount(commented(), 1400, () => {
      let ref: HTMLDivElement | undefined;
      const margin = createMarginSurface({ page: () => pageByName(PAGE), eligible: () => true, outline: () => ref });
      return (
        <div class="page-section">
          <MarginContext.Provider value={margin.placement}>
            <div class="page-blocks" ref={ref} classList={{ "margin-active": margin.active() }}>
              <BlockList ids={pageByName(PAGE)?.roots ?? []} />
              <div class="test-embed"><SurfaceContext.Provider value="embed:test"><BlockList ids={["p"]} /></SurfaceContext.Provider></div>
            </div>
          </MarginContext.Provider>
          <margin.Column />
        </div>
      );
    });
    try {
      const embed = pane.querySelector(".test-embed") as HTMLElement;
      expect(column(pane)).not.toBeNull();
      expect(blockEl(embed, "c1")).not.toBeNull();
      expect(blockEl(embed, "r1")).not.toBeNull();
      expect(embed.querySelector(".margin-marker")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("measures again when a web font finishes loading, which moves passages without resizing anything", () => {
    const fonts = new EventTarget();
    Object.defineProperty(document, "fonts", { configurable: true, value: fonts });
    try {
      const { dispose } = mount(commented());
      try {
        const before = marginMeasurePasses();
        fonts.dispatchEvent(new Event("loadingdone"));
        flushFrames();
        expect(marginMeasurePasses()).toBe(before + 1);
      } finally {
        dispose();
      }
    } finally {
      delete (document as { fonts?: unknown }).fonts;
    }
  });

  it("measures again when the commented block's text renders its passages after the row", async () => {
    const { pane, dispose } = mount(commented());
    try {
      const before = marginMeasurePasses();
      const content = blockEl(outline(pane), "p")!.querySelector(":scope > .block-main .block-content")!;
      content.appendChild(document.createElement("span"));
      await Promise.resolve();
      flushFrames();
      expect(marginMeasurePasses()).toBe(before + 1);
    } finally {
      dispose();
    }
  });

  it("switches to the margin when the pane widens past the breakpoint", () => {
    let width = 800;
    loadSingle({ name: PAGE, kind: "page", title: PAGE, pre_block: null, blocks: commented() });
    const pane = document.createElement("main");
    pane.className = "main-content";
    Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => width });
    document.body.appendChild(pane);
    const dispose = render(() => <Host />, pane);
    try {
      expect(column(pane)).toBeNull();
      width = 1200;
      resized(pane);
      flushFrames();
      expect(column(pane)).not.toBeNull();
      expect(blockEl(outline(pane), "c1")).toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("editing in the margin", () => {
  it("a click on a margin comment edits it in place with the ordinary block editor", () => {
    const { pane, dispose } = mount(commented());
    try {
      const wrapper = blockEl(thread(pane, "c2")!, "c2")!.querySelector(".block-content-wrapper")!;
      wrapper.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      expect(editingId()).toBe("c2");
      expect(editor(thread(pane, "c2")!)).not.toBeNull();
      expect(editor(outline(pane))).toBeNull();
    } finally {
      dispose();
    }
  });

  it("Enter in a comment's text adds a reply inside its thread and keeps the quote on the comment", async () => {
    const { pane, dispose } = mount(commented());
    try {
      startEditing("c2", "Which step size?".length);
      await Promise.resolve();
      const ta = editor(thread(pane, "c2")!)!;
      expect(ta.value.startsWith("Which step size?")).toBe(true);
      ta.setSelectionRange("Which step size?".length, "Which step size?".length);
      key(ta, "Enter");
      const reply = docNode("c2")!.children[0];
      expect(reply).toBeDefined();
      expect(docNode(reply)!.raw).toBe("");
      expect(docNode("c2")!.raw).toBe("Which step size?\nquote:: the step size");
      expect(editingId()).toBe(reply);
      await Promise.resolve();
      expect(editor(thread(pane, "c2")!)!.closest<HTMLElement>(".ls-block")!.dataset.blockId).toBe(reply);
    } finally {
      dispose();
    }
  });

  it("arrow up from a thread's first line returns to the commented block; down past its end goes to the next outline block", async () => {
    const { pane, dispose } = mount(commented());
    try {
      startEditing("c1", 0);
      await Promise.resolve();
      key(editor(thread(pane, "c1")!)!, "ArrowUp");
      expect(editingId()).toBe("p");
      startEditing("r1", 0);
      await Promise.resolve();
      const ta = editor(thread(pane, "c1")!)!;
      // The reply's own property line is the last line: put the caret there.
      ta.setSelectionRange(ta.value.length, ta.value.length);
      key(ta, "ArrowDown");
      expect(editingId()).toBe("kid");
    } finally {
      dispose();
    }
  });

  it("arrows in the outline step over the threads drawn in the margin", async () => {
    const { pane, dispose } = mount(commented());
    try {
      startEditing("kid", "an ordinary child".length);
      await Promise.resolve();
      const ta = editor(outline(pane))!;
      ta.setSelectionRange(ta.value.length, ta.value.length);
      key(ta, "ArrowDown");
      expect(editingId()).toBe("after");
      await Promise.resolve();
      key(editor(outline(pane))!, "ArrowUp");
      expect(editingId()).toBe("kid");
    } finally {
      dispose();
    }
  });
});

describe("emphasis between a thread and its passage", () => {
  it("hovering a thread marks its passage, and hovering a passage marks its thread", () => {
    const { pane, dispose } = mount(commented());
    try {
      const mark = (id: string) => outline(pane).querySelector<HTMLElement>(`.comment-quote-anchor[data-comment-id="${id}"]`)!;
      thread(pane, "c2")!.dispatchEvent(new MouseEvent("mouseenter"));
      expect(mark("c2").classList.contains("active")).toBe(true);
      expect(mark("c1").classList.contains("active")).toBe(false);
      thread(pane, "c2")!.dispatchEvent(new MouseEvent("mouseleave"));
      expect(mark("c2").classList.contains("active")).toBe(false);
      mark("c1").firstElementChild!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      expect(thread(pane, "c1")!.classList.contains("active")).toBe(true);
      expect(thread(pane, "c2")!.classList.contains("active")).toBe(false);
    } finally {
      dispose();
    }
  });
});

describe("measurement cost", () => {
  it("a keystroke in the outline of a 300-block page with 20 comments costs at most one measurement pass", async () => {
    const blocks: BlockDto[] = [];
    for (let i = 0; i < 300; i++) {
      const kids = i % 15 === 0 ? [block(`c${i}`, `comment ${i}\nquote:: text ${i}`)] : [];
      blocks.push(block(`b${i}`, `block text ${i} here`, kids));
    }
    const { pane, dispose } = mount(blocks);
    try {
      expect(pane.querySelectorAll(".margin-thread")).toHaveLength(20);
      startEditing("b7", 0);
      await Promise.resolve();
      flushFrames();
      const ta = editor(outline(pane))!;
      const before = marginMeasurePasses();
      ta.value = "x" + ta.value;
      ta.setSelectionRange(1, 1);
      ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "x" }));
      expect(docNode("b7")!.raw).toBe("xblock text 7 here");
      // The typed line rewrapped: the browser reports the outline resized.
      resized(outline(pane));
      flushFrames();
      // One pass for the resize and the edit together, never one per trigger.
      expect(marginMeasurePasses() - before).toBe(1);
      // A commented block's own keystroke (its passage can move) also costs one.
      endEdit("page-navigation");
      startEditing("b15", 0);
      await Promise.resolve();
      flushFrames();
      const parent = editor(outline(pane))!;
      const again = marginMeasurePasses();
      parent.value = "y" + parent.value;
      parent.setSelectionRange(1, 1);
      parent.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "y" }));
      resized(outline(pane));
      flushFrames();
      expect(marginMeasurePasses() - again).toBe(1);
    } finally {
      dispose();
    }
  });
});
