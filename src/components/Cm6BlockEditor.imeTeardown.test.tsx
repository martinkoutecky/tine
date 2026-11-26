// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { batch, For, Show } from "solid-js";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { editingId, endEdit, startEditing } from "../editorController";
import { redo, resetStore, setRaw, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";

// GH #767: text entered by a voice keyboard on Android was gone after a tap on another block. A tap on another
// block moves editing without a blur, so the editor is unmounted while it still holds a composition whose text
// was never committed.

let dispose: (() => void) | undefined;
beforeEach(() => { vi.spyOn(document, "hasFocus").mockReturnValue(true); });
afterEach(() => { dispose?.(); dispose = undefined; endEdit("page-navigation"); resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });

function mount() {
  loadSingle({ name: "Voice", title: "Voice", kind: "page", pre_block: null, blocks: [
    { id: "a", raw: "first ", collapsed: false, children: [] },
    { id: "b", raw: "second", collapsed: false, children: [] },
    { id: "c", raw: "third", collapsed: false, children: [] }] });
  startEditing("a", 6);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <For each={["a", "b", "c"]}>{id =>
    <Show when={editingId() === id} fallback={<div class="rendered">{doc.byId[id].raw}</div>}>
      <Cm6BlockEditor id={id} surface="main" scope={null} />
    </Show>}</For>, root);
  const view = EditorView.findFromDOM(root.querySelector(".cm-content")!)!;
  view.focus();
  return view;
}
type InputState = { composing: number; compositionFirstChange: boolean | null };
const inputState = (v: EditorView) => (v as unknown as { inputState: InputState }).inputState;

/** A composition as CodeMirror sees it through Android's EditContext: the composition events go to the
 *  EditContext object and never to the element, and each text update is a compose transaction. */
function composeThroughEditContext(v: EditorView, steps: string[], ends: boolean) {
  const state = inputState(v);
  state.composing = 0; state.compositionFirstChange = true;
  const at = v.state.doc.length; let prev = 0;
  for (const text of steps) {
    state.composing++;
    const first = state.compositionFirstChange; state.compositionFirstChange = false;
    v.dispatch({ changes: { from: at, to: at + prev, insert: text }, selection: { anchor: at + text.length },
      userEvent: first ? "input.type.compose.start" : "input.type.compose", scrollIntoView: true });
    prev = text.length;
  }
  if (ends) { state.composing = -1; state.compositionFirstChange = null; }
}

it.each([["has ended", true], ["is still open", false]])(
  "text of a composition that %s is saved when a tap on another block unmounts the editor", async (_name, ends) => {
    const view = mount();
    composeThroughEditContext(view, ["hello", "hello world"], ends);
    expect(view.state.doc.toString()).toBe("first hello world");
    startEditing("b", 0); // what a tap on another block does: no blur, the editor is unmounted
    await Promise.resolve(); await Promise.resolve();
    expect(editingId()).toBe("b");
    expect(doc.byId.a.raw).toBe("first hello world");
  });

it("a composition ended by the element's own compositionend is saved once and survives the same tap", async () => {
  const view = mount();
  view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  view.dispatch({ changes: { from: 6, insert: "hello" }, selection: { anchor: 11 }, userEvent: "input.type.compose" });
  view.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "hello" }));
  await Promise.resolve(); await Promise.resolve();
  expect(doc.byId.a.raw).toBe("first hello");
  startEditing("b", 0);
  await Promise.resolve();
  expect(doc.byId.a.raw).toBe("first hello");
});

it("an editor unmounted without a composition writes nothing", async () => {
  mount();
  const before = doc.byId.a;
  startEditing("b", 0);
  await Promise.resolve();
  expect(doc.byId.a).toBe(before);
  expect(doc.byId.a.raw).toBe("first ");
});

it("a block written since its composition began is not overwritten by the lagging view", async () => {
  const view = mount();
  composeThroughEditContext(view, ["hello world"], true);
  // What a split in the middle does: it writes this block's first part and moves editing in one batch, so the
  // editor is unmounted before its view is brought up to date.
  batch(() => { setRaw("a", "first hello", { timetracking: false }); startEditing("b", 0); });
  await Promise.resolve(); await Promise.resolve();
  expect(view.state.doc.toString()).toBe("first hello world");
  expect(doc.byId.a.raw).toBe("first hello");
});

const editorOf = (id: string) => {
  startEditing(id, doc.byId[id].raw.length);
  const view = EditorView.findFromDOM(document.querySelector(".cm-content")!)!;
  view.focus();
  return view;
};

it("Undo while a composition is held keeps what it undid redoable, and the held text is not written", async () => {
  mount();
  setRaw("b", "second!");
  composeThroughEditContext(editorOf("a"), ["hello"], false);
  expect(undo()).toBe(true);
  await Promise.resolve();
  expect(doc.byId.b.raw).toBe("second");
  expect(doc.byId.a.raw).toBe("first ");
  redo();
  expect(doc.byId.b.raw).toBe("second!");
});

it("Redo while a composition is held keeps the entries after the one it replays", async () => {
  mount();
  setRaw("b", "second!"); setRaw("c", "third!");
  undo(); undo();
  expect([doc.byId.b.raw, doc.byId.c.raw]).toEqual(["second", "third"]);
  composeThroughEditContext(editorOf("a"), ["hello"], false);
  redo();
  await Promise.resolve();
  redo();
  expect([doc.byId.b.raw, doc.byId.c.raw]).toEqual(["second!", "third!"]);
  expect(doc.byId.a.raw).toBe("first ");
});
