// "Changed since you last looked" (vision 9a, ADR 0073): the block hash, the
// live comparison against a baseline, the baseline's load/mark/forget, and the
// rename policy.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { resetStore, setRaw, deleteBlock, indentBlock, insertEmptyChildBlock, toggleCollapse, moveBlock } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { setGraphMeta } from "../graphSession";
import { backend } from "../backend";
import { pageIdentityKey } from "../pageIdentity";
import type { BlockDto, PageDto } from "../types";
import { pageBlockIds, seenBlockHash } from "./hash";
import { createSeenTracker } from "./tracker";
import { currentPageHashes, forgetPageSeen, loadSeenBaseline, markPageSeen, resetSeenBaselinesForTests, seenBaselineFor } from "./baseline";

let counter = 0;
const blk = (raw: string, children: BlockDto[] = []): BlockDto => ({ id: `s${counter++}`, raw, collapsed: false, children });
function load(blocks: BlockDto[], name = "Seen"): PageDto {
  const dto: PageDto = { name, kind: "page", title: name, pre_block: null, blocks, format: "md" };
  loadSingle(dto);
  return dto;
}
const hashOf = (raw: string) => seenBlockHash(raw, "md");

beforeAll(() => initParser());
beforeEach(() => {
  counter = 0;
  resetStore();
  resetSeenBaselinesForTests();
  setGraphMeta({ root: "/graphs/seen" } as never);
});
afterEach(() => vi.restoreAllMocks());

describe("seen block hash", () => {
  it("is 64 bits of the block's own text, property lines included", () => {
    expect(hashOf("Plan the trip")).toMatch(/^[0-9a-f]{16}$/);
    expect(hashOf("Plan the trip")).toBe(hashOf("Plan the trip"));
    expect(hashOf("Plan the trip")).not.toBe(hashOf("Plan the trip!"));
    // A property line is content: an agent setting `status::` is a change.
    expect(hashOf("Plan the trip\nstatus:: open")).not.toBe(hashOf("Plan the trip"));
    expect(hashOf("Plan the trip\nstatus:: open")).not.toBe(hashOf("Plan the trip\nstatus:: done"));
  });

  it("ignores the fold state and a save's trailing-space trim", () => {
    expect(hashOf("Plan the trip\ncollapsed:: true")).toBe(hashOf("Plan the trip"));
    expect(hashOf("Plan the trip   ")).toBe(hashOf("Plan the trip"));
    expect(seenBlockHash("Plan\n:PROPERTIES:\n:collapsed: true\n:END:", "org")).toBe(seenBlockHash("Plan", "org"));
  });

  it("covers the block's own content, never its children", () => {
    const child = blk("child text");
    const parent = blk("parent text", [child]);
    load([parent]);
    const before = currentPageHashes("Seen");
    setRaw(child.id, "child text edited");
    const after = currentPageHashes("Seen");
    expect(after).toContain(hashOf("parent text"));
    expect(before.filter((h) => !after.includes(h))).toEqual([hashOf("child text")]);
    expect(pageBlockIds("Seen")).toEqual([parent.id, child.id]);
  });

  it("does not see a fold made in Tine as a change", () => {
    const parent = blk("parent", [blk("child")]);
    load([parent]);
    const before = currentPageHashes("Seen");
    toggleCollapse(parent.id);
    expect(doc.byId[parent.id].raw).toContain("collapsed:: true");
    expect(currentPageHashes("Seen")).toEqual(before);
  });
});

