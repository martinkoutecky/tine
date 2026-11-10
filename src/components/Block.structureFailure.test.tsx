// The textarea editor's counterpart of Cm6BlockEditor.structureFailure.test.tsx (og-wasm-enter-trap
// round 2): Enter over a selection whose structural step fails (a parser trap in outline_apply_json)
// is refused whole. The textarea's Enter deletes the selection and splits at its start, as OG and the
// CodeMirror editor do (og-enter-family-1109); the deletion is staged and written only once the kernel
// has answered, so this pins that a failed split leaves the text, the selection, the block, undo, the
// dirty bit and the store as they were, and that a retry works.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

const wasm = vi.hoisted(() => ({ failNextApply: false }));
const writes = vi.hoisted(() => ({ document: 0 }));
vi.mock("../document/model", async (importOriginal) => {
  const real = await importOriginal<typeof import("../document/model")>();
  return { ...real, setDoc: Object.assign((...args: unknown[]) => { writes.document++; return (real.setDoc as (...a: unknown[]) => unknown)(...args); }, real.setDoc) };
});
vi.mock("../render/wasm/lsdoc_wasm.js", async (importOriginal) => {
  const real = await importOriginal<typeof import("../render/wasm/lsdoc_wasm.js")>();
  return {
    ...real,
    outline_apply_json: (request: string) => {
      if (wasm.failNextApply) {
        wasm.failNextApply = false;
        throw new Error("lsdoc-wasm trap in outline_apply_json: Out of bounds memory access [instance: 1 calls, 0 resets, 25 pages; args string:400]");
      }
      return real.outline_apply_json(request);
    },
  };
});

import { Editor } from "./Block";
import { editingId, endEdit, startEditing } from "../editorController";
import { isDirty, pageByName, resetStore, undoTopTag } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { initParser } from "../render/parse";
import { setToasts, toasts } from "../toasts";
import { editorIn } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";

// The CodeMirror engine has its own file; under the CM6 census this half would silently run CM6.
describe("textarea editor: a failed Enter over a selection", () => {
  useEditorKind("textarea");
  let dispose: (() => void) | undefined;
  beforeEach(async () => { await initParser(); setToasts([]); });
  afterEach(() => {
    wasm.failNextApply = false;
    dispose?.(); dispose = undefined;
    endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
  });

  function mount() {
    loadSingle({ name: "Third", title: "Third", kind: "page", pre_block: null,
      blocks: [{ id: "first", raw: "first", collapsed: false, children: [] },
        { id: "last", raw: "last block", collapsed: false, children: [] }] });
    startEditing("last", 10);
    const root = document.createElement("div"); document.body.append(root);
    dispose = render(() => <Editor id="last" />, root);
    return editorIn(root);
  }
  const trapToasts = () => toasts().filter((t) => t.kind === "error" && t.message.includes("lsdoc-wasm trap in outline_apply_json"));

  it("keeps the text, the selection and the block, and says so", () => {
    const editor = mount();
    editor.select(0, 4);
    writes.document = 0;
    wasm.failNextApply = true;
    const event = editor.keydown({ key: "Enter", bubbles: true, cancelable: true });
    expect(event.defaultPrevented).toBe(true);
    expect(editor.value()).toBe("last block");
    expect(editor.selection()).toEqual([0, 4]);
    expect(doc.byId.last.raw).toBe("last block");
    expect(pageByName("Third")!.roots).toEqual(["first", "last"]);
    expect(editingId()).toBe("last");
    // A refusal makes no document mutation: no store write, nothing to save, nothing to undo.
    expect(writes.document).toBe(0);
    expect(isDirty("Third")).toBe(false);
    expect(undoTopTag()).toBe(null);
    expect(trapToasts()).toHaveLength(1);
  });

  it("lets the retried Enter split", () => {
    const editor = mount();
    editor.select(0, 4);
    wasm.failNextApply = true;
    editor.keydown({ key: "Enter", bubbles: true, cancelable: true });
    editor.select(10, 10);
    editor.keydown({ key: "Enter", bubbles: true, cancelable: true });
    const roots = pageByName("Third")!.roots;
    expect(roots).toHaveLength(3);
    expect(roots.map((id) => doc.byId[id].raw)).toEqual(["first", "last block", ""]);
  });
});
