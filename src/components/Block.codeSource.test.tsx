import { For } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { node, pageByName } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit, startEditing } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { blk, installBlockEditorLifecycle, mount, page } from "../tests/blockEditorTestkit";
import { Block } from "./Block";

const input = vi.hoisted(() => ({ touch: false }));
vi.mock("../inputCapability", async original => ({
  ...(await original<typeof import("../inputCapability")>()),
  touchPrimary: () => input.touch,
}));

installBlockEditorLifecycle();
// Run under both render and CM6 configs: block navigation always opens source.
afterEach(() => endEdit("page-navigation"));
const raw = "```js\nconst x = 1;\n```";

function outline() {
  loadSingle(page("Code source", [blk("before", "before"), blk("code", raw), blk("after", "")]));
  return mount(() => <For each={pageByName("Code source")!.roots}>{id => <Block id={id} />}</For>);
}

describe.each([false, true])("standalone code source (GH #879; touch primary: %s)", touch => {
  beforeEach(() => { input.touch = touch; });
  it.each(["Backspace", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"])("%s into the block exposes its fences", key => {
    const { root, dispose } = outline();
    try {
      const fromAbove = key === "ArrowDown" || key === "ArrowRight";
      startEditing(fromAbove ? "before" : "after", fromAbove ? 6 : 0);
      editorIn(root).keydown({ key, bubbles: true, cancelable: true });
      const editor = editorIn(root);
      expect(editor.blockId).toBe("code");
      expect(editor.value()).toBe(raw);
      editor.input("intro\n```text\nconst x = 1;\n```\nafter", 5);
      endEdit("page-navigation");
      expect(node("code")!.raw).toBe("intro\n```text\nconst x = 1;\n```\nafter");
    } finally { dispose(); }
  });

  it("block entry keeps the source visible while moving into the payload", () => {
    const { root, dispose } = outline();
    try {
      startEditing("code", 0);
      const editor = editorIn(root);
      expect(editor.value()).toBe(raw);
      editor.select(raw.indexOf("const"), raw.indexOf("const"));
      editor.element.dispatchEvent(new Event("select", { bubbles: true }));
      expect(editor.value()).toBe(raw);
    } finally { dispose(); }
  });

  it("returning from code editing keeps the latest payload and permits surrounding prose", () => {
    const { root, dispose } = outline();
    try {
      startEditing("code", raw.indexOf("const"));
      const editor = editorIn(root);
      editor.input("updated code", 12);
      editor.keydown({ key: "Escape", bubbles: true, cancelable: true });
      expect(editorIn(root).value()).toBe("```js\nupdated code\n```");
      editorIn(root).input("before\n```text\nupdated code\n```\nafter", 6);
      endEdit("page-navigation");
      expect(node("code")!.raw).toBe("before\n```text\nupdated code\n```\nafter");
    } finally { dispose(); }
  });

  it("Org block entry exposes and edits its source wrapper", () => {
    const source = "#+BEGIN_SRC python\nx = 1\n#+END_SRC";
    loadSingle({ ...page("Org source", [blk("org", source)]), format: "org" });
    const { root, dispose } = mount(() => <Block id="org" />);
    try {
      startEditing("org", 0);
      expect(editorIn(root).value()).toBe(source);
      editorIn(root).input("before\n#+BEGIN_SRC text\nx = 1\n#+END_SRC\nafter", 6);
      endEdit("page-navigation");
      expect(node("org")!.raw).toBe("before\n#+BEGIN_SRC text\nx = 1\n#+END_SRC\nafter");
    } finally { dispose(); }
  });

  it("clicking the block outside its code opens source", () => {
    const { root, dispose } = outline();
    try {
      const content = root.querySelector<HTMLElement>('[data-block-id="code"] .block-content')!;
      content.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true }));
      document.dispatchEvent(new MouseEvent("mouseup", { button: 0, bubbles: true }));
      expect(editorIn(root).value()).toBe(raw);
    } finally { dispose(); }
  });

  it.each(["Escape", "Backspace", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"])("%s from code editing reveals the same source", key => {
    const { root, dispose } = outline();
    try {
      // A direct code click requests its position inside the payload (GH #489).
      startEditing("code", raw.indexOf("const"));
      const editor = editorIn(root);
      expect(editor.value()).toBe("const x = 1;");
      const caret = key === "ArrowRight" || key === "ArrowDown" ? editor.value().length : 0;
      editor.select(caret, caret);
      editor.keydown({ key, bubbles: true, cancelable: true });
      expect(editorIn(root).value()).toBe(raw);
      expect(node("code")!.raw).toBe(raw);
    } finally { dispose(); }
  });
});
