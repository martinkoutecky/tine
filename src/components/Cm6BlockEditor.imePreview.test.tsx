// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { endEdit, startEditing } from "../editorController";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { PreviewWidget } from "../editor/cm6PreviewWidget";

// GH #767: while a Chinese input method composed in a block, the bold text after the caret vanished until the
// editor was left. CodeMirror rebuilds the composed line, draws the same widget again and then destroys the
// element it replaced; the widget disposed its one Solid root, which emptied the element still on screen.

afterEach(() => { endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; });
const RAW = "我们先在这里写**重要的内容** 然后继续";
const BOLD = "重要的内容";
const CARET = RAW.indexOf("这");
const tick = () => new Promise(r => setTimeout(r, 0));

function mount() {
  loadSingle({ name: "IME", title: "IME", kind: "page", pre_block: null,
    blocks: [{ id: "ime", raw: RAW, collapsed: false, children: [] }] });
  startEditing("ime", CARET);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Cm6BlockEditor id="ime" surface="main" scope={null} />, root);
  const view = EditorView.findFromDOM(root.querySelector('[data-block-editor="cm6"]') as HTMLElement)!;
  return { view, dispose };
}

/** Replace the preedit in the composed text node, as the browser does, and let CodeMirror observe it. */
async function preedit(view: EditorView, at: number, prevLen: number, text: string) {
  const { node, offset } = view.domAtPos(at);
  if (node.nodeType !== 3) throw new Error("no text node at the caret: " + node.nodeName);
  const t = node as Text;
  t.nodeValue = t.nodeValue!.slice(0, offset) + text + t.nodeValue!.slice(offset + prevLen);
  document.getSelection()!.collapse(t, offset + text.length);
  view.contentDOM.dispatchEvent(new InputEvent("input", { bubbles: true, isComposing: true, inputType: "insertCompositionText", data: text }));
  await tick();
  (view as unknown as { observer: { flush(): void } }).observer.flush();
}

it.each([
  ["CodeMirror redraws without a change", (view: EditorView) => view.update([])],
  ["an empty transaction is dispatched", (view: EditorView) => view.dispatch({})],
])("a bold run after the caret stays drawn through every step of a composition when %s", async (_name, redraw) => {
  const { view, dispose } = mount();
  try {
    view.focus();
    view.dispatch({ selection: { anchor: CARET } });
    await new Promise(r => setTimeout(r, 60)); // CodeMirror's focus-change update (a 10 ms timer) lands first
    const content = view.contentDOM;
    content.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "" }));
    let prev = 0;
    for (const step of ["s", "sh", "shu", "shu'r", "shu'ru", "输入"]) {
      content.dispatchEvent(new CompositionEvent("compositionupdate", { bubbles: true, data: step }));
      await preedit(view, CARET, prev, step); prev = step.length;
      redraw(view);
      expect(content.querySelector('[data-cm6-construct="bold"]')?.textContent, `after ${step}`).toBe(BOLD);
    }
    content.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "输入" }));
    await tick(); await tick();
    expect(content.querySelector('[data-cm6-construct="bold"]')?.textContent).toBe(BOLD);
  } finally { dispose(); }
});

it("destroying one drawn element of a widget leaves its other drawn element intact", () => {
  const { view, dispose } = mount();
  try {
    // The widget's own two draws, without CodeMirror in between: the contract `destroy(dom)` has to keep.
    const source = new PreviewWidget({ from: 0, to: 4, kind: "priority", label: "A" }, "[#A]", "md", null, "ime");
    const first = source.toDOM(view), second = source.toDOM(view);
    expect(first.textContent).not.toBe("");
    source.destroy(first);
    expect(first.textContent).toBe("");
    expect(second.textContent).not.toBe("");
    source.destroy(second);
    expect(second.textContent).toBe("");
  } finally { dispose(); }
});
