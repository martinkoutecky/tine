import { editorIn, type EditorDriver } from "../tests/editorDriver";
// C5 P4 (L10-S1, L13-S1): list-prefix editing and on-type typography never rewrite literal source.
// Real entry points: the Backspace keydown and the `input` event of the block's textarea (I-4, I-12).
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { endEdit, startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { node, pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { setAutoPairing, setTypographyMode } from "../ui";
import { Block } from "./Block";

beforeAll(initParser);
afterEach(() => {
  setTypographyMode("render"); setAutoPairing(true);
  endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
});

let seq = 0;
function mount(raw: string, format: "md" | "org" = "md") {
  const name = `Literal ${++seq}`;
  loadSingle({ name, title: name, kind: "page", format, pre_block: null, blocks: [{ id: "b", raw, collapsed: false, children: [] }] });
  startEditing("b", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <For each={pageByName(name)?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
  return { ta: editorIn(root), dispose };
}
const key = (ta: EditorDriver, name: string) =>
  ta.keydown({ key: name, bubbles: true, cancelable: true });
function typeChar(ta: EditorDriver, ch: string) {
  const caret = ta.selection()[0];
  ta.input(ta.value().slice(0, caret) + ch + ta.value().slice(caret), caret + 1, { inputType: "insertText", data: ch });
}
function backspace(ta: EditorDriver) {
  const at = ta.selection()[0], before = ta.value();
  const event = key(ta, "Backspace");
  // Synthetic textarea keys have no browser default; CM's keymap performs it.
  if (ta.kind === "textarea" && !event.defaultPrevented)
    ta.input(before.slice(0, at - 1) + before.slice(at), at - 1, { inputType: "deleteContentBackward", data: null });
}

describe("L10-S1: in-block list editing ignores code text", () => {
  it("Backspace after `+ ` in a code body removes one character, not the list prefix", () => {
    const { ta, dispose } = mount("```text\n+ keep\n```");
    try {
      // Body projection is a later CM port; either buffer uses the same source offset.
      expect(ta.value()).toBe(ta.kind === "textarea" ? "+ keep" : "```text\n+ keep\n```");
      const at = ta.value().indexOf("+ keep") + 2;
      ta.select(at, at); backspace(ta);
      expect(ta.value()).toBe(ta.kind === "textarea" ? "+keep" : "```text\n+keep\n```");
      expect(node("b")!.raw).toBe("```text\n+keep\n```");
    } finally { dispose(); }
  });
  it("Backspace after `+ ` on a fenced line of the raw view is not list editing either", () => {
    const raw = "intro\n```text\n+ keep\n```\n+ real";
    const { ta, dispose } = mount(raw);
    try {
      expect(ta.value()).toBe(raw); // mixed prose/fence stays in the raw view
      const at = raw.indexOf("+ keep") + 2;
      ta.select(at, at); backspace(ta);
      expect(node("b")!.raw).toBe(raw.replace("+ keep", "+keep"));
    } finally { dispose(); }
  });
  it("a real list line keeps its Backspace behaviour", () => {
    const raw = "intro\n+ real";
    const { ta, dispose } = mount(raw);
    try {
      const at = raw.indexOf("+ real") + 2;
      ta.select(at, at); key(ta, "Backspace");
      expect(node("b")!.raw).toBe("intro\nreal");
    } finally { dispose(); }
  });
});

describe("L13-S1: on-type typography leaves inline code alone", () => {
  it("`>` before the closer of double-backtick code", () => {
    setTypographyMode("type"); setAutoPairing(false);
    const { ta, dispose } = mount("prefix ``a-``");
    try {
      ta.select(11, 11); typeChar(ta, ">");
      expect(ta.value()).toBe("prefix ``a->``"); expect(node("b")!.raw).toBe("prefix ``a->``");
    } finally { dispose(); }
  });
  it("`>` before the closer of Org ~code~", () => {
    setTypographyMode("type"); setAutoPairing(false);
    const { ta, dispose } = mount("prefix ~a-~", "org");
    try {
      ta.select(10, 10); typeChar(ta, ">");
      expect(node("b")!.raw).toBe("prefix ~a->~");
    } finally { dispose(); }
  });
  it("closing a single-backtick span after `--` keeps the dashes", () => {
    setTypographyMode("type"); setAutoPairing(false);
    const { ta, dispose } = mount("prefix `a--");
    try {
      ta.select(11, 11); typeChar(ta, "`");
      expect(node("b")!.raw).toBe("prefix `a--`");
    } finally { dispose(); }
  });
  it("prose still gets its glyph", () => {
    setTypographyMode("type"); setAutoPairing(false);
    const { ta, dispose } = mount("`x` a-");
    try {
      ta.select(6, 6); typeChar(ta, ">");
      expect(node("b")!.raw).toBe("`x` a→");
    } finally { dispose(); }
  });
});
