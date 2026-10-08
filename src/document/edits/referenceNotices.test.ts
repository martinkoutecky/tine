import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "../../render/parse";
import { deleteBlock, deleteSelection, extendSelectionTo, loadFeed, mergeWithNext, mergeWithPrev, resetStore, selectBlock, setRaw, undo } from "..";
import { doc } from "../model";
import { installReferenceChangeCounter, setToasts, toasts } from "../../toasts";
import type { BlockDto } from "../../types";

const counts = new Map<string, number>();
const a = "aaaaaaaa-0000-4000-8000-000000000001";
const b = "aaaaaaaa-0000-4000-8000-000000000002";
const c = "aaaaaaaa-0000-4000-8000-000000000003";
const block = (id: string, external?: string, children: BlockDto[] = []): BlockDto => ({ id, raw: id + (external ? `\nid:: ${external}` : ""), collapsed: false, children });
const load = (blocks: BlockDto[]) => loadFeed([{ name: "Test", kind: "page", title: "Test", pre_block: null, blocks }]);
const state = () => JSON.parse(JSON.stringify({ pages: doc.pages, byId: doc.byId }));
beforeAll(() => initParser());
beforeEach(() => {
  counts.clear(); setToasts([]);
  installReferenceChangeCounter((ids) => ids.reduce((sum, id) => sum + (counts.get(id) ?? 0), 0));
});
afterEach(() => resetStore());

describe("reference-change notices (GH #635 / #652)", () => {
  it("counts a deleted subtree and restores its exact identities with notice Undo", () => {
    load([block("root", a, [block("child", b)]), block("tail")]);
    counts.set(a, 2); counts.set(b, 3);
    const before = state();
    deleteBlock("root");
    expect(toasts().at(-1)?.message).toContain("5 references are now broken");
    expect(toasts().at(-1)?.action?.label).toBe("Undo");
    toasts().at(-1)!.action!.run();
    expect(state()).toEqual(before);
  });
  it("counts all selected roots and descendants in one ordinary Undo unit", () => {
    load([block("one", a, [block("child", b)]), block("two", c), block("tail")]);
    counts.set(a, 1); counts.set(b, 2); counts.set(c, 3);
    selectBlock("one"); extendSelectionTo("two");
    const before = state();
    deleteSelection();
    expect(toasts().at(-1)?.message).toContain("6 references are now broken");
    undo(); expect(state()).toEqual(before);
  });
  it.each(["prev", "next"])("notifies id transfer through %s merge and restores with Undo", (direction) => {
    load([block("keep"), block("gone", a, [block("child", b)])]);
    counts.set(a, 2); counts.set(b, 4);
    const before = state();
    expect(direction === "prev" ? mergeWithPrev("gone") : mergeWithNext("keep")).toBe(true);
    expect(doc.byId.keep.raw).toContain(a);
    expect(toasts().at(-1)?.message).toBe("2 references now point to this block");
    toasts().at(-1)!.action!.run(); expect(state()).toEqual(before);
  });
  it("keeps an existing survivor id and reports broken references to the absorbed id", () => {
    load([block("keep", b), block("gone", a)]); counts.set(a, 1);
    mergeWithPrev("gone");
    expect(doc.byId.keep.raw).toContain(b); expect(doc.byId.keep.raw).not.toContain(a);
    expect(toasts().at(-1)?.message).toContain("1 reference is now broken");
  });
  it("transfers even an empty block's id", () => {
    load([block("keep"), { ...block("gone", a), raw: `id:: ${a}` }]); counts.set(a, 1);
    mergeWithPrev("gone");
    expect(toasts().at(-1)?.message).toBe("1 reference now points to this block");
  });
  it("uses Org drawer ids for transfer notices and restores the drawer with Undo", () => {
    loadFeed([{ name: "Test", kind: "page", title: "Test", pre_block: null, format: "org", blocks: [
      block("keep"), { ...block("gone"), raw: `Gone\n:PROPERTIES:\n:id: ${a}\n:END:` },
    ] }]);
    counts.set(a, 2); const before = state();
    mergeWithNext("keep");
    expect(toasts().at(-1)?.message).toBe("2 references now point to this block");
    expect(doc.byId.keep.raw).toContain(`:id: ${a}`);
    toasts().at(-1)!.action!.run(); expect(state()).toEqual(before);
  });
  it("does not notify unreferenced deletion or merge", () => {
    load([block("keep"), block("gone", a), block("tail", b)]);
    mergeWithPrev("gone"); deleteBlock("tail"); expect(toasts()).toEqual([]);
  });
  it("a stale notice action cannot undo a later edit", () => {
    load([block("keep"), block("gone", a)]); counts.set(a, 1);
    deleteBlock("gone"); const action = toasts().at(-1)!.action!;
    setRaw("keep", "later"); action.run();
    expect(doc.byId.keep.raw).toBe("later"); expect(doc.byId.gone).toBeUndefined();
  });
  it("does not count references removed inside the deleted subtree", () => {
    load([block("root", a, [{ ...block("child", b), raw: `((${a})) ((${a}))\nid:: ${b}` }]), block("tail")]);
    counts.set(a, 1);
    deleteBlock("root");
    expect(toasts()).toEqual([]);
  });
});
