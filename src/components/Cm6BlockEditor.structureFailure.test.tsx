// og-wasm-enter-trap: an Enter whose outline verb failed (a parser trap in outline_apply_json)
// must leave the editor and the document as they were and say so. It used to throw out of the
// structural key listener before preventDefault, so CodeMirror's own Enter binding inserted a
// line break into the block and the next Tab indented the original block instead of a new one.
// Enter over a selection used to delete the selected text, and publish that deletion, before the kernel
// was asked: a refused split kept the deletion although the toast said nothing was changed. The
// kernel is now asked about the text the selection leaves, and the editor and the block lose the
// selected text only once it has answered.
import { afterEach, beforeEach, expect, it, onTestFinished, vi } from "vitest";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";

const wasm = vi.hoisted(() => ({ failNextApply: false, failNextOffset: false, calls: {} as Record<string, number> }));
const writes = vi.hoisted(() => ({ document: 0 }));
// Every store write the gesture makes, counted at the one setter all document edits go through.
vi.mock("../document/model", async (importOriginal) => {
  const real = await importOriginal<typeof import("../document/model")>();
  return { ...real, setDoc: Object.assign((...args: unknown[]) => { writes.document++; return (real.setDoc as (...a: unknown[]) => unknown)(...args); }, real.setDoc) };
});
vi.mock("../render/wasm/lsdoc_wasm.js", async (importOriginal) => {
  const real = await importOriginal<typeof import("../render/wasm/lsdoc_wasm.js")>();
  // Every parser call by door, to pin what a gesture asks the parser.
  const counted: Record<string, unknown> = {};
  for (const [name, value] of Object.entries(real)) {
    counted[name] = typeof value === "function" && !name.startsWith("__") && name !== "default" && name !== "initSync"
      ? (...args: unknown[]) => { wasm.calls[name] = (wasm.calls[name] ?? 0) + 1; return (value as (...a: unknown[]) => unknown)(...args); }
      : value;
  }
  return {
    ...real,
    ...counted,
    outline_apply_json: (request: string) => {
      wasm.calls.outline_apply_json = (wasm.calls.outline_apply_json ?? 0) + 1;
      if (wasm.failNextApply) {
        wasm.failNextApply = false;
        // What the glue's guard throws after it reinstantiated a trapped instance.
        throw new Error("lsdoc-wasm trap in outline_apply_json: Out of bounds memory access [instance: 1 calls, 0 resets, 25 pages; args string:400]");
      }
      return real.outline_apply_json(request);
    },
    raw_byte_of_editor: (...args: Parameters<typeof real.raw_byte_of_editor>) => {
      wasm.calls.raw_byte_of_editor = (wasm.calls.raw_byte_of_editor ?? 0) + 1;
      if (wasm.failNextOffset) {
        wasm.failNextOffset = false;
        throw new Error("lsdoc-wasm trap in raw_byte_of_editor: Out of bounds memory access");
      }
      return real.raw_byte_of_editor(...args);
    },
  };
});

import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { editingId, endEdit, startEditing } from "../editorController";
import { isDirty, pageByName, resetStore, undoTopTag } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { initParser } from "../render/parse";
import { setToasts, toasts } from "../toasts";
import { installKeybindings } from "../keybindings";

