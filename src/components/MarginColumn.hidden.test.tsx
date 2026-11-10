import { maybeEditorIn, type EditorDriver } from "../tests/editorDriver";
// GH #845: with the comment margin hidden no surface draws a comment thread. Not
// the margin column, not the narrow-pane inline copies, not an embed's copy, and
// not the highlight of the commented words or the count on the commented block;
// the arrow keys never stop on one; nothing is written to the graph.
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { openCommentThread, showThread } from "../tests/commentEntry";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { editingId, endEdit, startEditing } from "../editorController";
import { blockRegions, initParser } from "../render/parse";
import { nextVisible, node as docNode, prevVisible, resetStore, visibleOrder } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import type { BlockDto } from "../types";
import { installKeybindings } from "../keybindings";
import { BlockList } from "./BlockList";
import { Block, SurfaceContext } from "./Block";
import { createMarginSurface } from "./MarginColumn";
import { MarginContext } from "./marginContext";
import { CommentResolutionScope } from "./CommentResolutionScope";
import { forceCommentMarginShown } from "../commentMarginPref";
import { revealCommentMargin, setCommentMarginVisible } from "../commentMargin";

const PAGE = "MarginHidden";

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

let frames: FrameRequestCallback[] = [];
const realRaf = window.requestAnimationFrame;
const realRO = (window as { ResizeObserver?: unknown }).ResizeObserver;
function flushFrames(): void {
  for (let guard = 0; frames.length && guard < 10; guard++) {
    const run = frames;
    frames = [];
    for (const cb of run) cb(0);
  }
}

