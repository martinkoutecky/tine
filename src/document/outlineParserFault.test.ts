// og-wasm-enter-trap: an outline verb whose parser call fails (a wasm trap in outline_apply_json,
// after which the glue has a fresh instance) is a refusal for every editor, not an exception: the
// document is untouched, nothing is marked dirty or pushed to undo, the user is told once, and the
// next gesture works. Before, Enter threw out of splitBlock (the textarea editor then left the
// caret in the original block with no message; CM6 also inserted the Enter into its text).
import { afterEach, beforeAll, expect, it, vi } from "vitest";

const wasm = vi.hoisted(() => ({ failNextApply: false, garbleNextApply: false }));
vi.mock("../render/wasm/lsdoc_wasm.js", async (importOriginal) => {
  const real = await importOriginal<typeof import("../render/wasm/lsdoc_wasm.js")>();
  return {
    ...real,
    outline_apply_json: (request: string) => {
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
import { indentBlock, isDirty, pageByName, resetStore, splitBlock, undo } from ".";
import { doc } from "./model";
import { loadSingle } from "./workingSet";
import { setToasts, toasts } from "../toasts";

beforeAll(async () => { await initParser(); });
afterEach(() => { wasm.failNextApply = false; wasm.garbleNextApply = false; resetStore(); setToasts([]); });

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
