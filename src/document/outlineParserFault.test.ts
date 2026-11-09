// og-wasm-enter-trap: an outline verb whose parser call fails (a wasm trap in outline_apply_json,
// after which the glue has a fresh instance) is a refusal for every editor, not an exception: the
// document is untouched, nothing is marked dirty or pushed to undo, the user is told once, and the
// next gesture works. Before, Enter threw out of splitBlock (the textarea editor then left the
// caret in the original block with no message; CM6 also inserted the Enter into its text).
import { afterEach, beforeAll, expect, it, vi } from "vitest";

const wasm = vi.hoisted(() => ({ failNextApply: false, garbleNextApply: false, calls: {} as Record<string, number> }));
const writes = vi.hoisted(() => ({ document: 0 }));
// Every store write, counted at the one setter all document edits go through.
vi.mock("./model", async (importOriginal) => {
  const real = await importOriginal<typeof import("./model")>();
  return { ...real, setDoc: Object.assign((...args: unknown[]) => { writes.document++; return (real.setDoc as (...a: unknown[]) => unknown)(...args); }, real.setDoc) };
});
vi.mock("../render/wasm/lsdoc_wasm.js", async (importOriginal) => {
  const real = await importOriginal<typeof import("../render/wasm/lsdoc_wasm.js")>();
  // The doors an outline verb uses, counted.
  const counted = (name: "raw_byte_of_editor" | "outline_reads_json") => (...args: never[]) => {
    wasm.calls[name] = (wasm.calls[name] ?? 0) + 1;
    return (real[name] as (...a: never[]) => unknown)(...args);
  };
  return {
    ...real,
    raw_byte_of_editor: counted("raw_byte_of_editor"),
    outline_reads_json: counted("outline_reads_json"),
    outline_apply_json: (request: string) => {
      wasm.calls.outline_apply_json = (wasm.calls.outline_apply_json ?? 0) + 1;
      if (wasm.failNextApply) {
        wasm.failNextApply = false;
        throw new Error("lsdoc-wasm trap in outline_apply_json: Out of bounds memory access [instance: 1 calls, 0 resets, 25 pages; args string:400]");
      }
      if (wasm.garbleNextApply) {
        wasm.garbleNextApply = false;
        return "{not json";
      }
      return real.outline_apply_json(request);
    },
  };
});

import { initParser } from "../render/parse";
import { exitMultilineBlock, indentBlock, isDirty, pageByName, resetStore, setRaw, splitBlock, undo, undoTopTag } from ".";
import { doc } from "./model";
import { loadSingle } from "./workingSet";
import { setToasts, toasts } from "../toasts";

beforeAll(async () => { await initParser(); });
afterEach(() => { wasm.failNextApply = false; wasm.garbleNextApply = false; wasm.calls = {}; resetStore(); setToasts([]); });

function load() {
  loadSingle({ name: "Third", title: "Third", kind: "page", pre_block: null, format: "md",
    blocks: [{ id: "first", raw: "first", collapsed: false, children: [] },
      { id: "last", raw: "last block", collapsed: false, children: [] }] });
}
const failures = () => toasts().filter((t) => t.kind === "error" && t.message.includes("lsdoc-wasm trap in outline_apply_json"));

it("a failed Enter changes nothing, says so, and leaves the next Enter and Tab working", () => {
  load();
  wasm.failNextApply = true;
  expect(() => splitBlock("last", "last block".length)).not.toThrow();
  expect(pageByName("Third")!.roots).toEqual(["first", "last"]);
  expect(doc.byId.last.raw).toBe("last block");
  expect(isDirty("Third")).toBe(false);
  expect(failures()).toHaveLength(1);
  // Nothing was pushed to undo for the refused edit.
  undo();
  expect(pageByName("Third")!.roots).toEqual(["first", "last"]);

  splitBlock("last", "last block".length);
  const roots = pageByName("Third")!.roots;
  expect(roots).toHaveLength(3);
  indentBlock(roots[2], 0);
  expect(doc.byId.last.children).toEqual([roots[2]]);
  expect(doc.byId.last.raw).toBe("last block");
});

it("a failed Tab changes nothing and says so", () => {
  load();
  wasm.failNextApply = true;
  expect(() => indentBlock("last", 0)).not.toThrow();
  expect(pageByName("Third")!.roots).toEqual(["first", "last"]);
  expect(doc.byId.first.children).toEqual([]);
  expect(failures()).toHaveLength(1);
});

it("only a failed parser call is a parser fault: a bug in reading its answer still throws", () => {
  load();
  wasm.garbleNextApply = true;
  expect(() => splitBlock("last", "last block".length)).toThrow(SyntaxError);
  expect(failures()).toHaveLength(0);
});

