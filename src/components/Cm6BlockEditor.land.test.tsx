import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { editingId, endEdit, startEditing } from "../editorController";
import { node, pageByName, resetStore, selectedIds } from "../document";
import { loadSingle } from "../document/workingSet";
import { setDoc } from "../document/model";
import { editorIn } from "../tests/editorDriver";
import { focusedEditorCommandBridge } from "../editorCommandBridge";
import { installKeybindings } from "../keybindings";
import * as clipboard from "../clipboard";

let dispose: (() => void) | undefined;
beforeEach(() => {
  // These directly mount CM6 even in the default-off suite. jsdom has no
  // Range layout; match the empty geometry used by the switch-on census.
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
afterEach(() => {
  vi.restoreAllMocks();
  dispose?.(); dispose = undefined;
  endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
});

it("keeps a plain cut with no selection from deleting CM's current line", () => {
  const { editor } = mount("first\nsecond");
  editor.select(8);
  const setData = vi.fn();
  const event = new Event("cut", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: { setData, clearData() {} } });
  editor.element.dispatchEvent(event);
  expect(editor.value()).toBe("first\nsecond");
  expect(node("land").raw).toBe("first\nsecond");
  expect(setData).not.toHaveBeenCalled();
});

it.each(["typing", "read-only", "replacement graph"])("retires an async paste-as-link after %s", async change => {
  const { editor } = mount("original");
  editor.select(0, 8);
  let resolve!: (text: string) => void;
  vi.spyOn(clipboard, "readClipboardText").mockImplementation(() => new Promise<string>(done => { resolve = done; }));
  expect(focusedEditorCommandBridge()!.dispatch("editor/paste-as-link")).toBe(true);
  if (change === "typing") editor.type("new input");
  else if (change === "read-only") setDoc("pages", page => page.name === "Land", "readOnly", true);
  else {
    resetStore();
    loadSingle({ name: "Land", title: "Land", kind: "page", pre_block: null,
      blocks: [{ id: "land", raw: "replacement", collapsed: false, children: [] }] });
  }
  resolve("https://example.org");
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  expect(node("land").raw).toBe(change === "typing" ? "new input" : change === "read-only" ? "original" : "replacement");
});

function mount(raw = "visible\nid:: retained") {
  loadSingle({ name: "Land", title: "Land", kind: "page", pre_block: null,
    blocks: [{ id: "land", raw, collapsed: false, children: [] }] });
  startEditing("land", 0);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Cm6BlockEditor id="land" surface="main" scope={null} />, root);
  const editor = editorIn(root);
  const view = EditorView.findFromDOM(editor.element)!;
  return { editor, view };
}

it("edits through the document path while retaining hidden properties, including undo", () => {
  const uninstall = installKeybindings();
  try {
    const { editor } = mount();
    expect(editor.value()).toBe("visible");
    editor.select(7); editor.type(" 中文😀");
    expect(node("land").raw).toBe("visible 中文😀\nid:: retained");
    editor.keydown({ key: "z", ctrlKey: true });
    expect(node("land").raw).toBe("visible\nid:: retained");
  } finally { uninstall(); }
});

it("refuses document mutations when the page becomes read-only", () => {
  const { editor } = mount("unchanged");
  setDoc("pages", page => page.name === "Land", "readOnly", true);
  editor.input("refused");
  editor.keydown({ key: "Enter" });
  expect(node("land").raw).toBe("unchanged");
  expect(pageByName("Land")!.roots).toEqual(["land"]);
});

it("does not apply detached transactions to a replacement graph with the same block id", () => {
  const { view } = mount("old graph");
  resetStore();
  loadSingle({ name: "Land", title: "Land", kind: "page", pre_block: null,
    blocks: [{ id: "land", raw: "replacement", collapsed: false, children: [] }] });
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "stale edit" } });
  expect(node("land").raw).toBe("replacement");
});

it("routes mounted replacements through the document only for the requesting surface", () => {
  const { editor } = mount();
  const bridge = focusedEditorCommandBridge()!;
  const container = editor.element.closest<HTMLElement>(".cm6-block-editor")!;
  expect(bridge.replaceText?.("wrong surface", document.createElement("div"))).toBe(false);
  expect(node("land").raw).toBe("visible\nid:: retained");
  expect(bridge.replaceText?.("replacement", container)).toBe(true);
  expect(node("land").raw).toBe("replacement\nid:: retained");
  expect(editor.selection()).toEqual([11, 11]);
  editor.compositionStart();
  expect(bridge.replaceText?.("preedit overwrite", container)).toBe(false);
  editor.compositionEnd();
  setDoc("pages", page => page.name === "Land", "readOnly", true);
  expect(bridge.replaceText?.("read-only overwrite", container)).toBe(false);
  expect(node("land").raw).toBe("replacement\nid:: retained");
});

it("honors a remapped structural command after the global capture handler", () => {
  const { editor } = mount("text");
  const uninstall = installKeybindings({ "editor/select-all": "alt+q" });
  try {
    editor.select(0, 4, "backward");
    const event = editor.keydown({ key: "q", altKey: true });
    expect(event.defaultPrevented).toBe(true);
    expect(node("land").raw).toBe("text");
    expect(selectedIds()).toEqual(["land"]);
    expect(editingId()).toBeNull();
  } finally { uninstall(); }
});
