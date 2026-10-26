// GH #240 (master 6eea5b70c): a bullet drag and the context-menu heading row act
// on the active selection, as one transaction.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { initParser } from "../../render/parse";
import { clearSeededFacets } from "../../render/facets";
import {
  extendSelectionTo, flushAll, isDirty, loadFeed, moveBlocksRelative, pageByName, redo, resetStore,
  selectBlock, selectedIds, setSelectionHeading, undo,
} from "..";
import { loadSingle } from "../workingSet";
import { doc } from "../model";
import { backend } from "../../backend";
import type { BlockDto, PageDto } from "../../types";
import { transferInProgress } from "../host/wiring";
import { answerOpensFromDocument } from "../host/documentHost.test.support";
import { bindTestHost } from "../host/wiring.test.support";

beforeAll(() => initParser());
afterEach(async () => {
  // A multi-page move keeps persisting after its intent returns.
  await vi.waitFor(() => expect(transferInProgress()).toBe(false));
  vi.restoreAllMocks();
  resetStore();
});

const blk = (id: string, children: BlockDto[] = []): BlockDto => ({ id, raw: id, collapsed: false, children });
const pageDto = (name: string, blocks: BlockDto[], extra: Partial<PageDto> = {}): PageDto => ({
  name, kind: "page", title: name, pre_block: null, blocks, ...extra,
});
/** The page with its file, as a graph load installs it (the host opens it by path). */
const filed = (dto: PageDto): PageDto & { id: string } => ({ ...dto, id: `pages/${dto.name}.md` });
const ids = (blocks: BlockDto[]): string[] => blocks.flatMap((block) => [block.id!, ...ids(block.children)]);
type Endpoint = [key: string, dto: PageDto, version: number];
/** The host moves sent: [source page, receiver page] DTOs per step. */
const movesOf = (spy: { mock: { calls: unknown[][] } }) =>
  spy.mock.calls.map((call) => ({ source: (call[2] as Endpoint)[1], receiver: (call[3] as Endpoint)[1] }));
const snapshot = () => JSON.parse(JSON.stringify({ pages: doc.pages, byId: doc.byId }));

describe("selection heading ownership (GH #240)", () => {
  it("writes Markdown and Org targets in one undo unit and restores both pages", () => {
    loadFeed([
      pageDto("Markdown", [{ id: "heading-md", raw: "Markdown", collapsed: false, children: [] }], { format: "md" }),
      pageDto("Org", [{ id: "heading-org", raw: "Org", collapsed: false, children: [] }], { format: "org" }),
    ]);
    clearSeededFacets();
    selectBlock("heading-md");
    extendSelectionTo("heading-org");
    const before = snapshot();

    expect(setSelectionHeading("unused-pointer", 2)).toBe(true);
    expect(doc.byId["heading-md"].raw).toBe("## Markdown");
    expect(doc.byId["heading-org"].raw).toBe("Org\n:PROPERTIES:\n:heading: 2\n:END:");
    expect(selectedIds()).toEqual(["heading-md", "heading-org"]);
    expect(isDirty("Markdown") && isDirty("Org")).toBe(true);
    const after = snapshot();

    undo(); // one unit restores both pages
    expect(snapshot()).toEqual(before);
    redo();
    expect(snapshot()).toEqual(after);
  });

  it("is an exact no-op when any selected page is read-only", () => {
    loadFeed([
      pageDto("Writable", [{ id: "w", raw: "Writable", collapsed: false, children: [] }]),
      pageDto("Read only", [{ id: "r", raw: "Read only", collapsed: false, children: [] }], { read_only: true }),
    ]);
    selectBlock("w");
    extendSelectionTo("r");
    expect(setSelectionHeading("w", 3)).toBe(false);
    expect(doc.byId.w.raw).toBe("Writable");
    expect(doc.byId.r.raw).toBe("Read only");
    expect(isDirty("Writable") || isDirty("Read only")).toBe(false);
  });
});

