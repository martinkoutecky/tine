import { afterEach, describe, expect, it, vi } from "vitest";
import { openCommentThread } from "../tests/commentEntry";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { editingId, endEdit, startEditing } from "../editorController";
import { insertEmptyChildBlock, node, pageByName, resetStore, setRaw } from "../document";
import { loadSingle } from "../document/workingSet";
import { persistCommentMarginHidden } from "../commentMarginPref";
import { maybeEditorIn } from "../tests/editorDriver";
import { beginDrag, dragId, dropInd } from "./blockGestures";
import { Block, SurfaceContext } from "./Block";
import type { BlockDto } from "../types";
import { blockRegions } from "../render/parse";
import { closeContextMenu, contextMenu } from "../ui";

const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({
  id, raw, children, collapsed: false,
  properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]),
});
let dispose: (() => void) | undefined;
let mountedSurface = "main";
function mount(child = true, surfaceKey = "main") {
  mountedSurface = surfaceKey;
  loadSingle({ name: "M", title: "M", kind: "page", pre_block: null, blocks: [
    block("parent", "Parent", [
      block("comment", "Remark\ntine.quote:: Parent", [block("reply", "Reply", [block("deep", "Deep")])]),
      ...(child ? [block("child", "Child")] : []),
    ]), block("after", "After"),
  ] });
  const root = document.createElement("div");
  document.body.appendChild(root);
  dispose = render(() => <SurfaceContext.Provider value={surfaceKey}><For each={pageByName("M")!.roots}>{id => <Block id={id} />}</For></SurfaceContext.Provider>, root);
  openCommentThread(root);
  return document.body;
}
function key(root: HTMLElement, id: string, key: string, end = false) {
  startEditing(id, end ? Number.MAX_SAFE_INTEGER : 0, null, mountedSurface);
  const ta = maybeEditorIn(root)!;
  const at = end ? ta.value().length : 0;
  ta.select(at, at);
  ta.keydown(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}
afterEach(() => {
  endEdit("page-navigation"); dispose?.(); dispose = undefined;
  resetStore(); persistCommentMarginHidden(false); document.body.innerHTML = "";
  vi.restoreAllMocks();
  Reflect.deleteProperty(document, "elementFromPoint");
  closeContextMenu();
});

describe("GH #891 body keystrokes treat inline threads as annotations", () => {
  it("Delete at parent end merges Child and preserves every comment", () => {
    const root = mount();
    key(root, "parent", "Delete", true);
    expect(node("parent").raw).toBe("ParentChild");
    expect(node("comment").raw).toBe("Remark\ntine.quote:: Parent");
    expect(node("reply").children).toEqual(["deep"]);
  });
  it("Backspace at Child start merges into Parent", () => {
    const root = mount();
    key(root, "child", "Backspace");
    expect(node("parent").raw).toBe("ParentChild");
    expect(node("deep").raw).toBe("Deep");
  });
  it("Enter at parent end with only comment children creates a body sibling", () => {
    const root = mount(false);
    key(root, "parent", "Enter", true);
    const added = editingId()!;
    expect(node(added).parent).toBeNull();
    expect(node("parent").children).toEqual(["comment"]);
  });
  it("Backspace on an empty reply leaves it inside its thread", () => {
    const root = mount();
    setRaw("deep", "");
    key(root, "deep", "Backspace");
    expect(node("deep")).toMatchObject({ parent: "reply", raw: "" });
  });
  it("body Backspace preserves a reply's numbered-list metadata", () => {
    const root = mount();
    const raw = "Reply\nlogseq.order-list-type:: number";
    setRaw("deep", raw);
    key(root, "deep", "Backspace");
    expect(node("deep")).toMatchObject({ parent: "reply", raw });
  });
  it("retains inline navigation past a quoted reply to the next reply", () => {
    const root = mount();
    setRaw("reply", "Reply\ntine.quote:: Remark");
    const following = insertEmptyChildBlock("comment", 1)!;
    setRaw(following, "Another reply");
    key(root, "deep", "ArrowRight", true);
    expect(editingId()).toBe(following);
  });
  it("Backspace on an empty code reply preserves the reply and its wrapper", () => {
    const root = mount();
    setRaw("deep", "```\n```");
    startEditing("deep", 4, null, "main");
    const ta = maybeEditorIn(root)!;
    expect(ta.value()).toBe("");
    ta.keydown(new KeyboardEvent("keydown", { key: "Backspace", bubbles: true, cancelable: true }));
    expect(node("deep")).toMatchObject({ parent: "reply", raw: "```\n```" });
  });
  it("removing an empty body code wrapper focuses the previous body block across a thread", () => {
    const root = mount();
    setRaw("child", "```\n```");
    startEditing("child", 4, null, "main");
    const ta = maybeEditorIn(root)!;
    expect(ta.value()).toBe("");
    ta.keydown(new KeyboardEvent("keydown", { key: "Backspace", bubbles: true, cancelable: true }));
    expect(node("child")).toBeUndefined();
    expect(editingId()).toBe("parent");
    expect(node("deep").parent).toBe("reply");
  });
  for (const surface of ["main", "ref:fixture", "embed:fixture"]) {
    it.each([false, true])(`all body arrows skip whole threads on ${surface} (hidden=%s)`, hidden => {
      persistCommentMarginHidden(hidden);
      const root = mount(true, surface);
      for (const arrow of ["ArrowRight", "ArrowDown"]) {
        key(root, "parent", arrow, true);
        expect(editingId(), arrow).toBe("child");
      }
      for (const arrow of ["ArrowLeft", "ArrowUp"]) {
        key(root, "child", arrow);
        expect(editingId(), arrow).toBe("parent");
      }
    });
  }
  it("a comment's author chip opens its menu and cannot start a reorder", () => {
    const root = mount();
    const chip = root.querySelector('[data-block-id="comment"] .comment-handle')!;
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: vi.fn(() => null) });
    chip.dispatchEvent(new MouseEvent("mousedown", { button: 0, clientX: 20, clientY: 20, bubbles: true }));
    document.dispatchEvent(new MouseEvent("mousemove", { buttons: 1, clientX: 80, clientY: 80, bubbles: true }));
    const moving = dragId();
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    expect(moving).toBeNull();
    chip.dispatchEvent(new MouseEvent("contextmenu", { button: 2, bubbles: true, cancelable: true }));
    expect(contextMenu()).toMatchObject({ kind: "block", blockId: "comment" });
  });
  it.each(["comment", "reply", "deep"])("a dragged body block offers no drop inside %s", id => {
    const root = mount();
    const target = root.querySelector(`[data-block-id="${id}"]`)!;
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: vi.fn(() => target) });
    const handle = root.querySelector('[data-block-id="after"] .bullet-container')!;
    beginDrag("after", new MouseEvent("mousedown", { clientX: 10, clientY: 10 }));
    document.dispatchEvent(new MouseEvent("mousemove", { buttons: 1, clientX: 80, clientY: 80, bubbles: true }));
    const indicator = dropInd();
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    expect(handle).not.toBeNull();
    expect(indicator).toBeNull();
    expect(node("after").parent).toBeNull();
  });
});
