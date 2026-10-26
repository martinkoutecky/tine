import { afterEach, expect, it } from "vitest";
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

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.(); dispose = undefined;
  endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
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
  const { editor } = mount();
  expect(editor.value()).toBe("visible");
  editor.select(7); editor.type(" 中文😀");
  expect(node("land").raw).toBe("visible 中文😀\nid:: retained");
  editor.keydown({ key: "z", ctrlKey: true });
  expect(node("land").raw).toBe("visible\nid:: retained");
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
