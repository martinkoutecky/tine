// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";

// GH #767 (Louis-15, Doubao keyboard): text typed by hand was lost after Enter on Android. CodeMirror reads
// the user agent once, at module load, so the Android Chrome identity is set before it is imported.
vi.hoisted(() => Object.defineProperty(navigator, "userAgent", { configurable: true,
  value: "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36" }));

import { For, Show } from "solid-js";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { editingId, endEdit, startEditing } from "../editorController";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { recordImeTrace } from "../editor/imeTrace";

/** Frames are held, so a test decides when CodeMirror's deferred DOM flush runs. */
let frames: FrameRequestCallback[] = [];
let dispose: (() => void) | undefined;
beforeEach(() => {
  frames = [];
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(cb => { frames.push(cb); return frames.length; });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => { if (frames[id - 1]) frames[id - 1] = () => {}; });
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  endEdit("page-navigation"); resetStore(); vi.restoreAllMocks();
  delete (window as { EditContext?: unknown }).EditContext;
  document.body.replaceChildren();
});

const settle = async () => { for (let i = 0; i < 4; i++) await Promise.resolve(); };
async function runFrames() {
  for (let round = 0; round < 4 && frames.length; round++) {
    const now = frames; frames = [];
    for (const cb of now) cb(performance.now());
    await settle();
  }
}

function mount() {
  loadSingle({ name: "Doubao", title: "Doubao", kind: "page", pre_block: null, blocks: [
    { id: "a", raw: "hello ", collapsed: false, children: [] },
    { id: "b", raw: "next", collapsed: false, children: [] }] });
  startEditing("a", 6);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <For each={pageByName("Doubao")!.roots}>{id =>
    <Show when={editingId() === id} fallback={<div class="rendered">{doc.byId[id].raw}</div>}>
      <Cm6BlockEditor id={id} surface="main" scope={null} />
    </Show>}</For>, root);
  const view = EditorView.findFromDOM(root.querySelector(".cm-content")!)!;
  view.focus();
  return view;
}

const enterKey = (view: EditorView) => view.contentDOM.dispatchEvent(new KeyboardEvent("keydown",
  { key: "Enter", code: "Enter", keyCode: 13, bubbles: true, cancelable: true }));
/** The keyboard's final composition text lands in the DOM, with the caret after it, as a browser places it. */
function lateFinalText(content: HTMLElement, value: string) {
  const text = content.querySelector(".cm-line")!.firstChild as Text;
  text.nodeValue = value;
  document.getSelection()!.collapse(text, value.length);
}
const texts = () => pageByName("Doubao")!.roots.map(id => doc.byId[id].raw);

it("M1: Enter right after the keyboard's late final composition change keeps the composed text and splits once", async () => {
  const view = mount();
  const content = view.contentDOM;
  content.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  content.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "你好" }));
  await settle();
  // Android reconciles the final composition text into the DOM after compositionend; CodeMirror holds that
  // change until its next frame. The keyboard's Enter arrives inside that frame.
  lateFinalText(content, "hello 你好");
  await settle();
  enterKey(view);
  await runFrames();
  expect(texts()).toEqual(["hello 你好", "", "next"]);
});

/** The parts of the browser's EditContext CodeMirror uses (Chrome on Android, CM 6.43). */
class FakeEditContext extends EventTarget {
  text: string; selectionStart: number; selectionEnd: number;
  constructor(init: { text?: string; selectionStart?: number; selectionEnd?: number } = {}) {
    super();
    this.text = init.text ?? ""; this.selectionStart = init.selectionStart ?? 0; this.selectionEnd = init.selectionEnd ?? 0;
  }
  updateText(from: number, to: number, text: string) { this.text = this.text.slice(0, from) + text + this.text.slice(to); }
  updateSelection(start: number, end: number) { this.selectionStart = start; this.selectionEnd = end; }
  updateSelectionBounds() {}
  updateControlBounds() {}
  updateCharacterBounds() {}
  attachedElements() { return []; }
}

/** What the keyboard sends through an EditContext: composition events and text updates on the context, never
 *  on the element. */
function composeOnContext(context: FakeEditContext, steps: string[], at: number) {
  context.dispatchEvent(new Event("compositionstart"));
  let prev = 0;
  for (const text of steps) {
    const update = Object.assign(new Event("textupdate"), { updateRangeStart: at, updateRangeEnd: at + prev, text,
      selectionStart: at + text.length, selectionEnd: at + text.length });
    context.dispatchEvent(update);
    prev = text.length;
  }
  context.dispatchEvent(new Event("compositionend"));
}

it("M3: with an EditContext, a finished composition is saved, and Enter then splits once keeping it", async () => {
  (window as { EditContext?: unknown }).EditContext = FakeEditContext;
  const view = mount();
  const context = (view.contentDOM as HTMLElement & { editContext?: unknown }).editContext as FakeEditContext;
  expect(context).toBeInstanceOf(FakeEditContext);
  composeOnContext(context, ["ni", "你好"], 6);
  await settle();
  expect(view.state.doc.toString()).toBe("hello 你好");
  expect(doc.byId.a.raw).toBe("hello 你好");
  enterKey(view);
  await runFrames();
  expect(texts()).toEqual(["hello 你好", "", "next"]);
});

it("M1 via beforeinput: an Enter the keyboard sends as insertParagraph after the late change keeps the text", async () => {
  const view = mount();
  const content = view.contentDOM;
  content.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  content.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "你好" }));
  await settle();
  const line = content.querySelector(".cm-line")!;
  lateFinalText(content, "hello 你好");
  await settle();
  content.dispatchEvent(new InputEvent("beforeinput", { inputType: "insertParagraph", bubbles: true, cancelable: true }));
  const next = document.createElement("div"); next.className = "cm-line"; next.append(document.createElement("br"));
  line.after(next);
  await settle();
  await runFrames();
  expect(texts()).toEqual(["hello 你好", "", "next"]);
});

it("the opt-in Enter trace records event names, codes and lengths, and none of the typed text", async () => {
  const view = mount();
  const content = view.contentDOM;
  const trace = recordImeTrace();
  content.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "你" }));
  content.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "你好" }));
  await settle();
  lateFinalText(content, "hello 你好");
  await settle();
  enterKey(view);
  await runFrames();
  const text = await trace;
  expect(text).toMatch(/^EditContext=false focused editor=true editContext=false doc=6 dom=6 domLines=1/);
  expect(text).toMatch(/element compositionend dataLength=2 doc=6 /);
  expect(text).toMatch(/element keydown key=Enter keyCode=13 isComposing=false/);
  expect(text).not.toMatch(/你|好|hello|next/);
  expect(texts()).toEqual(["hello 你好", "", "next"]);
});
