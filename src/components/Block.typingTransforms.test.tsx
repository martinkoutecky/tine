import { afterEach, beforeAll, beforeEach, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { blockProperty, node, pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { clearSeededFacets } from "../render/facets";
import { initParser } from "../render/parse";
import { endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { setAutoPairing, setTypographyMode } from "../ui";
import { editorIn } from "../tests/editorDriver";
import { Block } from "./Block";

beforeAll(initParser);
let dispose: (() => void) | undefined, uninstall: () => void;
beforeEach(() => { uninstall = installKeybindings(); setAutoPairing(false); setTypographyMode("render"); });
afterEach(() => {
  dispose?.(); dispose = undefined; uninstall();
  endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
  setAutoPairing(true); setTypographyMode("render");
});
function mount(raw = "", format: "md" | "org" = "md") {
  loadSingle({ name: "Typing", title: "Typing", kind: "page", pre_block: null, format,
    blocks: [{ id: "before", raw: "unchanged", collapsed: false, children: [] },
      { id: "typing", raw, collapsed: false, children: [] }] });
  clearSeededFacets();
  startEditing("typing", raw.length);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <For each={pageByName("Typing")!.roots}>{id => <Block id={id} />}</For>, root);
  return editorIn(root);
}

it.each([false, true])("pairs doubled page refs, types through and erases empty pairs with auto-pairing=%s", pairing => {
  setAutoPairing(pairing);
  const editor = mount();
  editor.type("["); editor.type("[");
  expect(editor.value()).toBe("[[]]"); expect(editor.selection()).toEqual([2, 2]);
  editor.keydown({ key: "Backspace" });
  expect(editor.value()).toBe("[]"); expect(editor.selection()).toEqual([1, 1]);
  editor.type("x"); editor.type("]");
  expect(editor.value()).toBe("[x]"); expect(editor.selection()).toEqual([3, 3]);
  expect(node("typing").raw).toBe("[x]"); expect(node("before").raw).toBe("unchanged");
});

it.each(["(", "{", '"', "`"])("opt-in %s pairing skips its closer and Backspace removes an empty pair", opener => {
  setAutoPairing(true);
  const editor = mount(), closer = opener === "(" ? ")" : opener === "{" ? "}" : opener;
  editor.type(opener);
  expect(editor.value()).toBe(opener + closer); expect(editor.selection()).toEqual([1, 1]);
  editor.keydown({ key: "Backspace" });
  expect(editor.value()).toBe("");
  editor.type(opener); editor.type("x"); editor.type(closer);
  expect(editor.value()).toBe(opener + "x" + closer); expect(editor.selection()).toEqual([3, 3]);
  expect(node("typing").raw).toBe(editor.value());
});

it("keeps opt-in openers literal when disabled and before a word", () => {
  const editor = mount("word");
  editor.select(0); editor.type("(");
  expect(editor.value()).toBe("(word");
  setAutoPairing(true); editor.select(1); editor.type("{");
  expect(editor.value()).toBe("({word");
});

it("does not transform a paste or a multi-character insertion into pairing or own numbering", () => {
  setAutoPairing(true);
  const editor = mount();
  editor.input("1. ", 3, { inputType: "insertFromPaste", data: "1. " });
  expect(editor.value()).toBe("1. "); expect(blockProperty("typing", "logseq.order-list-type")).toBeNull();
  editor.input("", 0, { inputType: "deleteContentBackward", data: null });
  editor.type("[["); expect(editor.value()).toBe("[[");
});

it.each(["md", "org"] as const)("own-numbered trigger and property removal share document history (%s)", async format => {
  let editor = mount("", format);
  for (const ch of "1. ") editor.type(ch);
  expect(editor.value()).toBe(""); expect(editor.selection()).toEqual([0, 0]);
  expect(blockProperty("typing", "logseq.order-list-type")).toBe("number");
  editor.keydown({ key: "z", ctrlKey: true });
  await Promise.resolve();
  editor = editorIn(); // document history may remount the focused editor
  expect(blockProperty("typing", "logseq.order-list-type")).toBeNull();
  expect(editor.value()).toBe("1.");
  editor.keydown({ key: "z", ctrlKey: true, shiftKey: true });
  await Promise.resolve();
  editor = editorIn();
  expect(blockProperty("typing", "logseq.order-list-type")).toBe("number");
  expect(editor.value()).toBe("");
  editor.type("item"); editor.select(0); editor.keydown({ key: "Backspace" });
  expect(node("typing").raw).toBe("item"); expect(node("before").raw).toBe("unchanged");
});

it.each([
  ["md", "+ [x] first", "+ [x] first\n+ [ ] "],
  ["md", "intro\n  9) first", "intro\n  9) first\n  10) "],
  ["org", "- [X] first", "- [X] first\n- [ ] "],
] as const)("continues and removes the whole list prefix (%s, %s)", async (format, raw, next) => {
  const editor = mount(raw, format);
  editor.keydown({ key: "Enter" });
  await Promise.resolve();
  expect(editor.value()).toBe(next); expect(editor.selection()).toEqual([next.length, next.length]);
  editor.keydown({ key: "Backspace" });
  await Promise.resolve();
  expect(editor.value()).toBe(raw + "\n"); expect(node("typing").raw).toBe(raw + "\n");
  expect(pageByName("Typing")!.roots).toEqual(["before", "typing"]);
});

it("continues a list at an interior caret and keeps Shift+Enter as a bare newline", async () => {
  const editor = mount("+ first last");
  editor.select(7); editor.keydown({ key: "Enter" });
  await Promise.resolve();
  expect(editor.value()).toBe("+ first\n+  last"); expect(editor.selection()).toEqual([10, 10]);
  editor.keydown({ key: "Enter", shiftKey: true });
  await Promise.resolve();
  expect(editor.value()).toBe("+ first\n+ \n last");
});

it.each(["render", "type"] as const)("replaces prose typography only in %s mode", mode => {
  setTypographyMode(mode);
  const editor = mount("a-"); editor.type(">");
  expect(editor.value()).toBe(mode === "type" ? "a→" : "a->");
  expect(node("typing").raw).toBe(editor.value());
});
