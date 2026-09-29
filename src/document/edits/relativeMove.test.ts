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

beforeAll(() => initParser());
afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
});

const blk = (id: string, children: BlockDto[] = []): BlockDto => ({ id, raw: id, collapsed: false, children });
const pageDto = (name: string, blocks: BlockDto[], extra: Partial<PageDto> = {}): PageDto => ({
  name, kind: "page", title: name, pre_block: null, blocks, ...extra,
});
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

  it("moves subtrees from several pages in captured order, inherits per root, and saves every page as one group", async () => {
    const sourceTwoRaw = "source two\n:PROPERTIES:\n:logseq.order-list-type: number\n:END:";
    const targetRaw = "target\n:PROPERTIES:\n:logseq.order-list-type: number\n:END:";
    loadFeed([
      pageDto("Source one", [{ id: "source-one", raw: "source one", collapsed: false, children: [
        { id: "child-one", raw: "child one\nbody:: byte-exact", collapsed: false, children: [] },
      ] }], { format: "md" }),
      pageDto("Source two", [{ id: "source-two", raw: sourceTwoRaw, collapsed: false, children: [
        { id: "child-two", raw: "child two\n:literal: byte-exact", collapsed: false, children: [] },
      ] }], { format: "org" }),
      pageDto("Destination", [
        { id: "destination-target", raw: targetRaw, collapsed: false, children: [] },
        { id: "destination-tail", raw: "tail", collapsed: false, children: [] },
      ], { format: "org" }),
    ]);
    clearSeededFacets();
    const before = snapshot();
    const save = vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => ({ ok: entries.map(() => "rev") }));

    expect(await moveBlocksRelative(["source-two", "source-one"], "destination-target", "before")).toBe(true);
    expect(pageByName("Source one")!.roots).toEqual([]);
    expect(pageByName("Source two")!.roots).toEqual([]);
    expect(pageByName("Destination")!.roots).toEqual(["source-two", "source-one", "destination-target", "destination-tail"]);
    expect(doc.byId["source-two"].raw).toBe(sourceTwoRaw);
    expect(doc.byId["source-one"].raw).toBe("source one\n:PROPERTIES:\n:logseq.order-list-type: number\n:END:");
    expect(doc.byId["child-one"]).toMatchObject({ page: "Destination", raw: "child one\nbody:: byte-exact" });
    expect(doc.byId["child-two"]).toMatchObject({ page: "Destination", raw: "child two\n:literal: byte-exact" });

    expect(await flushAll()).toBe(true);
    // The whole move is ONE save request: the gaining page and both emptied
    // sources commit together, so no interleaving save can observe a block on
    // neither page or on two pages.
    const moveRequests = save.mock.calls.filter((call) => call[0].some((entry) => entry.page.name === "Destination"));
    expect(moveRequests).toHaveLength(1);
    expect(moveRequests[0][0].map((entry) => entry.page.name).sort()).toEqual(["Destination", "Source one", "Source two"]);
    const after = snapshot();

    undo();
    expect(snapshot()).toEqual(before);
    redo();
    expect(snapshot()).toEqual(after);
  });
});