describe("target-relative multi-root drag (GH #240)", () => {
  it("moves normalized roots from different sibling arrays together with exact undo/redo", async () => {
    loadSingle(pageDto("Test", [blk("parent", [blk("descendant")]), blk("holder", [blk("nested")]), blk("target")]));
    const before = snapshot();

    expect(await moveBlocksRelative(["parent", "descendant", "nested", "nested"], "target", "after")).toBe(true);
    expect(pageByName("Test")!.roots).toEqual(["holder", "target", "parent", "nested"]);
    expect(doc.byId.holder.children).toEqual([]);
    expect(doc.byId.parent.children).toEqual(["descendant"]);
    expect(doc.byId.nested.parent).toBeNull();
    expect(isDirty("Test")).toBe(true);
    const after = snapshot();

    undo();
    expect(snapshot()).toEqual(before);
    redo();
    expect(snapshot()).toEqual(after);
  });

  it("appends selected roots as children of the target with exact undo/redo (GH #326)", async () => {
    loadSingle(pageDto("Test", [blk("first"), blk("second", [blk("second child")]), blk("target", [blk("existing child")])]));
    const before = snapshot();

    expect(await moveBlocksRelative(["first", "second"], "target", "child")).toBe(true);
    expect(pageByName("Test")!.roots).toEqual(["target"]);
    expect(doc.byId.target.children).toEqual(["existing child", "first", "second"]);
    expect(doc.byId.first.parent).toBe("target");
    expect(doc.byId.second.parent).toBe("target");
    expect(doc.byId["second child"]).toMatchObject({ parent: "second", page: "Test" });
    expect(isDirty("Test")).toBe(true);
    const after = snapshot();

    undo();
    expect(snapshot()).toEqual(before);
    redo();
    expect(snapshot()).toEqual(after);
  });

  it.each([
    ["target is a moved root", "same"],
    ["target is inside a moved subtree", "descendant"],
    ["source page is read-only", "source-read-only"],
    ["destination page is read-only", "destination-read-only"],
  ] as const)("refuses when %s without mutation or dirty marks", async (_label, scenario) => {
    let sourceId = "source";
    let targetId: string;
    if (scenario === "same" || scenario === "descendant") {
      loadSingle(pageDto("Test", [blk("source", [blk("child")]), blk("target")]));
      targetId = scenario === "same" ? "source" : "child";
    } else {
      loadFeed([
        pageDto("Source", [blk("invalid-source")], { read_only: scenario === "source-read-only" }),
        pageDto("Destination", [blk("invalid-target")], { read_only: scenario === "destination-read-only" }),
      ]);
      sourceId = "invalid-source";
      targetId = "invalid-target";
    }
    const before = snapshot();
    expect(await moveBlocksRelative([sourceId], targetId, "after")).toBe(false);
    expect(snapshot()).toEqual(before);
    expect(doc.pages.some((page) => isDirty(page.name))).toBe(false);
  });

  it("moves subtrees from several pages in captured order, inherits per root, and moves every page through paired host moves", async () => {
    const sourceTwoRaw = "source two\n:PROPERTIES:\n:logseq.order-list-type: number\n:END:";
    const targetRaw = "target\n:PROPERTIES:\n:logseq.order-list-type: number\n:END:";
    loadFeed([
      filed(pageDto("Source one", [{ id: "source-one", raw: "source one", collapsed: false, children: [
        { id: "child-one", raw: "child one\nbody:: byte-exact", collapsed: false, children: [] },
      ] }], { format: "md" })),
      filed(pageDto("Source two", [{ id: "source-two", raw: sourceTwoRaw, collapsed: false, children: [
        { id: "child-two", raw: "child two\n:literal: byte-exact", collapsed: false, children: [] },
      ] }], { format: "org" })),
      filed(pageDto("Destination", [
        { id: "destination-target", raw: targetRaw, collapsed: false, children: [] },
        { id: "destination-tail", raw: "tail", collapsed: false, children: [] },
      ], { format: "org" })),
    ]);
    answerOpensFromDocument(await bindTestHost());
    clearSeededFacets();
    const before = snapshot();
    const move = vi.spyOn(backend(), "pageMove");
    const submit = vi.spyOn(backend(), "pageSubmit");

    expect(await moveBlocksRelative(["source-two", "source-one"], "destination-target", "before")).toBe(true);
    expect(pageByName("Source one")!.roots).toEqual([]);
    expect(pageByName("Source two")!.roots).toEqual([]);
    expect(pageByName("Destination")!.roots).toEqual(["source-two", "source-one", "destination-target", "destination-tail"]);
    expect(doc.byId["source-two"].raw).toBe(sourceTwoRaw);
    expect(doc.byId["source-one"].raw).toBe("source one\n:PROPERTIES:\n:logseq.order-list-type: number\n:END:");
    expect(doc.byId["child-one"]).toMatchObject({ page: "Destination", raw: "child one\nbody:: byte-exact" });
    expect(doc.byId["child-two"]).toMatchObject({ page: "Destination", raw: "child two\n:literal: byte-exact" });

    await vi.waitFor(() => expect(transferInProgress()).toBe(false));
    expect(await flushAll()).toBe(true);
    // Each source's subtree reaches the gaining page in one host move that
    // commits both endpoints together, so no save can observe a block on
    // neither page or on two pages; no single-page submit carries the move.
    const moves = movesOf(move);
    expect(moves.map(({ source, receiver }) => [source.name, receiver.name]).sort())
      .toEqual([["Source one", "Destination"], ["Source two", "Destination"]]);
    for (const { source, receiver } of moves) expect(ids(source.blocks).filter((id) => ids(receiver.blocks).includes(id))).toEqual([]);
    expect(ids(moves.at(-1)!.receiver.blocks))
      .toEqual(["source-two", "child-two", "source-one", "child-one", "destination-target", "destination-tail"]);
    expect(moves.every(({ source }) => source.blocks.length === 0)).toBe(true);
    expect(submit).not.toHaveBeenCalled();
    const after = snapshot();

    undo();
    expect(snapshot()).toEqual(before);
    // The undo is itself a multi-page move: its endpoints stay frozen until the
    // host took it, and the redo then applies.
    await vi.waitFor(() => expect(transferInProgress()).toBe(false));
    redo();
    expect(snapshot()).toEqual(after);
  });

  it("nests a cross-page drop under the target and saves both pages in one host move (GH #326)", async () => {
    loadFeed([
      filed(pageDto("Source", [blk("moved", [blk("moved child")])])),
      filed(pageDto("Destination", [blk("target", [blk("existing")]), blk("tail")])),
    ]);
    clearSeededFacets();
    answerOpensFromDocument(await bindTestHost());
    const move = vi.spyOn(backend(), "pageMove");
    const submit = vi.spyOn(backend(), "pageSubmit");

    expect(await moveBlocksRelative(["moved"], "target", "child")).toBe(true);
    expect(pageByName("Source")!.roots).toEqual([]);
    expect(pageByName("Destination")!.roots).toEqual(["target", "tail"]);
    expect(doc.byId.target.children).toEqual(["existing", "moved"]);
    expect(doc.byId.moved.parent).toBe("target");
    expect(doc.byId["moved child"]).toMatchObject({ page: "Destination", parent: "moved" });

    await vi.waitFor(() => expect(transferInProgress()).toBe(false));
    expect(await flushAll()).toBe(true);
    const moves = movesOf(move);
    expect(moves.map(({ source, receiver }) => [source.name, receiver.name])).toEqual([["Source", "Destination"]]);
    expect(moves[0].source.blocks).toEqual([]);
    expect(ids(moves[0].receiver.blocks)).toEqual(["target", "existing", "moved", "moved child", "tail"]);
    expect(submit).not.toHaveBeenCalled();
  });

  it("refuses to nest a block under its own descendant", async () => {
    loadSingle(pageDto("Test", [blk("source", [blk("child")])]));
    const before = snapshot();
    expect(await moveBlocksRelative(["source"], "child", "child")).toBe(false);
    expect(snapshot()).toEqual(before);
  });
});