const parserDoors = () => ({ offset: wasm.calls.raw_byte_of_editor ?? 0, reads: wasm.calls.outline_reads_json ?? 0, apply: wasm.calls.outline_apply_json ?? 0 });

it("a refused Enter makes no document mutation, and one that succeeds asks the parser once per door", () => {
  load();
  wasm.failNextApply = true;
  writes.document = 0; wasm.calls = {};
  splitBlock("last", "last block".length);
  // The refusal wrote nothing: no store write, no undo entry, nothing to save.
  expect(writes.document).toBe(0);
  expect(undoTopTag()).toBe(null);
  expect(isDirty("Third")).toBe(false);
  // It did ask: the offset, the read set, and the failing apply (each once).
  expect(parserDoors()).toEqual({ offset: 1, reads: 1, apply: 1 });

  writes.document = 0; wasm.calls = {};
  splitBlock("last", "last block".length);
  // The same three doors, once each (measured on 492f6699f): the guard, the toast and the refusal path add no call.
  expect(parserDoors()).toEqual({ offset: 1, reads: 1, apply: 1 });
  expect(writes.document).toBe(1);
});

it("a pending edit is written only once the kernel has answered, and the kernel splits the text it leaves", () => {
  load();
  const write = vi.fn(() => setRaw("last", " block", { timetracking: false }));
  // `last` selected in `last block`, Enter: the text the selection leaves is ` block`.
  wasm.failNextApply = true;
  writes.document = 0;
  splitBlock("last", 0, false, false, null, undefined, { raw: " block", write });
  expect(write).not.toHaveBeenCalled();
  expect(writes.document).toBe(0);
  expect(doc.byId.last.raw).toBe("last block");
  expect(undoTopTag()).toBe(null);

  splitBlock("last", 0, false, false, null, undefined, { raw: " block", write });
  expect(write).toHaveBeenCalledTimes(1);
  expect(pageByName("Third")!.roots.map((id) => doc.byId[id].raw)).toEqual(["first", "", " block"]);
  expect(undoTopTag()).toBe("split");
});

// The multiline exit (double Enter out of a code, calc, fence, display-math or page-properties block) is one
// gesture: the trim of the trailing blank line is written, and the sibling inserted, only once the kernel
// has answered about the insert. A refusal or a parser failure writes nothing (the trim used to be
// committed before the insert was asked, and a failed insert kept it).
it("a failed multiline exit writes no trim, enters no undo unit and keeps the block's node; a retry exits in one undo step", () => {
  loadSingle({ name: "Third", title: "Third", kind: "page", pre_block: null, format: "md",
    blocks: [{ id: "first", raw: "first", collapsed: false, children: [] },
      { id: "last", raw: "```js\nconst x = 1\n\n```", collapsed: false, children: [] }] });
  const slot = doc.byId.last;
  const trimmed = "```js\nconst x = 1\n```";
  const write = vi.fn(() => setRaw("last", trimmed, { timetracking: false }));
  wasm.failNextApply = true;
  writes.document = 0; wasm.calls = {};
  expect(exitMultilineBlock("last", { raw: trimmed, write })).toBeNull();
  // Nothing was written, pushed to undo, marked dirty, or replaced; the user is told once.
  expect(write).not.toHaveBeenCalled();
  expect(writes.document).toBe(0);
  expect(undoTopTag()).toBe(null);
  expect(isDirty("Third")).toBe(false);
  expect(doc.byId.last).toBe(slot);
  expect(doc.byId.last.raw).toBe("```js\nconst x = 1\n\n```");
  expect(pageByName("Third")!.roots).toEqual(["first", "last"]);
  expect(failures()).toHaveLength(1);
  // The kernel was asked once for the insert and nothing else.
  expect(parserDoors()).toEqual({ offset: 0, reads: 0, apply: 1 });

  const sibling = exitMultilineBlock("last", { raw: trimmed, write });
  expect(write).toHaveBeenCalledTimes(1);
  expect(doc.byId.last.raw).toBe(trimmed);
  expect(pageByName("Third")!.roots).toEqual(["first", "last", sibling]);
  expect(doc.byId[sibling!].raw).toBe("");
  expect(undoTopTag()).toBe("multiline-exit:last");
  // One Undo: the blank line is back and the sibling is gone.
  undo();
  expect(doc.byId.last.raw).toBe("```js\nconst x = 1\n\n```");
  expect(pageByName("Third")!.roots).toEqual(["first", "last"]);
});