describe("seen tracker", () => {
  function tracked(blocks: BlockDto[]) {
    load(blocks);
    const baseline = new Set(currentPageHashes("Seen"));
    const tracker = createSeenTracker("Seen", () => baseline);
    const changed = () => pageBlockIds("Seen").filter((id) => tracker.changed(id));
    return { tracker, changed };
  }

  it("starts with nothing changed against the page's own hashes", () => {
    const { tracker, changed } = tracked([blk("a"), blk("b")]);
    expect(tracker.count()).toBe(0);
    expect(changed()).toEqual([]);
    tracker.dispose();
  });

  it("marks an edited block and only that block, and unmarks it when the edit is undone by hand", () => {
    const a = blk("a"), b = blk("b"), c = blk("c");
    const { tracker, changed } = tracked([a, b, c]);
    setRaw(b.id, "b edited");
    expect(changed()).toEqual([b.id]);
    expect(tracker.count()).toBe(1);
    setRaw(b.id, "b");
    expect(changed()).toEqual([]);
    expect(tracker.count()).toBe(0);
    tracker.dispose();
  });

  it("marks a new block, not a moved unchanged one, and forgets a deleted one", async () => {
    const a = blk("a"), b = blk("b"), c = blk("c");
    const { tracker, changed } = tracked([a, b, c]);
    // Moved: indent b under a, then move c to the top. Neither is a change.
    indentBlock(b.id, 0);
    expect(doc.byId[b.id].parent).toBe(a.id);
    await moveBlock(c.id, null, 0);
    expect(doc.pages[0].roots[0]).toBe(c.id);
    expect(tracker.count()).toBe(0);
    // New: a block whose content the baseline never had.
    const added = insertEmptyChildBlock(c.id, 0)!;
    setRaw(added, "brand new");
    expect(changed()).toEqual([added]);
    expect(tracker.count()).toBe(1);
    // Deleted: a block that leaves the page is not counted.
    deleteBlock(added);
    expect(doc.byId[added]).toBeUndefined();
    expect(tracker.count()).toBe(0);
    deleteBlock(c.id);
    expect(tracker.count()).toBe(0);
    tracker.dispose();
  });

  it("hashes only the edited block on a keystroke (D-10)", () => {
    const blocks = Array.from({ length: 200 }, (_, i) => blk(`line ${i}`));
    const { tracker } = tracked(blocks);
    const hashed = new Set<string>();
    const original = String.prototype.charCodeAt;
    vi.spyOn(String.prototype, "charCodeAt").mockImplementation(function (this: string, i: number) {
      hashed.add(String(this));
      return original.call(this, i);
    });
    setRaw(blocks[7].id, "line 7 edited");
    // Only the edited block's text is hashed, not every block on the page.
    expect([...hashed].filter((text) => text.startsWith("line "))).toEqual(["line 7 edited"]);
    expect(tracker.count()).toBe(1);
    tracker.dispose();
  });
});

describe("seen baseline record", () => {
  function fakeStore() {
    const records = new Map<string, string[]>();
    const io = vi.fn(async (request: { op: string; graph: string; page: string; hashes?: string[] }) => {
      const key = `${request.graph}\n${request.page}`;
      if (request.op === "mark") records.set(key, request.hashes!);
      else if (request.op === "forget") records.delete(key);
      else return records.get(key) ?? null;
      return null;
    });
    (backend() as unknown as { seenBaseline: typeof io }).seenBaseline = io;
    return { records, io };
  }

  it("is written only by Mark page seen, keyed by graph and page identity", async () => {
    const { records, io } = fakeStore();
    load([blk("a"), blk("b")], "Reading List");
    loadSeenBaseline("Reading List");
    await vi.waitFor(() => expect(seenBaselineFor("Reading List")).toBeNull());
    expect(io.mock.calls.map(([r]) => r.op)).toEqual(["load"]);
    expect(await markPageSeen("Reading List")).toBe(true);
    expect([...records.keys()]).toEqual([`/graphs/seen\n${pageIdentityKey("Reading List")}`]);
    expect(seenBaselineFor("reading list")?.size).toBe(2);
    // Editing writes nothing.
    setRaw(doc.pages[0].roots[0], "a edited");
    expect(io.mock.calls.map(([r]) => r.op)).toEqual(["load", "mark"]);
  });

  it("treats a failed or empty read as no baseline, never an error", async () => {
    const { io } = fakeStore();
    io.mockRejectedValueOnce(new Error("unreadable"));
    load([blk("a")]);
    loadSeenBaseline("Seen");
    await vi.waitFor(() => expect(seenBaselineFor("Seen")).toBeNull());
  });

  it("drops the baseline on Forget seen state and on a rename (never carries it)", async () => {
    const { records } = fakeStore();
    load([blk("a")], "Old Name");
    await markPageSeen("Old Name");
    expect(records.size).toBe(1);
    await forgetPageSeen("Old Name");
    expect(records.size).toBe(0);
    expect(seenBaselineFor("Old Name")).toBeNull();

    await markPageSeen("Old Name");
    const graph = await import("../graph");
    const disk = await import("../document");
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "missing" } as never);
    vi.spyOn(disk, "renamePageOnDisk").mockResolvedValue("renamed" as never);
    expect(await graph.renameOrMergePage("Old Name", "New Name")).toBe("renamed");
    await vi.waitFor(() => expect(records.size).toBe(0));
    expect(seenBaselineFor("New Name")).toBeUndefined();
  });
});
