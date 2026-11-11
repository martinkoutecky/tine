import { afterAll, afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser, blockRegions, editBlock } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { node, resetStore, setRaw, setCollapsed, insertEmptyChildBlock, undo } from "../document";
import { marginNext } from "../margin";
import type { BlockDto } from "../types";
import { collapseComments, createComment } from "../commentActions";
import { startEditing, endEdit } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { installKeybindings } from "../keybindings";
import { setDoc } from "../document/model";
import { setCm6EditorEnabled } from "../editor/cm6Setting";
import { preloadCm6Editor } from "./EditorGate";

let disposeKeys: (() => void) | undefined;
beforeAll(async () => {
  await initParser(); await preloadCm6Editor(); disposeKeys = installKeybindings();
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
afterAll(() => disposeKeys?.());
afterEach(() => { endEdit("page-navigation"); resetStore(); setCm6EditorEnabled(false); document.body.innerHTML = ""; });
const date = "\ntine.date:: 2026-11-07T13:37Z";
function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, children, collapsed: false, properties: blockRegions(raw, "md").properties.map(p => [p.key, p.value]) };
}
function mount(dated = true) {
  loadSingle({ name: "Messages", kind: "page", title: "Messages", pre_block: null, blocks: [block("p", "Passage", [
    block("c", "First message\ntine.quote:: Passage" + (dated ? date : ""), [
      block("body", "Evidence", [block("nested", "Nested evidence"), block("r", "Second message" + (dated ? date : ""), [block("rb", "Reply evidence")])]),
    ]),
  ])] });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Block id="c" />, host);
  return { host, dispose };
}
it("draws only dated messages with headers and keeps their body bullets nested", () => {
  const { host, dispose } = mount();
  try {
    expect([...host.querySelectorAll(".comment-header")].map(h => h.closest<HTMLElement>(".ls-block")!.dataset.blockId)).toEqual(["c", "r"]);
    expect(host.querySelector('[data-block-id="body"] .bullet-container')).not.toBeNull();
    expect(host.querySelector('[data-block-id="body"] [data-block-id="nested"]')).not.toBeNull();
    expect(host.querySelector('[data-block-id="body"] [data-block-id="r"]')).toBeNull();
    expect(host.querySelector('[data-block-id="r"] [data-block-id="rb"]')).not.toBeNull();
  } finally { dispose(); }
});
it("keeps undated threads flat and does not edit their raw text on open", () => {
  const { host, dispose } = mount(false);
  try {
    expect(host.querySelectorAll(".comment-header")).toHaveLength(5);
    expect(node("c")!.raw).toBe("First message\ntine.quote:: Passage");
    expect(node("body")!.raw).toBe("Evidence");
  } finally { dispose(); }
});
it("removing a message date turns it into ordinary body without losing its descendants", () => {
  const { host, dispose } = mount();
  try {
    setRaw("r", editBlock(node("r")!.raw, "md", { kind: "property", key: "tine.date", value: null }));
    expect(host.querySelectorAll(".comment-header")).toHaveLength(1);
    expect(host.querySelector('[data-block-id="body"] [data-block-id="r"] [data-block-id="rb"]')).not.toBeNull();
    expect(node("rb")!.raw).toBe("Reply evidence");
  } finally { dispose(); }
});
it("arrow navigation follows the drawn message outlines when replies are hand-nested", () => {
  const { dispose } = mount();
  try {
    const tail = insertEmptyChildBlock("c", 1)!;
    setRaw(tail, "Root's trailing body bullet");
    const nav = { format: "md" as const, thread: { root: "c", parent: "p", order: () => ["c"] } };
    expect(marginNext("nested", null, nav)).toBe(tail);
    expect(marginNext(tail, null, nav)).toBe("r");
    setCollapsed("body", true);
    expect(marginNext("body", null, nav)).toBe(tail);
    expect(marginNext(tail, null, nav)).toBe("r");
  } finally { dispose(); }
});
it.each(["textarea", "cm6"])("%s Enter creates an ordinary body sibling and Tab nests it within the same message", async kind => {
  setCm6EditorEnabled(kind === "cm6");
  const { host, dispose } = mount();
  try {
    startEditing("nested", Number.MAX_SAFE_INTEGER);
    const editor = editorIn(host);
    editor.select(editor.value().length, editor.value().length);
    editor.keydown(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    expect(node("body")!.children).toHaveLength(3);
    const sibling = node("body")!.children[1];
    expect(node(sibling)!.raw).not.toContain("tine.date");
    await Promise.resolve();
    startEditing(sibling, 0);
    editorIn(host).keydown(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    expect(node(sibling)!.parent).toBe("nested");
    expect(node("r")!.parent).toBe("body");
  } finally { dispose(); }
});
it("folds a message body and a whole thread through stored properties and undo", () => {
  const { host, dispose } = mount();
  try {
    setCollapsed("r", true);
    expect(node("r")!.raw).toContain("collapsed:: true");
    expect(host.querySelector('[data-block-id="rb"]')).toBeNull();
    undo();
    expect(host.querySelector('[data-block-id="rb"]')).not.toBeNull();
    setCollapsed("c", true);
    expect(host.querySelector('[data-block-id="body"]')).toBeNull();
    expect(host.querySelector('[data-block-id="r"]')).toBeNull();
    expect(host.textContent).toContain("1 reply");
    undo();
    expect(host.querySelector('[data-block-id="r"]')).not.toBeNull();
  } finally { dispose(); }
});
it("folds all passage threads as one undo without folding their body blocks", () => {
  const { dispose } = mount();
  try {
    const second = createComment("p", "Passage", 0, 7)!;
    collapseComments("p");
    expect(node("c")!.collapsed).toBe(true);
    expect(node(second)!.collapsed).toBe(true);
    expect(node("body")!.collapsed).toBe(false);
    undo();
    expect(node(second)!.collapsed).toBe(false);
    expect(node("c")!.raw).toBe("First message\ntine.quote:: Passage" + date);
  } finally { dispose(); }
});
it("uses ordinary view folds to expand a read-only thread without changing its file properties", () => {
  const { host, dispose } = mount();
  try {
    setCollapsed("c", true);
    const raw = node("c")!.raw;
    setDoc("pages", 0, "readOnly", true);
    host.querySelector<HTMLButtonElement>(".comment-expand")!.click();
    expect(host.querySelector('[data-block-id="rb"]')).not.toBeNull();
    expect(node("c")!.raw).toBe(raw);
    expect(node("c")!.collapsed).toBe(true);
    host.querySelector<HTMLButtonElement>(":scope > .ls-block > .comment-header .comment-collapse")!.click();
    expect(host.querySelector('[data-block-id="rb"]')).toBeNull();
    expect(node("c")!.raw).toBe(raw);
  } finally { dispose(); }
});
