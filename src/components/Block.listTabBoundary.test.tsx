import { afterEach, beforeAll, beforeEach, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, pageByName } from "../document/model";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { startEditing, endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { Block } from "./Block";

beforeAll(() => initParser());
let disposeKeys: () => void;
beforeEach(() => { disposeKeys = installKeybindings(); });
afterEach(() => { disposeKeys(); endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; });

function mount(raw: string, nested = false, format: "md" | "org" = "md") {
  const item = { id: "item", raw, collapsed: false, children: [] };
  const previous = { id: "previous", raw: "previous", collapsed: false, children: nested ? [item] : [] };
  loadSingle({ name: "Lists", title: "Lists", kind: "page", pre_block: null, format,
    blocks: nested ? [previous] : [previous, item] });
  startEditing("item", raw.length);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <For each={pageByName("Lists")!.roots}>{id => <Block id={id} />}</For>, root);
  const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
  editor.setSelectionRange(raw.length, raw.length);
  return { editor, dispose };
}
function tab(editor: HTMLTextAreaElement, shiftKey = false) {
  editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey, bubbles: true, cancelable: true }));
}

it.each(["+ milk", "- milk", "* milk", "1. milk", "+ milk\nsecond line"])("Tab reparents first-line %j without changing its text (GH #632)", raw => {
  const { editor, dispose } = mount(raw);
  try {
    editor.setSelectionRange(3, 3);
    tab(editor);
    expect(doc.byId.item.parent).toBe("previous");
    expect(doc.byId.item.raw).toBe(raw);
  } finally { dispose(); }
});
it.each(["  + milk", "  - milk", "  * milk", "  1. milk"])("Shift+Tab outdents first-line %j without changing its text", raw => {
  const { editor, dispose } = mount(raw, true);
  try { tab(editor, true); expect(doc.byId.item.parent).toBeNull(); expect(doc.byId.item.raw).toBe(raw); }
  finally { dispose(); }
});
it("later list lines keep their in-block Tab and Shift+Tab nudges", () => {
  const raw = "shopping\n+ milk";
  const { editor, dispose } = mount(raw);
  try {
    tab(editor); expect(doc.byId.item.raw).toBe("shopping\n  + milk"); expect(doc.byId.item.parent).toBeNull();
    tab(editor, true); expect(doc.byId.item.raw).toBe(raw); expect(doc.byId.item.parent).toBeNull();
  } finally { dispose(); }
});
it("Org first-line lists retain their existing in-block nudge", () => {
  const { editor, dispose } = mount("+ milk", false, "org");
  try { tab(editor); expect(doc.byId.item.raw).toBe("  + milk"); expect(doc.byId.item.parent).toBeNull(); }
  finally { dispose(); }
});
it("literal list text in a fence remains an outline Tab", () => {
  const raw = "```\n+ milk\n```";
  const { editor, dispose } = mount(raw);
  try { editor.setSelectionRange(7, 7); tab(editor); expect(doc.byId.item.parent).toBe("previous"); expect(doc.byId.item.raw).toBe(raw); }
  finally { dispose(); }
});