let dispose: (() => void) | undefined;
beforeEach(async () => {
  await initParser();
  setToasts([]);
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
afterEach(() => {
  wasm.failNextApply = false; wasm.failNextOffset = false; wasm.calls = {};
  dispose?.(); dispose = undefined;
  endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
});

function mount(raw = "last block", caret = raw.length) {
  loadSingle({ name: "Third", title: "Third", kind: "page", pre_block: null,
    blocks: [{ id: "first", raw: "first", collapsed: false, children: [] },
      { id: "last", raw, collapsed: false, children: [] }] });
  startEditing("last", caret);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Cm6BlockEditor id="last" surface="main" scope={null} />, root);
  return EditorView.findFromDOM(root.querySelector('[data-block-editor="cm6"]') as HTMLElement)!;
}

function key(view: EditorView, init: KeyboardEventInit) {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  view.contentDOM.dispatchEvent(event);
  return event;
}
const enter = (view: EditorView) => key(view, { key: "Enter", code: "Enter", keyCode: 13 });

it("a failed Enter keeps the block and the editor text unchanged, and says so", () => {
  const view = mount();
  view.dispatch({ selection: { anchor: view.state.doc.length } });
  wasm.failNextApply = true;
  const uncaught: unknown[] = [];
  const onError = (e: ErrorEvent) => { uncaught.push(e.error); e.preventDefault(); };
  window.addEventListener("error", onError);
  let consumed = false;
  try { consumed = enter(view).defaultPrevented; } finally { window.removeEventListener("error", onError); }
  // No line break reaches the block's text, and nothing else changed.
  expect(view.state.doc.toString()).toBe("last block");
  expect(doc.byId.last.raw).toBe("last block");
  expect(pageByName("Third")!.roots).toEqual(["first", "last"]);
  expect(editingId()).toBe("last");
  expect(consumed).toBe(true);
  expect(uncaught).toEqual([]);
  // I-9: the failure is shown, once.
  expect(toasts().filter((t) => t.kind === "error" && t.message.includes("lsdoc-wasm trap in outline_apply_json"))).toHaveLength(1);
});

it("after a failed Enter, Tab indents the block as typed and a retried Enter splits it", () => {
  const uninstall = installKeybindings();
  onTestFinished(uninstall);
  const view = mount();
  view.dispatch({ selection: { anchor: view.state.doc.length } });
  wasm.failNextApply = true;
  const onError = (e: ErrorEvent) => e.preventDefault();
  window.addEventListener("error", onError);
  try { enter(view); } finally { window.removeEventListener("error", onError); }
  // Tab moves the block the caret is in, with its text as typed (no stray line from the failed Enter).
  expect(key(view, { key: "Tab", code: "Tab", keyCode: 9 }).defaultPrevented).toBe(true);
  expect(doc.byId.first.children).toEqual(["last"]);
  expect(doc.byId.last.raw).toBe("last block");
  // The parser recovered: the retried Enter splits, and the new block takes the caret.
  view.dispatch({ selection: { anchor: view.state.doc.length } });
  expect(enter(view).defaultPrevented).toBe(true);
  expect(doc.byId.first.children).toHaveLength(2);
  expect(doc.byId.last.raw).toBe("last block");
  expect(editingId()).toBe(doc.byId.first.children[1]);
});

it("a structural command that throws before the kernel answers still owns its key", () => {
  const view = mount();
  view.dispatch({ selection: { anchor: view.state.doc.length } });
  wasm.failNextOffset = true;
  const uncaught: unknown[] = [];
  const onError = (e: ErrorEvent) => { uncaught.push(e.error); e.preventDefault(); };
  window.addEventListener("error", onError);
  let consumed = false;
  try { consumed = enter(view).defaultPrevented; } finally { window.removeEventListener("error", onError); }
  expect(view.state.doc.toString()).toBe("last block");
  expect(doc.byId.last.raw).toBe("last block");
  expect(pageByName("Third")!.roots).toEqual(["first", "last"]);
  expect(consumed).toBe(true);
  // The error itself stays uncaught, for the flight recorder, as before.
  expect(String((uncaught[0] as Error)?.message)).toContain("lsdoc-wasm trap in raw_byte_of_editor");
});

// Enter over a selection (og-wasm-enter-trap round 2). `last` is selected in `last block`; the parser
// fails. The gesture is refused whole: the text, the selection, the block, undo, the dirty bit and
// the store are exactly as they were.
const selectLast = (view: EditorView) => view.dispatch({ selection: { anchor: 0, head: 4 } });
const parserCalls = () => ({
  offset: wasm.calls.raw_byte_of_editor ?? 0, reads: wasm.calls.outline_reads_json ?? 0, apply: wasm.calls.outline_apply_json ?? 0,
  clock: wasm.calls.logbook_apply_marker_transition ?? 0,
});

it("a failed Enter over a selection keeps the selected text, in the editor and in the block", () => {
  const view = mount();
  selectLast(view);
  writes.document = 0; wasm.calls = {};
  wasm.failNextApply = true;
  const onError = (e: ErrorEvent) => e.preventDefault();
  window.addEventListener("error", onError);
  let consumed = false;
  try { consumed = enter(view).defaultPrevented; } finally { window.removeEventListener("error", onError); }
  expect(view.state.doc.toString()).toBe("last block");
  expect(view.state.selection.main).toMatchObject({ from: 0, to: 4 });
  expect(doc.byId.last.raw).toBe("last block");
  expect(pageByName("Third")!.roots).toEqual(["first", "last"]);
  expect(editingId()).toBe("last");
  expect(consumed).toBe(true);
  // A refusal makes no document mutation at all: no store write, nothing to save, nothing to undo.
  expect(writes.document).toBe(0);
  expect(isDirty("Third")).toBe(false);
  expect(undoTopTag()).toBe(null);
  expect(toasts().filter((t) => t.kind === "error" && t.message.includes("lsdoc-wasm trap in outline_apply_json"))).toHaveLength(1);
});

it("after a failed Enter over a selection, the retried Enter splits the text the selection leaves", () => {
  const view = mount();
  selectLast(view);
  wasm.failNextApply = true;
  const onError = (e: ErrorEvent) => e.preventDefault();
  window.addEventListener("error", onError);
  try { enter(view); } finally { window.removeEventListener("error", onError); }
  expect(enter(view).defaultPrevented).toBe(true);
  // The selected word is gone from the block and the editor; the rest of the text moved on or stayed.
  const texts = pageByName("Third")!.roots.map((id) => doc.byId[id].raw);
  expect(texts.join("|")).not.toContain("last");
  expect(texts.join("|")).toContain("block");
  expect(texts).toHaveLength(3);
});

it("Enter over a selection asks the parser what it asked before, once each, and writes only once it answers", () => {
  const view = mount();
  selectLast(view);
  writes.document = 0; wasm.calls = {};
  enter(view);
  // The same doors, once each, and the same two store writes (the text, the split) as when the
  // deletion was published before the kernel was asked (measured on 492f6699f).
  expect(parserCalls()).toEqual({ offset: 1, reads: 1, apply: 1, clock: 1 });
  expect(writes.document).toBe(2);
  const roots = pageByName("Third")!.roots;
  expect(roots.map((id) => doc.byId[id].raw)).toEqual(["first", "", " block"]);
  expect(undoTopTag()).toBe("split");
  // Splitting at the head puts an empty block before the original, which keeps the text the selection
  // left; the caret goes to the new block.
  expect(roots[2]).toBe("last");
  expect(editingId()).toBe(roots[1]);
});
