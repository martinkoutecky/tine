// og-wasm-enter-trap: an Enter whose outline verb failed (a parser trap in outline_apply_json)
// must leave the editor and the document as they were and say so. It used to throw out of the
// structural key listener before preventDefault, so CodeMirror's own Enter binding inserted a
// line break into the block and the next Tab indented the original block instead of a new one.
import { afterEach, beforeEach, expect, it, onTestFinished, vi } from "vitest";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";

const wasm = vi.hoisted(() => ({ failNextApply: false, failNextOffset: false }));
vi.mock("../render/wasm/lsdoc_wasm.js", async (importOriginal) => {
  const real = await importOriginal<typeof import("../render/wasm/lsdoc_wasm.js")>();
  return {
    ...real,
    outline_apply_json: (request: string) => {
      if (wasm.failNextApply) {
        wasm.failNextApply = false;
        // What the glue's guard throws after it reinstantiated a trapped instance.
        throw new Error("lsdoc-wasm trap in outline_apply_json: Out of bounds memory access [instance: 1 calls, 0 resets, 25 pages; args string:400]");
      }
      return real.outline_apply_json(request);
    },
    raw_byte_of_editor: (...args: Parameters<typeof real.raw_byte_of_editor>) => {
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
import { pageByName, resetStore } from "../document";
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
  wasm.failNextApply = false; wasm.failNextOffset = false;
  dispose?.(); dispose = undefined;
  endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
});

function mount() {
  loadSingle({ name: "Third", title: "Third", kind: "page", pre_block: null,
    blocks: [{ id: "first", raw: "first", collapsed: false, children: [] },
      { id: "last", raw: "last block", collapsed: false, children: [] }] });
  startEditing("last", 10);
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
