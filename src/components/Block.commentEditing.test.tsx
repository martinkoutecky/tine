import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { openCommentThread } from "../tests/commentEntry";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { blockRegions, initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { pageToDto } from "../document/convert";
import { node, resetStore } from "../document";
import { editingId, endEdit, startEditing } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { installTouchMedia, setTouchInput } from "../tests/touchMediaTestkit";
import { bindTestHost } from "../document/host/wiring.test.support";
import { backend } from "../backend";
import { resetCommentAuthorNameForTests } from "../commentIdentity";
import { submitReply } from "../commentActions";
import type { BlockDto } from "../types";
import type { Format } from "../render/ast";

let dispose: (() => void) | undefined;
beforeAll(async () => { await initParser(); installTouchMedia(); });
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
  resetCommentAuthorNameForTests("Ada Test");
});
afterEach(() => {
  endEdit("page-navigation"); dispose?.(); dispose = undefined; resetStore();
  setTouchInput(false); resetCommentAuthorNameForTests();
  globalThis.__TINE_E2E_TOUCH_GESTURES__ = undefined;
  document.body.replaceChildren(); vi.restoreAllMocks();
});

function block(id: string, raw: string, format: Format, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, format).properties.filter(p => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, properties, children, collapsed: false };
}

it.each(["c", "deep"])("Enter and Shift+Enter keep %s multiline; modified Enter leaves editing without a reply", async id => {
  loadSingle({ name: "Keys", title: "Keys", kind: "page", format: "md", pre_block: null, blocks: [
    block("p", "words", "md", [block("c", "Root\ntine.quote:: words", "md", [block("deep", "Reply", "md")])]),
  ] });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Block id="p" />, root);
  startEditing(id, 0);
  const editor = editorIn(document.body);
  editor.select(0, editor.value().length);
  editor.keydown({ key: "Enter" });
  await Promise.resolve();
  expect(editingId()).toBe(id);
  expect(editor.value()).toBe("\n");
  editor.keydown({ key: "Enter", shiftKey: true });
  await Promise.resolve();
  expect(editor.value()).toBe("\n\n");
  editor.keydown({ key: "Enter", altKey: true });
  await Promise.resolve();
  expect(editor.value()).toBe("\n\n\n");
  editor.input("multiline\nkept");
  editor.keydown({ key: "Enter", ctrlKey: true });
  expect(editingId()).toBeNull();
  expect(node(id)!.raw).toContain("multiline\nkept");
  expect(node("p")!.children).toEqual(["c"]);
  expect(node("c")!.children).toEqual(["deep"]);
  expect(node("deep")!.children).toEqual([]);
});

it("reply drafts keep Enter as text, ignore composing submission and send with modified Enter", () => {
  loadSingle({ name: "Keys", title: "Keys", kind: "page", format: "md", pre_block: null, blocks: [
    block("p", "words", "md", [block("c", "Root\ntine.quote:: words", "md")]),
  ] });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Block id="p" />, root);
  openCommentThread(root);
  const draft = document.querySelector<HTMLTextAreaElement>(".comment-reply-draft")!;
  draft.focus(); draft.value = "line one\nline two"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  const enter = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
  draft.dispatchEvent(enter);
  expect(enter.defaultPrevented).toBe(false);
  expect(node("c")!.children).toEqual([]);
  draft.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, isComposing: true, bubbles: true }));
  expect(node("c")!.children).toEqual([]);
  draft.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true, cancelable: true }));
  expect(node("c")!.children).toHaveLength(1);
  expect(node(node("c")!.children[0])!.raw).toContain("line one\nline two");
});

it.each((["md", "org"] as const).flatMap(format => [false, true].flatMap(touch => ["c", "deep"].map(id => ({ format, touch, id })))))("$format comment $id hides every tine property (touch=$touch), retaining exact bytes", ({ format, touch, id }) => {
  const metadata = format === "md"
    ? "tine.author:: Ada Test  \t\ntine.custom:: opaque  \t\nTiNe.future:: retained  \t"
    : ":PROPERTIES:\n:tine.author: Ada Test  \t\n:tine.custom: opaque  \t\n:TiNe.future: retained  \t\n:END:";
  const rootMetadata = format === "md" ? `tine.quote:: words  \t\n${metadata}` : metadata.replace(":END:", ":tine.quote: words  \t\n:END:");
  loadSingle({ name: "Comment editors", title: "Comment editors", kind: "page", format, pre_block: null, blocks: [
    block("p", "words", format, [block("c", `Root\n${rootMetadata}`, format, [block("r", "Earlier reply", format, [block("deep", `Reply\n${metadata}`, format)])])]),
  ] });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Block id="p" />, root);
  setTouchInput(touch);
  globalThis.__TINE_E2E_TOUCH_GESTURES__ = touch ? "ios" : undefined;
  const body = id === "c" ? "Root" : "Reply";
  const hidden = id === "c" ? rootMetadata : metadata;
  openCommentThread(root);
  const content = document.querySelector(`[data-block-id="${id}"] > .block-main .block-content`)!;
  // A tap's compatibility mousedown enters the same Editor as a mouse.
  if (touch) for (const type of ["touchstart", "touchend"]) {
    const event = new Event(type, { bubbles: true, cancelable: true });
    const point = { identifier: 1, target: content, clientX: 100, clientY: 100 };
    Object.defineProperties(event, { touches: { value: type === "touchend" ? [] : [point] }, changedTouches: { value: [point] } });
    content.dispatchEvent(event);
  }
  content.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));
  document.dispatchEvent(new MouseEvent("mouseup", { button: 0, bubbles: true }));
  expect(editingId()).toBe(id);
  const editor = editorIn(document.body);
  expect(editor.value()).toBe(body);
  editor.input(`${body} revised`);
  endEdit("page-navigation");
  expect(node(id)!.raw).toBe(`${body} revised\n${hidden}`);
  const savedComment = pageToDto("Comment editors")!.blocks[0].children[0];
  expect((id === "c" ? savedComment : savedComment.children[0].children[0]).raw).toBe(node(id)!.raw);
  const reply = submitReply("c", "A new reply")!;
  expect(node(reply)!.raw).toContain(format === "md" ? "tine.author:: Ada Test" : ":tine.author: Ada Test");
  expect(node(reply)!.raw).not.toMatch(/tine.author(?:::|:) you/);
});
