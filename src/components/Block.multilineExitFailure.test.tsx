// og-wasm-enter-trap round 3: the double-Enter exit out of a code, calc, fence, display-math or
// page-properties block is one gesture. Its first half trims the trailing blank line the first
// Enter left; its second half inserts the empty sibling. When the second half fails (a parser trap
// in outline_apply_json) or is refused, the first half must not have happened: the block, the
// editor text and caret, the undo history and the mounted editor are exactly as before, and the
// "nothing was changed" toast is true. The trim used to be committed before the insert was asked,
// and a refused insert kept it (`const x = 1\n` became `const x = 1`, with no sibling).
//
// Both editors share the branch, so one table drives both;
// `useEditorKind` runs each half on its own engine.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

const wasm = vi.hoisted(() => ({ failNextApply: false, calls: {} as Record<string, number> }));
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
  };
});

import { Editor } from "./Block";
import { editingId, endEdit, startEditing } from "../editorController";
import { isDirty, pageByName, redo, resetStore, setRaw, undo, undoTopTag } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { initParser } from "../render/parse";
import { setToasts, toasts } from "../toasts";
import { editorIn, type EditorDriver, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";

/** One special block, as the first Enter leaves it. `typed` is the editor's text with the trailing
 *  blank line the first Enter added and `caret` sits on that line; `raw` is the block's text then;
 *  `exited` is the block's text once the exit has trimmed that line. */
interface ExitCase {
  kind: string;
  blocks: [id: string, raw: string][];
  id: string;
  /** The editor's text and caret after the first Enter. */
  typed: string; caret: number;
  raw: string; exited: string;
  /** Raw click offset that opens the block's code body. */
  open?: number;
}

const cases: ExitCase[] = [
  // Both editors enter a code body by its click offset, never a later selection.
  { kind: "code (whole block)", open: 6, blocks: [["first", "first"], ["target", "```js\nconst x = 1\n```"]], id: "target",
    typed: "const x = 1\n", caret: 12, raw: "```js\nconst x = 1\n\n```", exited: "```js\nconst x = 1\n```" },
  { kind: "code (in text)", blocks: [["first", "first"], ["target", "intro\n```js\nconst x = 1\n```"]], id: "target", open: 23,
    typed: "const x = 1\n", caret: 12, raw: "intro\n```js\nconst x = 1\n\n```", exited: "intro\n```js\nconst x = 1\n```" },
  { kind: "calc", blocks: [["first", "first"], ["target", "```calc\n1+1\n2+2\n```"]], id: "target",
    typed: "1+1\n2+2\n", caret: 8, raw: "```calc\n1+1\n2+2\n\n```", exited: "```calc\n1+1\n2+2\n```" },
  { kind: "fence", blocks: [["first", "first"], ["target", "intro\n```js\ncode\n```"]], id: "target",
    typed: "intro\n```js\ncode\n\n```", caret: 17, raw: "intro\n```js\ncode\n\n```", exited: "intro\n```js\ncode\n```" },
  { kind: "math", blocks: [["first", "first"], ["target", "$$\nx = 1\n$$"]], id: "target",
    typed: "$$\nx = 1\n\n$$", caret: 9, raw: "$$\nx = 1\n\n$$", exited: "$$\nx = 1\n$$" },
  { kind: "properties", blocks: [["target", "alias:: book"], ["body", "Reading list"]], id: "target",
    typed: "alias:: book\n", caret: 13, raw: "alias:: book\n", exited: "alias:: book" },
];

const failure = () => toasts().filter((t) => t.kind === "error" && t.message.includes("lsdoc-wasm trap in outline_apply_json"));
const parserCalls = () => ({
  offset: wasm.calls.raw_byte_of_editor ?? 0, reads: wasm.calls.outline_reads_json ?? 0, apply: wasm.calls.outline_apply_json ?? 0,
  clock: wasm.calls.logbook_apply_marker_transition ?? 0,
});

describe.each<EditorKind>(["textarea", "cm6"])("the double-Enter exit on the %s editor", (kind) => {
  useEditorKind(kind);
  let dispose: (() => void) | undefined;
  beforeEach(async () => { await initParser(); setToasts([]); });
  afterEach(() => {
    wasm.failNextApply = false; wasm.calls = {};
    dispose?.(); dispose = undefined;
    endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
  });

  /** Mounts the block under test and applies the first Enter's result as typed text. */
  function mount(c: ExitCase): { editor: EditorDriver; root: HTMLElement } {
    loadSingle({ name: "Third", title: "Third", kind: "page", pre_block: null, format: "md",
      blocks: c.blocks.map(([id, raw]) => ({ id, raw, collapsed: false, children: [] })) });
    startEditing(c.id, c.open ?? c.blocks.find(([id]) => id === c.id)![1].length);
    const root = document.createElement("div"); document.body.append(root);
    dispose = render(() => <Editor id={c.id} />, root);
    const editor = editorIn(root);
    editor.input(c.typed, c.caret);
    expect(doc.byId[c.id].raw, `${c.kind}: the first Enter's text reached the block`).toBe(c.raw);
    return { editor, root };
  }
  const enter = (editor: EditorDriver) => {
    const onError = (e: ErrorEvent) => e.preventDefault();
    window.addEventListener("error", onError);
    try { return editor.keydown({ key: "Enter", bubbles: true, cancelable: true }); } finally { window.removeEventListener("error", onError); }
  };
  const roots = () => pageByName("Third")!.roots;

  it.each(cases)("$kind: a failed exit changes nothing, in the editor or in the block, and says so", (c) => {
    const { editor, root } = mount(c);
    const slot = doc.byId[c.id];
    const rootsBefore = [...roots()];
    const undoBefore = undoTopTag(), dirtyBefore = isDirty("Third");
    writes.document = 0; wasm.calls = {};
    wasm.failNextApply = true;
    const event = enter(editor);
    // The key is consumed, whatever the parser did.
    expect(event.defaultPrevented).toBe(true);
    // The trailing blank line stays, in the editor and in the block, with the caret on it.
    expect(editor.value()).toBe(c.typed);
    expect(editor.selection()).toEqual([c.caret, c.caret]);
    expect(doc.byId[c.id].raw).toBe(c.raw);
    expect(roots()).toEqual(rootsBefore);
    expect(editingId()).toBe(c.id);
    // The same mounted editor, still the block's own: its node was not replaced and it still accepts edits.
    expect(doc.byId[c.id]).toBe(slot);
    expect(editorIn(root).element).toBe(editor.element);
    expect(editor.isReadOnly()).toBe(false);
    // A refusal is not a write: no store write, no undo entry (or redo lost), nothing newly dirty.
    expect(writes.document).toBe(0);
    expect(undoTopTag()).toBe(undoBefore);
    expect(isDirty("Third")).toBe(dirtyBefore);
    // I-9: the failure is shown once, and "nothing was changed" is true.
    expect(failure()).toHaveLength(1);
  });

  it.each(cases)("$kind: after a failed exit the retried Enter trims the line and opens the sibling", (c) => {
    const { editor } = mount(c);
    wasm.failNextApply = true;
    enter(editor);
    enter(editor);
    expect(doc.byId[c.id].raw).toBe(c.exited);
    const after = roots();
    expect(after).toHaveLength(c.blocks.length + 1);
    const index = after.indexOf(c.id);
    expect(doc.byId[after[index + 1]].raw).toBe("");
    expect(editingId()).toBe(after[index + 1]);
  });

  it.each(cases)("$kind: a failed exit keeps the redo history", (c) => {
    const { editor } = mount(c);
    // One undone edit to redo, in the page's other block, so the editor under test is left alone.
    const other = c.blocks.find(([id]) => id !== c.id)![0];
    setRaw(other, "edited elsewhere");
    expect(undo()).toBe(true);
    expect(doc.byId[other].raw).not.toBe("edited elsewhere");
    expect(editor.value()).toBe(c.typed);
    wasm.failNextApply = true;
    enter(editor);
    redo();
    expect(doc.byId[other].raw).toBe("edited elsewhere");
  });

  it.each(cases)("$kind: a successful exit trims the line, opens the sibling and undoes as one step", (c) => {
    const { editor } = mount(c);
    writes.document = 0; wasm.calls = {};
    expect(enter(editor).defaultPrevented).toBe(true);
    expect(doc.byId[c.id].raw).toBe(c.exited);
    const after = roots();
    const index = after.indexOf(c.id);
    expect(doc.byId[after[index + 1]].raw).toBe("");
    expect(editingId()).toBe(after[index + 1]);
    expect(undoTopTag()).toBe(`multiline-exit:${c.id}`);
    expect(failure()).toHaveLength(0);
    // One Undo restores the block as it was before the exit and removes the sibling.
    expect(undo()).toBe(true);
    expect(doc.byId[c.id].raw).toBe(c.raw);
    expect(roots()).toHaveLength(c.blocks.length);
  });

  // PC-3/PC-5: the exit asks the parser, and writes the store, what it did before the trim was staged
  // (measured on a399250b5 with this same table): the clock once for the trim, the outline apply once for
  // the insert, no offset or read-set call, and one store write for the trim plus one for the insert.
  it.each(cases)("$kind: a successful exit makes the parser and store calls it always made", (c) => {
    const { editor } = mount(c);
    writes.document = 0; wasm.calls = {};
    enter(editor);
    expect(parserCalls()).toEqual({ offset: 0, reads: 0, apply: 1, clock: 1 });
    expect(writes.document).toBe(2);
  });
});
