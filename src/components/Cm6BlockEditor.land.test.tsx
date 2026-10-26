import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { endEdit, startEditing } from "../editorController";
import { node, pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { setDoc } from "../document/model";
import { editorIn } from "../tests/editorDriver";

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
