import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend, type SavePageEntry, type SavePagesResult } from "../../backend";
import { loadFeed, resetStore, pageByName, moveBlock, undo, setRaw, flushAll, flushPage, isConflicted, ensurePageLoaded, deletePage, selectBlock, extendSelectionTo, cycleSelectionTasks, deleteSelection } from "../index";
import { conflictReason, group, markConflict, persistTogether, resolveConflict, waitingOn } from "./engine";
import { doc } from "../model";
import { carryUnfinished } from "../edits/carry";
import { journalTitle } from "../../journal";
import { setToasts, toasts } from "../../toasts";
import { initParser } from "../../render/parse";
import type { BlockDto, PageRead } from "../../types";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `group-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[], id = `pages/${name}.md`): PageRead => ({
  id, name, title: name, kind: "page", pre_block: null, rev: `initial-${name}`, blocks: raws.map(block),
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
    return { ok: entries.map((_, i) => `rev-${requests.length}-${i}`) };
  });
  return { disk, requests, save };
}

beforeAll(() => initParser());
beforeEach(() => { serial = 0; resetStore(); setToasts([]); });
afterEach(() => vi.restoreAllMocks());

describe("save groups", () => {
  it("B1: choosing disk for a conflicted destination releases its source without removing the disk copy", async () => {
    const moved = block("X");
    loadFeed([page("A", []), { ...page("B", []), blocks: [moved] }]);
    const { disk, save } = diskBackend({ A: [], B: ["X"] });
    save.mockResolvedValue({ failed: { index: 0, family: "conflict", undoFailed: [] } });
    await moveBlock(moved.id, null, 0, "A");
    expect(await flushAll()).toBe(false);
    expect(isConflicted("A")).toBe(true);
    vi.spyOn(backend(), "getPage").mockResolvedValue(page("A", []));
    expect(await resolveConflict("A", "disk")).toBe(true);
    expect(conflictReason("B")).toEqual({ kind: "released", partner: "A" });
    expect(disk.get("B")).toContain("X");
    expect(disk.get("A")).not.toContain("X");
    expect(memory("B")).not.toContain("X");
  });

  it("B2: keeping a conflicted source writes the destination in the same request", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const { disk, save } = diskBackend({ A: ["X"], B: [] });
    save.mockResolvedValueOnce({ failed: { index: 1, family: "conflict", undoFailed: [] } });
    await moveBlock(moved.id, null, 0, "B");
    expect(await flushPage("B")).toBe(false);
    expect(isConflicted("A")).toBe(true);
    expect(await resolveConflict("A", "mine")).toBe(true);
    expect(save.mock.calls.at(-1)![0].map((entry) => entry.page.name)).toEqual(["B", "A"]);
    expect(save.mock.calls.at(-1)![0][1].force).toBe(true);
    expect(disk.get("B")).toContain("X");
    expect(disk.get("A")).not.toContain("X");
  });

  it("B3/N2: chained moves merge and order C, B, A; every crash prefix retains X", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", []), page("C", [])]);
    const { requests } = diskBackend({ A: ["X"], B: [], C: [] });
    await moveBlock(moved.id, null, 0, "B");
    await moveBlock(moved.id, null, 0, "C");
    expect(group("A")).toBe(group("C"));
    expect(await flushAll()).toBe(true);
    expect(requests).toHaveLength(1);
    expect(requests[0].map((entry) => entry.page.name)).toEqual(["C", "B", "A"]);
    for (let count = 0; count <= 3; count++) {
      const crashDisk = new Map([["A", ["X"]], ["B", [] as string[]], ["C", [] as string[]]]);
      for (const entry of requests[0].slice(0, count)) crashDisk.set(entry.page.name, entry.page.blocks.map((b) => b.raw));
      expect([...crashDisk.values()].flat()).toContain("X");
    }
  });

  it("S1: a preexisting single save finishes before the group snapshots that page", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const prior = deferred<SavePagesResult>();
    const save = vi.spyOn(backend(), "savePages").mockImplementationOnce(() => prior.promise)
      .mockImplementation(async (entries) => ({ ok: entries.map(() => "group-rev") }));
    setRaw(moved.id, "X edited");
    const first = flushPage("A");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    await moveBlock(moved.id, null, 0, "B");
    const all = flushAll();
    expect(save).toHaveBeenCalledTimes(1);
    prior.resolve({ ok: ["prior-rev"] });
    expect(await first).toBe(true);
    expect(await all).toBe(true);
    expect(save.mock.calls[1][0].find((entry) => entry.page.name === "A")?.baseRev).toBe("prior-rev");
    expect(save.mock.calls[1][0].map((entry) => entry.page.name)).toEqual(["B", "A"]);
  });

  it("S1: a queued single save defers at snapshot after its page joins a group", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const prior = deferred<SavePagesResult>();
    const save = vi.spyOn(backend(), "savePages").mockImplementationOnce(() => prior.promise)
      .mockImplementation(async (entries) => ({ ok: entries.map(() => "rev") }));
    setRaw(moved.id, "X edited");
    const first = flushPage("A");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    setRaw(moved.id, "X latest");
    const queued = flushPage("A");
    await moveBlock(moved.id, null, 0, "B");
    const grouped = flushPage("B");
    prior.resolve({ ok: ["prior-rev"] });
    expect(await first).toBe(true);
    expect(await grouped).toBe(true);
    expect(await queued).toBe(true);
    expect(save).toHaveBeenCalledTimes(2);
    expect(save.mock.calls[1][0].map((entry) => entry.page.name)).toEqual(["B", "A"]);
    expect(save.mock.calls[1][0][0].page.blocks.map((b) => b.raw)).toContain("X latest");
  });

  it("S2: undoFailed conflicts its member; repeated conflicts all members with one toast", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ failed: { index: 0, family: "io", undoFailed: [1] } });
    await moveBlock(moved.id, null, 0, "B");
    expect(await flushPage("B")).toBe(false);
    expect(isConflicted("A")).toBe(true);
    resetStore(); setToasts([]);
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    save.mockResolvedValue({ failed: { index: 0, family: "repeated", undoFailed: [] } });
    await moveBlock(moved.id, null, 0, "B");
    expect(await flushPage("B")).toBe(false);
    expect(conflictReason("A")?.kind).toBe("repeated");
    expect(conflictReason("B")?.kind).toBe("repeated");
    expect(toasts().filter((toast) => toast.kind === "error")).toHaveLength(1);
  });

  it("P4/F1: waiting pages are listed and two mine decisions produce one request", async () => {
    loadFeed([page("A", ["a"]), page("B", ["b"])]);
    const { save } = diskBackend({ A: ["a"], B: ["b"] });
    markConflict("A"); markConflict("B");
    void persistTogether(["A", "B"]);
    expect(waitingOn("A")).toEqual([]);
    expect(await resolveConflict("A", "mine")).toBe(true);
    expect(save).not.toHaveBeenCalled();
    expect(await resolveConflict("B", "mine")).toBe(true);
    expect(save).toHaveBeenCalledTimes(1);
    expect(save.mock.calls[0][0].map((entry) => entry.force)).toEqual([true, true]);
  });

  it("P4: a conflicted member lists the pages waiting on it", () => {
    loadFeed([page("A", ["a"]), page("B", ["b"]), page("C", ["c"])]);
    void persistTogether(["A", "B", "C"]);
    markConflict("A");
    expect(waitingOn("A")).toEqual(["B", "C"]);
  });

  it("X3/X4: an edit during a request lands later and an external conflict remains visible", async () => {
    const moved = block("X"), keeper = block("keeper");
    loadFeed([{ ...page("A", []), blocks: [moved, keeper] }, page("B", [])]);
    const pending = deferred<SavePagesResult>();
    const { save, disk } = diskBackend({ A: ["X", "keeper"], B: [] });
    save.mockImplementationOnce(() => pending.promise);
    await moveBlock(moved.id, null, 0, "B");
    const first = flushPage("B");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    setRaw(keeper.id, "later");
    markConflict("B");
    pending.resolve({ ok: ["first-b", "first-a"] });
    expect(await first).toBe(true);
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(2));
    expect(save.mock.calls[1][0]).toHaveLength(1);
    expect(save.mock.calls[1][0][0].page.blocks.map((b) => b.raw)).toEqual(["later"]);
    expect(disk.get("A")).toEqual(["later"]);
    expect(isConflicted("B")).toBe(true);
  });

  it("X1/X2/N3: an undo during an in-flight move remains grouped after the first request", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const pending = deferred<SavePagesResult>();
    const { save } = diskBackend({ A: ["X"], B: [] });
    save.mockImplementationOnce(() => pending.promise);
    await moveBlock(moved.id, null, 0, "B");
    const first = flushPage("B");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    undo();
    expect(group("A")?.state).toBe("open");
    const all = flushAll();
    pending.resolve({ ok: ["first-b", "first-a"] });
    expect(await first).toBe(true);
    expect(await all).toBe(true);
    expect(save.mock.calls[1][0]).toHaveLength(2);
    expect(save.mock.calls[1][0].map((entry) => entry.page.name)).toEqual(["A", "B"]);
    expect(memory("A")).toContain("X");
  });

  it("X1: a new intent during a sealed request saves in a successor group", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", []), page("C", [])]);
    const pending = deferred<SavePagesResult>();
    const { save } = diskBackend({ A: ["X"], B: [], C: [] });
    save.mockImplementationOnce(() => pending.promise);
    await moveBlock(moved.id, null, 0, "B");
    const first = flushPage("B");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    await moveBlock(moved.id, null, 0, "C");
    expect(group("B")?.state).toBe("open");
    const all = flushAll();
    pending.resolve({ ok: ["b-rev", "a-rev"] });
    expect(await first).toBe(true);
    expect(await all).toBe(true);
    expect(save.mock.calls[1][0].map((entry) => entry.page.name)).toEqual(["C", "B"]);
    expect(isConflicted("B")).toBe(false);
  });

  it("seal point: a failed predecessor merges into its open successor before sealing", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", []), page("C", [])]);
    const pending = deferred<SavePagesResult>();
    const { save } = diskBackend({ A: ["X"], B: [], C: [] });
    save.mockImplementationOnce(() => pending.promise);
    await moveBlock(moved.id, null, 0, "B");
    const first = flushPage("B");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    await moveBlock(moved.id, null, 0, "C");
    const later = flushPage("C");
    pending.resolve({ failed: { index: 0, family: "conflict", undoFailed: [] } });
    expect(await first).toBe(false);
    expect(await later).toBe(false);
    expect(group("A")).toBe(group("C"));
    expect(group("C")?.state).toBe("open");
    expect(await resolveConflict("B", "mine")).toBe(true);
    expect(save.mock.calls[1][0].map((entry) => entry.page.name)).toEqual(["C", "B", "A"]);
  });

  it("S3: carry to a pathless today creates today in its grouped request", async () => {
    const today = journalTitle(new Date()), old = "2026-09-20";
    loadFeed([
      { ...page(today, []), kind: "journal", id: undefined, rev: undefined },
      { ...page(old, ["TODO X"]), kind: "journal" },
    ]);
    const resolve = vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: `journals/${today}.md` });
    const { save } = diskBackend({ [today]: [], [old]: ["TODO X"] });
    expect(carryUnfinished([old], false, null)).toBe(1);
    expect(await flushPage(today)).toBe(true);
    expect(resolve).toHaveBeenCalledWith(today, "journal");
    expect(save.mock.calls[0][0].map((entry) => entry.page.name)).toEqual([today, old]);
    expect(save.mock.calls[0][0][0].baseRev).toBeNull();
  });

  it("R2: a failed carry followed by disk choice keeps the task on its source disk", async () => {
    const today = journalTitle(new Date()), old = "2026-09-20";
    loadFeed([{ ...page(today, []), kind: "journal" }, { ...page(old, ["TODO X", "keeper"]), kind: "journal" }]);
    const { disk, save } = diskBackend({ [today]: [], [old]: ["TODO X", "keeper"] });
    save.mockResolvedValue({ failed: { index: 0, family: "conflict", undoFailed: [] } });
    expect(carryUnfinished([old], false, null)).toBe(1);
    expect(await flushPage(today)).toBe(false);
    setRaw(pageByName(old)!.roots[0], "edited");
    vi.spyOn(backend(), "getPage").mockResolvedValue({ ...page(today, []), kind: "journal" });
    expect(await resolveConflict(today, "disk")).toBe(true);
    expect(disk.get(old)).toContain("TODO X");
    expect(conflictReason(old)).toEqual({ kind: "released", partner: today });
  });

  it("P2: a conflicted destination refuses a move before memory changes", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    markConflict("B");
    const save = vi.spyOn(backend(), "savePages");
    await moveBlock(moved.id, null, 0, "B");
    expect(memory("A")).toContain("X");
    expect(memory("B")).not.toContain("X");
    expect(save).not.toHaveBeenCalled();
    expect(toasts().some((toast) => toast.message === "Resolve the conflict on “B” first.")).toBe(true);
  });

  it("N1: both members of one debounced group finish with one request", async () => {
    vi.useFakeTimers();
    try {
      loadFeed([page("A", ["a"]), page("B", ["b"])]);
      const { save } = diskBackend({ A: ["a"], B: ["b"] });
      const settled = persistTogether(["A", "B"]);
      await vi.advanceTimersByTimeAsync(400);
      expect(await settled).toBe(true);
      expect(save).toHaveBeenCalledTimes(1);
      expect(save.mock.calls[0][0]).toHaveLength(2);
    } finally { vi.useRealTimers(); }
  });

  it("F3: a group member stays loaded while its request is in flight", async () => {
    loadFeed([page("View", [])]);
    ensurePageLoaded(page("A", ["a"]));
    ensurePageLoaded(page("B", ["b"]));
    const pending = deferred<SavePagesResult>();
    const save = vi.spyOn(backend(), "savePages").mockImplementation(() => pending.promise);
    void persistTogether(["A", "B"]);
    const first = flushPage("A");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    for (let i = 0; i < 85; i++) ensurePageLoaded(page(`Extra ${i}`, []));
    expect(pageByName("A")).toBeTruthy();
    expect(pageByName("B")).toBeTruthy();
    pending.resolve({ ok: ["a-rev", "b-rev"] });
    expect(await first).toBe(true);
  });

  it("P1: deleting a grouped destination first lands the whole group", async () => {
    const moved = block("X");
    loadFeed([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const save = vi.spyOn(backend(), "savePages");
    vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined);
    await moveBlock(moved.id, null, 0, "B");
    expect(await deletePage("B", "page")).toBe(true);
    expect(save).toHaveBeenCalledTimes(1);
    expect(save.mock.calls[0][0].map((entry) => entry.page.name)).toEqual(["B", "A"]);
    expect(conflictReason("A")).toBeUndefined();
  });

  it("F4: a grouped page cannot be replaced by a later path-pinned load", async () => {
    loadFeed([page("A", ["local"], "pages/original.md"), page("B", ["b"])]);
    void persistTogether(["A", "B"]);
    ensurePageLoaded(page("A", ["stray"], "pages/stray.md"));
    expect(pageByName("A")?.id).toBe("pages/original.md");
    expect(memory("A")).toEqual(["local"]);
    resetStore();
    expect(group("A")).toBeUndefined();
  });

  it("R7: a pathless alias whose loaded owner is busy stays conflicted without a write", async () => {
    loadFeed([{ ...page("Draft", ["X"]), id: undefined, rev: undefined }, page("Other", ["other"]), page("Owner", ["owner"])]);
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "alias", owners: ["pages/Owner.md"] });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page("Owner", ["owner"]));
    const save = vi.spyOn(backend(), "savePages");
    setRaw(pageByName("Owner")!.roots[0], "owner edited");
    void persistTogether(["Draft", "Other"]);
    expect(await flushPage("Draft")).toBe(false);
    expect(conflictReason("Draft")?.kind).toBe("alias-owner-busy");
    expect(save).not.toHaveBeenCalled();
  });

  it("P3: cycle and delete selection across pages each use one group", async () => {
    const a = block("TODO A"), b = block("TODO B");
    loadFeed([{ ...page("A", []), blocks: [a] }, { ...page("B", []), blocks: [b] }]);
    const { save } = diskBackend({ A: ["TODO A"], B: ["TODO B"] });
    selectBlock(a.id);
    extendSelectionTo(b.id);
    expect(cycleSelectionTasks()).toBe(true);
    expect(await flushAll()).toBe(true);
    expect(save.mock.calls[0][0]).toHaveLength(2);
    deleteSelection();
    expect(await flushAll()).toBe(true);
    expect(save.mock.calls[1][0]).toHaveLength(2);
  });

  it("unit cost: measures page bytes and request count for 1 and 60 block drag and carry", async () => {
    for (const count of [1, 60]) for (const intent of ["drag", "carry"] as const) {
      resetStore(); vi.restoreAllMocks(); serial = 0;
      let destination: string;
      if (intent === "drag") {
        destination = "B";
        const sourceBlocks = Array.from({ length: count }, (_, i) => block(`source ${i}`));
        const destBlocks = Array.from({ length: count }, (_, i) => block(`destination ${i}`));
        loadFeed([{ ...page("A", []), blocks: sourceBlocks }, { ...page("B", []), blocks: destBlocks }]);
        const { save } = diskBackend({});
        await moveBlock(sourceBlocks[0].id, null, 0, "B");
        expect(await flushPage("B")).toBe(true);
        const entries = save.mock.calls[0][0];
        const bytes = entries.reduce((sum, entry) => sum + new TextEncoder().encode(JSON.stringify(entry.page)).length, 0);
        expect(save).toHaveBeenCalledTimes(1);
        expect(entries).toHaveLength(2);
        console.info(`save-groups unit cost ${intent} ${count} blocks: before=${bytes} page bytes/2 requests; after=${bytes} page bytes/1 request`);
      } else {
        destination = journalTitle(new Date());
        const old = "2026-09-20";
        const sourceBlocks = Array.from({ length: count }, (_, i) => block(i === 0 ? "TODO X" : `source ${i}`));
        const destBlocks = Array.from({ length: count }, (_, i) => block(`today ${i}`));
        loadFeed([{ ...page(destination, []), kind: "journal", blocks: destBlocks }, { ...page(old, []), kind: "journal", blocks: sourceBlocks }]);
        const { save } = diskBackend({});
        expect(carryUnfinished([old], false, null)).toBe(1);
        expect(await flushPage(destination)).toBe(true);
        const entries = save.mock.calls[0][0];
        const bytes = entries.reduce((sum, entry) => sum + new TextEncoder().encode(JSON.stringify(entry.page)).length, 0);
        expect(save).toHaveBeenCalledTimes(1);
        expect(entries).toHaveLength(2);
        console.info(`save-groups unit cost ${intent} ${count} blocks: before=${bytes} page bytes/2 requests; after=${bytes} page bytes/1 request`);
      }
    }
  });
});