function Host(): JSX.Element {
  let outline: HTMLDivElement | undefined;
  const margin = createMarginSurface({ page: () => pageByName(PAGE), eligible: () => true, outline: () => outline });
  return (
    <CommentResolutionScope roots={pageByName(PAGE)?.roots ?? []}><div class="page-section">
      <MarginContext.Provider value={margin.placement}>
        <div class="page-blocks" ref={outline} classList={{ "margin-active": margin.active() }}>
          <BlockList ids={pageByName(PAGE)?.roots ?? []} />
        </div>
      </MarginContext.Provider>
      <margin.Column />
    </div></CommentResolutionScope>
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

const blockEl = (root: ParentNode, id: string) => root.querySelector(`.ls-block[data-block-id="${id}"]`) as HTMLElement | null;
const editor = (root: ParentNode) => maybeEditorIn(root) as EditorDriver | null;
const key = (ta: EditorDriver, k: string) => ta.keydown(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));

const commented = () => [
  block("p", "Gradient descent converges here because the step size shrinks.", [
    block("kid", "an ordinary child"),
    block("c1", "Only if the loss is convex.\ntine.quote:: converges here because", [block("r1", "Fair, I will add it.\ntine.author:: claude")]),
    block("c2", "Which step size?\ntine.quote:: the step size"),
  ]),
  block("after", "The next block."),
];
const COMMENTS = ["c1", "r1", "c2"];

let disposeKeys: (() => void) | null = null;
beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});
afterAll(() => disposeKeys?.());
beforeEach(() => {
  localStorage.clear();
  forceCommentMarginShown();
  frames = [];
  window.requestAnimationFrame = (cb) => { frames.push(cb); return frames.length; };
  (window as { ResizeObserver?: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
  localStorage.clear();
  forceCommentMarginShown();
  window.requestAnimationFrame = realRaf;
  (window as { ResizeObserver?: unknown }).ResizeObserver = realRO;
});

const anywhere = (pane: HTMLElement) => COMMENTS.filter((id) => blockEl(pane, id) !== null);

describe("a hidden comment margin", () => {
  it("draws no thread, count or highlight on a wide pane, and shows them again when switched back on", () => {
    const { pane, dispose } = mount(commented());
    try {
      expect(pane.querySelector(".margin-column")).not.toBeNull();
      expect(setCommentMarginVisible(false)).toBe(true);
      flushFrames();
      expect(pane.querySelector(".margin-column")).toBeNull();
      expect(pane.querySelector(".page-blocks")!.classList.contains("margin-active")).toBe(false);
      expect(anywhere(pane)).toEqual([]);
      expect(pane.querySelector(".margin-marker")).toBeNull();
      expect(pane.querySelector(".comment-quote-anchor")).toBeNull();
      expect(blockEl(pane, "kid")).not.toBeNull();
      expect(blockEl(pane, "p")!.textContent).toContain("converges here because the step size shrinks.");
      // The comments are still blocks of the page: nothing was removed or rewritten.
      expect(COMMENTS.every((id) => docNode(id) !== undefined)).toBe(true);
      expect(docNode("c1").raw).toBe("Only if the loss is convex.\ntine.quote:: converges here because");
      expect(docNode("p").children).toEqual(["kid", "c1", "c2"]);

      expect(setCommentMarginVisible(true)).toBe(true);
      flushFrames();
      expect(pane.querySelector(".margin-column")).not.toBeNull();
      expect(blockEl(pane.querySelector(".margin-column")!, "c1")).not.toBeNull();
      expect(pane.querySelector(".page-blocks button.margin-marker")).toBeNull();
      expect(pane.querySelectorAll(".comment-quote-anchor")).toHaveLength(2);
    } finally {
      dispose();
    }
  });

  it("hides and restores narrow-pane entry points without any inline fallback", () => {
    const { pane, dispose } = mount(commented(), 800);
    try {
      expect(anywhere(pane)).toEqual([]);
      // D-20261108-MC8: the highlights are the entry points; no counted marker.
      expect(pane.querySelector(".margin-marker")).toBeNull();
      expect(pane.querySelectorAll(".comment-quote-anchor")).toHaveLength(2);
      setCommentMarginVisible(false);
      expect(anywhere(pane)).toEqual([]);
      expect(pane.querySelector(".comment-quote-anchor")).toBeNull();
      setCommentMarginVisible(true);
      expect(anywhere(pane)).toEqual([]);
      openCommentThread(pane, "c1");
      // Separate passages: the shown one is mounted, the other when reached (review round 3).
      expect(anywhere(document.body)).toEqual(["c1", "r1"]);
      expect(showThread("c2")).not.toBeNull();
      expect(anywhere(document.body)).toEqual(["c2"]);
      expect(pane.querySelectorAll(".comment-quote-anchor")).toHaveLength(2);
    } finally {
      dispose();
    }
  });

  it("covers a split pane, the sidebar and a zoomed block (plain Blocks outside the margin) and an embed copy", () => {
    const { pane, dispose } = mount(commented(), 1400, () => (
      <div>
        <div class="plain"><Block id="p" /></div>
        <div class="embed"><SurfaceContext.Provider value="embed:test"><BlockList ids={["p"]} /></SurfaceContext.Provider></div>
      </div>
    ));
    try {
      expect(pane.querySelectorAll(".plain .comment-quote-anchor")).toHaveLength(2);
      expect(pane.querySelectorAll(".embed .comment-quote-anchor")).toHaveLength(2);
      setCommentMarginVisible(false);
      for (const part of ["plain", "embed"]) {
        const surface = pane.querySelector(`.${part}`)!;
        expect(COMMENTS.filter((id) => blockEl(surface, id))).toEqual([]);
        expect(blockEl(surface, "kid")).not.toBeNull();
        expect(surface.querySelector(".comment-quote-anchor")).toBeNull();
      }
    } finally {
      dispose();
    }
  });

  it("still draws a comment that is the zoomed block itself", () => {
    const { pane, dispose } = mount(commented(), 1400, () => <Block id="c1" forceExpanded />);
    try {
      setCommentMarginVisible(false);
      expect(blockEl(pane, "c1")).not.toBeNull();
      // Its reply is an ordinary block under the comment, not a comment of its own.
      expect(blockEl(pane, "r1")).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("keeps the arrow keys off hidden threads, in the model and in the editor", () => {
    expect(visibleOrder()).toEqual([]);
    const { pane, dispose } = mount(commented(), 800);
    try {
      expect(visibleOrder()).toEqual(["p", "c1", "r1", "c2", "kid", "after"]);
      setCommentMarginVisible(false);
      expect(visibleOrder()).toEqual(["p", "kid", "after"]);
      expect(nextVisible("p")).toBe("kid");
      expect(prevVisible("after")).toBe("kid");
      startEditing("p", 0);
      const ta = editor(blockEl(pane, "p")!)!;
      key(ta, "ArrowDown");
      expect(editingId()).toBe("kid");
      key(editor(blockEl(pane, "kid")!)!, "ArrowDown");
      expect(editingId()).toBe("after");
      key(editor(blockEl(pane, "after")!)!, "ArrowUp");
      expect(editingId()).toBe("kid");
    } finally {
      dispose();
    }
  });

  it("moves a caret in a thread to the commented block first, keeping the text", () => {
    const { pane, dispose } = mount(commented());
    try {
      startEditing("c1", 3);
      expect(editingId()).toBe("c1");
      const before = docNode("c1").raw;
      expect(setCommentMarginVisible(false)).toBe(true);
      expect(editingId()).toBe("p");
      expect(docNode("c1").raw).toBe(before);
      flushFrames();
      expect(anywhere(pane)).toEqual([]);
      expect(pane.querySelector(".margin-column")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("is shown again by revealCommentMargin, which every comment write calls", () => {
    const { pane, dispose } = mount(commented());
    try {
      setCommentMarginVisible(false);
      revealCommentMargin();
      flushFrames();
      expect(pane.querySelector(".margin-column")).not.toBeNull();
    } finally {
      dispose();
    }
  });
});
