import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import byteCases from "../../tests/fixtures/mixed-code-edit.json";
import { SheetCellContext } from "../sheet/context";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { endEdit, startEditing } from "../editorController";
import { node, resetStore, undo } from "../document";
import { loadSingle } from "../document/workingSet";

beforeAll(initParser);
afterEach(() => { endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; });
const prefix = "préface 🐈\n```js\n", suffix = "\n```\nuntouched  \n~~~py\nother\n~~~";
const raw = prefix + "one\ntwo" + suffix;
function mount(text = raw, caret = prefix.length, format: "md" | "org" = "md", sheet = false) {
  loadSingle({ name: "Mixed", title: "Mixed", kind: "page", format, pre_block: null,
    blocks: [{ id: "mixed", raw: text, collapsed: false, children: [] }] });
  startEditing("mixed", caret);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <SheetCellContext.Provider value={sheet ? { gridId: "grid", row: 0, col: 0 } : null}><Block id="mixed" /></SheetCellContext.Provider>, root);
  return { ta: root.querySelector("textarea")!, dispose };
}
function key(ta: HTMLTextAreaElement, name: string, shiftKey = false) {
  ta.dispatchEvent(new KeyboardEvent("keydown", { key: name, shiftKey, bubbles: true, cancelable: true }));
}
function paste(ta: HTMLTextAreaElement, text: string) {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: { files: [], types: ["text/plain"], getData: (kind: string) => kind === "text/plain" ? text : "" } });
  ta.dispatchEvent(event);
}
it("body paste preserves every outside byte (paired native outline reparse, I-2)", async () => {
  const { ta, dispose } = mount();
  try {
    expect(ta.value).toBe("one\ntwo");
    ta.setSelectionRange(0, ta.value.length); paste(ta, "- literal\n  code");
    await Promise.resolve();
    const next = node("mixed")!.raw;
    expect(next).toBe(prefix + "- literal\n  code" + suffix);
    undo(); await Promise.resolve();
    expect(node("mixed")!.raw).toBe(raw); expect(ta.value).toBe("one\ntwo");
  } finally { dispose(); }
});
it("Backspace at the body start returns to raw without deleting prose or a wrapper", () => {
  const { ta, dispose } = mount();
  try {
    ta.setSelectionRange(0, 0); key(ta, "Backspace");
    expect(ta.value).toBe(raw); expect(node("mixed")!.raw).toBe(raw);
    expect(ta.selectionStart).toBe(prefix.length - 1);
    ta.setSelectionRange(prefix.length + 2, prefix.length + 2);
    ta.dispatchEvent(new Event("select", { bubbles: true }));
    expect(ta.value).toBe("one\ntwo"); expect(ta.selectionStart).toBe(2);
  } finally { dispose(); }
});
it("Shift+Arrow can select across a body boundary and raw paste remains one edit", async () => {
  const { ta, dispose } = mount();
  try {
    ta.setSelectionRange(0, 0); key(ta, "ArrowLeft", true);
    expect(ta.value).toBe(raw); expect(ta.selectionEnd).toBe(prefix.length);
    // Extend the native selection from prose into the code body, then raw paste.
    ta.setSelectionRange(0, prefix.length + 3); paste(ta, "replacement\nline");
    await Promise.resolve();
    expect(node("mixed")!.raw).toBe("replacement\nline\ntwo" + suffix);
    undo(); expect(node("mixed")!.raw).toBe(raw);
  } finally { dispose(); }
});
it.each(["ArrowLeft", "ArrowUp", "ArrowRight", "ArrowDown"])("%s leaves the fence at its body boundary", name => {
  const { ta, dispose } = mount();
  try {
    const at = name === "ArrowLeft" || name === "ArrowUp" ? 0 : ta.value.length;
    ta.setSelectionRange(at, at); key(ta, name);
    expect(ta.value).toBe(raw); expect(node("mixed")!.raw).toBe(raw);
  } finally { dispose(); }
});
it("an empty mixed fence never deletes the containing block", () => {
  const text = "before\n```\n```\nafter";
  const { ta, dispose } = mount(text, "before\n```\n".length);
  try { expect(ta.value).toBe(""); key(ta, "Backspace"); expect(node("mixed")!.raw).toBe(text); }
  finally { dispose(); }
});
it("Org source body edits retain the surrounding bytes", () => {
  const open = "intro\n#+BEGIN_SRC python\n", close = "\n#+END_SRC\noutro";
  const { ta, dispose } = mount(open + "x" + close, open.length, "org");
  try {
    expect(ta.value).toBe("x"); ta.value = "y";
    ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "y" }));
    expect(node("mixed")!.raw).toBe(open + "y" + close);
  } finally { dispose(); }
});

it("first paste into an empty mixed body keeps the structural newline out of the editor", async () => {
  const open = "before\n```\n", close = "```\nafter";
  const { ta, dispose } = mount(open + close, open.length);
  try {
    paste(ta, "x\ny"); await Promise.resolve();
    expect(ta.value).toBe("x\ny"); expect(node("mixed")!.raw).toBe(open + "x\ny\n" + close);
  } finally { dispose(); }
});
it("paste of fence-like text never rewrites the captured suffix", async () => {
  const { ta, dispose } = mount();
  try {
    ta.setSelectionRange(0, ta.value.length); paste(ta, "```\nnew"); await Promise.resolve();
    expect(node("mixed")!.raw).toBe(prefix + "```\nnew" + suffix);
    expect(ta.value).toBe("```\nnew");
    ta.value += "er"; ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "r" }));
    expect(node("mixed")!.raw).toBe(prefix + "```\nnewer" + suffix);
  } finally { dispose(); }
});

it.each(byteCases)("edits the shared $format native byte fixture through Block input", c => {
  const { ta, dispose } = mount(c.prefix + c.body + c.suffix, ("caret" in c ? c.caret : c.prefix.length) as number, c.format as "md" | "org");
  try {
    expect(ta.value).toBe(c.body); ta.value = c.replacement;
    ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertFromPaste" }));
    expect(node("mixed")!.raw).toBe(c.prefix + c.replacement + c.suffix);
  } finally { dispose(); }
});

it("sheet cells keep their full source buffer even when the caret is in a fence", () => {
  const { ta, dispose } = mount(raw, prefix.length, "md", true);
  try {
    expect(ta.value).toBe(raw);
    ta.setSelectionRange(prefix.length, prefix.length); ta.dispatchEvent(new Event("select", { bubbles: true }));
    expect(ta.value).toBe(raw);
  } finally { dispose(); }
});

it.each(["md", "org"] as const)("a %s code edit preserves hidden metadata at its original position", format => {
  const meta = format === "md" ? "id:: aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
    : ":PROPERTIES:\n:id: aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n:END:";
  const open = format === "md" ? "```js" : "#+BEGIN_SRC js", close = format === "md" ? "```" : "#+END_SRC";
  const text = `intro\n${meta}\n${open}\nx\n${close}\noutro`;
  const { ta, dispose } = mount(text, `intro\n${open}\n`.length, format);
  try {
    expect(ta.value).toBe("x"); ta.value = "edited";
    ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "d" }));
    expect(node("mixed")!.raw).toBe(text.replace("\nx\n", "\nedited\n"));
  } finally { dispose(); }
});
