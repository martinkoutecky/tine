// og c3w W6 (L13): three edit paths that could leave the working copy in a state
// the save path will not persist as shown. Each case drives the real entry point.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { flushAll, isDirty, loadFeed, moveBlock, moveBlocksRelative, pageByName, pasteClipboardPayload, resetStore, setRaw, undo } from "./index";
import { doc } from "./model";
import { depthOf } from "./tree";
import { reloadPage } from "./workingSet";
import { setToasts, toasts } from "../toasts";
import { setGraphMeta } from "../graphSession";
import { OUTLINE_MAX_DEPTH } from "../editor/outline";
import type { ClipboardBlock, ClipboardPayloadSlot } from "../clipboard";
import type { BlockDto, PageDto } from "../types";
import { answerOpensFromDocument } from "./host/documentHost.test.support";
import { bindTestHost } from "./host/wiring.test.support";
import { transferInProgress } from "./host/wiring";

const uuid = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;
const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
const page = (name: string, blocks: BlockDto[], rev = `r1-${name}`): PageDto & { id: string; rev: string } => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev, blocks, format: "md",
});
const raws = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];
const maxDepth = () => Math.max(...Object.keys(doc.byId).map(depthOf));

function chainDto(levels: number, base: number): BlockDto {
  let node = block(uuid(base + levels), `level ${levels}`);
  for (let level = levels - 1; level >= 1; level--) node = block(uuid(base + level), `level ${level}`, [node]);
  return node;
}

function chainClipboard(levels: number): ClipboardBlock[] {
  let node: ClipboardBlock = { raw: `leaf ${levels}`, children: [], sourceFormat: "md" };
  for (let level = levels - 1; level >= 1; level--) node = { raw: `node ${level}`, children: [node], sourceFormat: "md" };
  return [node];
}

beforeAll(() => initParser());
beforeEach(async () => {
  resetStore();
  setToasts([]);
  answerOpensFromDocument(await bindTestHost());
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
});
afterEach(async () => {
  // A multi-page move keeps persisting after its intent returns: let it finish
  // before the store is reset under it.
  await vi.waitFor(() => expect(transferInProgress()).toBe(false));
  setGraphMeta(null);
  resetStore();
  setToasts([]);
  vi.restoreAllMocks();
});

describe("og c3w W6", () => {
  it("a typing undo whose block moved to a reloaded page never edits that page unsaved", async () => {
    const moved = uuid(1);
    loadFeed([page("A", [block(moved, "original"), block(uuid(2), "stays")]), page("B", [block(uuid(3), "b")])]);
    setRaw(moved, "typed");
    await moveBlock(moved, null, 0, "B");
    expect(doc.byId[moved].page).toBe("B");
    expect(await flushAll()).toBe(true);
    // B changed on disk (another device) and the watcher reloads it; the moved
    // block keeps its id because it carries it in the file.
    const disk = ["typed", "b edited elsewhere"];
    reloadPage(page("B", [block(moved, disk[0]), block(uuid(3), disk[1])], "r2-B"));
    undo();
    // Whatever B shows must be what B will save: a changed-but-clean B is an
    // edit that silently never reaches disk.
    if (!isDirty("B")) expect(raws("B")).toEqual(disk);
    expect(toasts().some((t) => /Undo history for this page was discarded/.test(t.message))).toBe(true);
  });

  it("a private-clipboard paste refuses an outline one level past the cap", async () => {
    loadFeed([page("P", [block(uuid(10), "host")])]);
    setGraphMeta({ root: "/graph" } as any);
    const before = Object.keys(doc.byId).length;
    const slot: ClipboardPayloadSlot = { op: "copy", generation: 0, graph: "/graph", text: "", blocks: chainClipboard(OUTLINE_MAX_DEPTH + 1), sourcePages: [] } as any;
    expect(await pasteClipboardPayload(uuid(10), slot)).toBeNull();
    expect(Object.keys(doc.byId).length).toBe(before);
    expect(isDirty("P")).toBe(false);
  });

  it("a private-clipboard paste exactly at the cap still lands", async () => {
    loadFeed([page("P", [block(uuid(10), "host")])]);
    setGraphMeta({ root: "/graph" } as any);
    const slot: ClipboardPayloadSlot = { op: "copy", generation: 0, graph: "/graph", text: "", blocks: chainClipboard(OUTLINE_MAX_DEPTH), sourcePages: [] } as any;
    expect(await pasteClipboardPayload(uuid(10), slot)).not.toBeNull();
    expect(maxDepth()).toBe(OUTLINE_MAX_DEPTH - 1);
  });

  it("a nested (child) relative drop checks the depth under the target", async () => {
    loadFeed([page("A", [chainDto(OUTLINE_MAX_DEPTH, 1000)]), page("B", [block(uuid(20), "target")])]);
    expect(maxDepth()).toBe(OUTLINE_MAX_DEPTH - 1);
    expect(await moveBlocksRelative([uuid(1001)], uuid(20), "child")).toBe(false);
    expect(maxDepth()).toBe(OUTLINE_MAX_DEPTH - 1);
    expect(doc.byId[uuid(1001)].page).toBe("A");
    // The same subtree still fits beside the target.
    expect(await moveBlocksRelative([uuid(1001)], uuid(20), "after")).toBe(true);
    expect(maxDepth()).toBe(OUTLINE_MAX_DEPTH - 1);
  });
});
