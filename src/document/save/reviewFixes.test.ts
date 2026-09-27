import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend, type SavePageEntry, type SavePagesResult } from "../../backend";
import { loadFeed, resetStore, pageByName, moveBlock, setRaw, flushAll, isConflicted, deletePage, isDirty } from "../index";
import { conflictReason, markConflict, persistTogether, resolveConflict } from "./engine";
import { doc } from "../model";
import { setToasts, toasts } from "../../toasts";
import { initParser } from "../../render/parse";
import type { BlockDto, PageRead } from "../../types";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `review-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[]): PageRead => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null,
  rev: `initial-${name}`, blocks: raws.map(block),
});
const memory = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
};
function diskBackend(initial: Record<string, string[]>) {
  const disk = new Map(Object.entries(initial).map(([name, raws]) => [name, raws.slice()]));
  const requests: SavePageEntry[][] = [];
  const save = vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
    requests.push(structuredClone(entries));
    for (const entry of entries) disk.set(entry.page.name, entry.page.blocks.map((b) => b.raw));
    return { ok: entries.map((_, index) => `rev-${requests.length}-${index}`) };
  });
  return { disk, requests, save };
}
beforeAll(() => initParser());
beforeEach(() => { serial = 0; resetStore(); setToasts([]); });
afterEach(() => vi.restoreAllMocks());

describe("save-group review regressions", () => {
  it("B-1: a new conflict after a mine decision needs another decision", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const { requests } = diskBackend({ A: ["X"], B: [] });
    await moveBlock(moved.id, null, 0, "B");
    markConflict("A"); markConflict("B");
    expect(await resolveConflict("B", "mine")).toBe(true);
    markConflict("B");
    const newerReason = conflictReason("B");
    expect(await resolveConflict("A", "mine")).toBe(true);
    expect(requests).toHaveLength(0);
    expect(conflictReason("B")).toBe(newerReason);
    expect(await resolveConflict("B", "mine")).toBe(true);
    expect(requests).toHaveLength(1);
    expect(requests[0].find((entry) => entry.page.name === "B")?.force).toBe(false);
  });

  it("B-1: a failed force cannot authorize a later external change", async () => {
    const moved = block("X"), other = block("keep");
    loadFeed([{ ...page("A", []), blocks: [moved, other] }, page("B", [])]);
    const { requests, save } = diskBackend({ A: ["X", "keep"], B: [] });
    await moveBlock(moved.id, null, 0, "B");
    markConflict("B");
    save.mockResolvedValueOnce({ failed: { index: 0, family: "io:Other", undoFailed: [] } });
    expect(await resolveConflict("B", "mine")).toBe(false);
    markConflict("B");
    setRaw(other.id, "typing in A");
    expect(await flushAll()).toBe(false);
    expect(save).toHaveBeenCalledTimes(1);
    expect(requests).toHaveLength(0);
    expect(isConflicted("B")).toBe(true);
  });

  it("S-3: use disk leaves that page clean and never rewrites its disk version", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const { requests, save } = diskBackend({ A: ["X"], B: [] });
    save.mockResolvedValueOnce({ failed: { index: 0, family: "conflict", undoFailed: [] } });
    await moveBlock(moved.id, null, 0, "B");
    expect(await flushAll()).toBe(false);
    vi.spyOn(backend(), "getPage").mockResolvedValueOnce({ ...page("B", ["external"]), rev: "ext" })
      .mockResolvedValueOnce(page("A", ["X"]));
    expect(await resolveConflict("B", "disk")).toBe(true);
    expect(isDirty("B")).toBe(false);
    expect(await resolveConflict("A", "disk")).toBe(true);
    const before = requests.length;
    expect(await flushAll()).toBe(true);
    expect(requests).toHaveLength(before);
  });

  it("S-1: deleting a released source keeps the only disk copy", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const { disk, save } = diskBackend({ A: ["X"], B: [] });
    save.mockResolvedValueOnce({ failed: { index: 0, family: "conflict", undoFailed: [] } });
    await moveBlock(moved.id, null, 0, "B");
    expect(await flushAll()).toBe(false);
    vi.spyOn(backend(), "getPage").mockResolvedValue({ ...page("B", []), rev: "ext" });
    expect(await resolveConflict("B", "disk")).toBe(true);
    expect(conflictReason("A")?.kind).toBe("released");
    const remove = vi.spyOn(backend(), "deletePage").mockImplementation(async () => { disk.delete("A"); });
    expect(await deletePage("A", "page")).toBe(false);
    expect(remove).not.toHaveBeenCalled();
    expect(disk.get("A")).toContain("X");
    expect(toasts().some((toast) => toast.message === "Resolve the conflict on “A” first.")).toBe(true);
  });

  it("S-2: disk read racing a group request cannot reload stale content", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const { disk, save } = diskBackend({ A: ["X"], B: [] });
    await moveBlock(moved.id, null, 0, "B");
    markConflict("A"); markConflict("B");
    expect(await resolveConflict("B", "mine")).toBe(true);
    const read = deferred<PageRead | null>();
    const getPage = vi.spyOn(backend(), "getPageByPath").mockImplementation(() => read.promise);
    const disking = resolveConflict("B", "disk");
    await vi.waitFor(() => expect(getPage).toHaveBeenCalledTimes(1));
    expect(await resolveConflict("B", "mine")).toBe(true);
    const flight = deferred<SavePagesResult>();
    save.mockImplementationOnce(async (entries) => {
      for (const entry of entries) disk.set(entry.page.name, entry.page.blocks.map((b) => b.raw));
      return flight.promise;
    });
    const mining = resolveConflict("A", "mine");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    read.resolve({ ...page("B", []), rev: "old-disk" });
    expect(await disking).toBe(false);
    flight.resolve({ ok: ["b1", "a1"] });
    expect(await mining).toBe(true);
    expect(disk.get("B")).toContain("X");
    expect(memory("B")).toContain("X");
  });

  it("R-5: pages with no transfer edges save after ordered members", async () => {
    loadFeed([page("C", ["C"]), page("A", ["A"]), page("B", ["B"])]);
    const { requests } = diskBackend({ A: ["A"], B: ["B"], C: ["C"] });
    void persistTogether(["C", "A", "B"], "move-blocks", [["A", "B"]]);
    expect(await flushAll()).toBe(true);
    expect(requests[0].map((entry) => entry.page.name)).toEqual(["B", "A", "C"]);
  });

  it("R-6: a twin failure on a pathless member requires use disk", async () => {
    loadFeed([page("A", ["A"]), { ...page("B", ["B"]), id: undefined, rev: undefined }]);
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/B.md" });
    const { save } = diskBackend({ A: ["A"], B: [] });
    save.mockResolvedValueOnce({ failed: { index: 0, family: "twin", undoFailed: [] } });
    void persistTogether(["A", "B"], "move-blocks", [["A", "B"]]);
    expect(await flushAll()).toBe(false);
    expect(conflictReason("A")?.kind).toBe("repeated");
    expect(conflictReason("B")?.kind).toBe("repeated");
    expect(await resolveConflict("B", "mine")).toBe(false);
  });

  it("resolve-id failure names the member whose resolution failed", async () => {
    loadFeed([page("A", ["A"]), page("B", ["B"]), { ...page("C", ["C"]), id: undefined, rev: undefined }]);
    vi.spyOn(backend(), "resolvePage").mockRejectedValue(new Error("unavailable"));
    const { save } = diskBackend({ A: ["A"], B: ["B"], C: [] });
    void persistTogether(["A", "B", "C"], "move-blocks", [["A", "B"]]);
    expect(await flushAll()).toBe(false);
    expect(save).not.toHaveBeenCalled();
    expect(toasts().some((toast) => toast.message.startsWith("Couldn't save “C”"))).toBe(true);
  });
});
