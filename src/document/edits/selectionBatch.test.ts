// master b3fc9c813 (F1): selection indent/outdent is ONE store publication however
// many roots are selected, and the page ends dirty. The counter sits at the real
// boundary (`setDoc`), so the old per-root loop (N moves + writeCollapsed) fails it.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const counts = { publications: 0 };
vi.mock("../model", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../model")>();
  const setDoc = ((...args: unknown[]) => {
    counts.publications++;
    return (actual.setDoc as (...a: unknown[]) => unknown)(...args);
  }) as typeof actual.setDoc;
  return { ...actual, setDoc };
});

import { initParser } from "../../render/parse";
import {
  extendSelectionTo, isDirty, indentSelection, loadFeed, outdentSelection, redo, resetStore, selectBlock, selectedIds, undo,
} from "..";
import { doc, setDoc } from "../model";
import type { BlockDto, PageDto } from "../../types";

beforeAll(() => initParser());
afterEach(() => resetStore());

const blk = (id: string, children: BlockDto[] = []): BlockDto => ({ id, raw: id, collapsed: false, children });
const load = (blocks: BlockDto[]) => {
  const page: PageDto = { name: "Test", kind: "page", title: "Test", pre_block: null, blocks };
  loadFeed([page]);
};
const state = () => JSON.parse(JSON.stringify({ pages: doc.pages, byId: doc.byId }));
function measure(run: () => void) {
  counts.publications = 0;
  run();
  return { ...counts };
}

describe("selection indent/outdent batches one command (master b3fc9c813)", () => {
  it.each([50, 200])("indents %i flat roots with one publication, undo/redo exact", (n) => {
    const roots = Array.from({ length: n }, (_, i) => blk(`s${i}`));
    load([blk("pred"), ...roots, blk("tail")]);
    selectBlock("s0");
    extendSelectionTo(`s${n - 1}`);
    const before = state();
    const selected = selectedIds();

    expect(measure(indentSelection)).toEqual({ publications: 1 });
    expect(isDirty("Test")).toBe(true);

    expect(selectedIds()).toEqual(selected);
    expect(doc.pages[0].roots).toEqual(["pred", "tail"]);
    expect(doc.byId["pred"].children).toEqual(roots.map((r) => r.id));
    expect(roots.every((r) => doc.byId[r.id].parent === "pred")).toBe(true);
    const after = state();
    undo();
    expect(state()).toEqual(before);
    redo();
    expect(state()).toEqual(after);
  });

  it.each([50, 200])("outdents %i children with one publication, undo/redo exact", (n) => {
    const kids = Array.from({ length: n }, (_, i) => blk(`s${i}`));
    load([blk("parent", kids), blk("tail")]);
    selectBlock("s0");
    extendSelectionTo(`s${n - 1}`);
    const before = state();

    expect(measure(outdentSelection)).toEqual({ publications: 1 });

    expect(doc.pages[0].roots).toEqual(["parent", ...kids.map((k) => k.id), "tail"]);
    expect(doc.byId["parent"].children).toEqual([]);
    const after = state();
    undo();
    expect(state()).toEqual(before);
    redo();
    expect(state()).toEqual(after);
  });

  it("indent removes roots from different sibling arrays once and inserts them in order", () => {
    load([blk("branch", [blk("dest"), blk("child")]), blk("later"), blk("tail")]);
    selectBlock("child");
    extendSelectionTo("later");

    expect(measure(indentSelection)).toEqual({ publications: 1 });

    expect(doc.byId["branch"].children).toEqual(["dest"]);
    expect(doc.pages[0].roots).toEqual(["branch", "tail"]);
    expect(doc.byId["dest"].children).toEqual(["child", "later"]);
  });

  it("indent keeps every descendant and expands a collapsed new parent with the move", () => {
    const sel = Array.from({ length: 5 }, (_, i) => blk(`s${i}`, [blk(`c${i}a`), blk(`c${i}b`)]));
    load([{ ...blk("pred"), collapsed: true, raw: "pred\ncollapsed:: true" }, ...sel]);
    selectBlock("s0");
    extendSelectionTo("s4");

    expect(measure(indentSelection)).toEqual({ publications: 1 });

    expect(doc.byId["pred"].collapsed).toBe(false);
    expect(doc.byId["pred"].raw).not.toMatch(/collapsed::/);
    expect(doc.byId["s3"].children).toEqual(["c3a", "c3b"]);
    expect(doc.byId["c3a"].parent).toBe("s3");
  });

  it("a stale malformed selection stays a whole no-op, never a committed prefix", () => {
    load([blk("pred"), blk("a"), blk("b")]);
    selectBlock("a");
    extendSelectionTo("b");
    setDoc("byId", "b", "parent", "pred"); // malformed: b claims a parent whose children do not list it
    const before = state();

    expect(measure(indentSelection).publications).toBe(0);
    expect(state()).toEqual(before);
  });
});
