import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { carryDay, carryDaysBack, carryPrevDay } from "./carry";
import { journalTitle } from "./journal";
import { doc, resetStore, loadFeed, loadSingle, pageByName, pageToDto, setRaw, moveBlock, moveBlockFeed, moveSelectionItems, moveItem, selectBlock, extendSelectionTo, selectedIds, outdentSelection, promotePagePreamble, persistBlockRefTarget, prepareCrossPageSources, markDirty, flushPage, flushAll, isDirty, forgetPage, deletePage, undo } from "./document";
import { clearConflict, conflicts, isConflicted, markConflict } from "./document";
import { toasts, setToasts } from "./toasts";
import type { BlockDto, PageDto, PageRead } from "./types";

let serial = 0;
const block = (raw: string, children: BlockDto[] = []): BlockDto => ({
  id: `pin-${++serial}`, raw, collapsed: false, children,
});
const page = (name: string, blocks: BlockDto[], kind: "page" | "journal" = "journal", pre_block: string | null = null): PageRead => ({
  name, title: name, kind, id: `${kind === "journal" ? "journals" : "pages"}/${name}.md`,
  rev: `initial-${name}`, pre_block, blocks,
});
const raws = (name: string) => pageByName(name)!.roots.map((id) => doc.byId[id].raw);
const savedRaws = (disk: Map<string, string[]>, name: string) => disk.get(name) ?? [];
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
};

beforeEach(() => {
  serial = 0;
  resetStore();
  conflicts().slice().forEach(clearConflict);
  setToasts([]);
});
afterEach(() => vi.restoreAllMocks());

describe("cross-page moves retain a durable copy until destination lands", () => {
  const moves = [
    ["drag", async (id: string) => { await moveBlock(id, null, 0, "Newer"); }],
    ["cross-day root shortcut", async (id: string) => { expect(await moveBlockFeed(id, -1)).toBe("crossed"); }],
    ["feed boundary downward", async (id: string) => { expect(await moveBlockFeed(id, 1)).toBe("crossed"); }],
    ["selection boundary", async (id: string) => { selectBlock(id); await moveSelectionItems(-1); }],
  ] as const;

  for (const [label, act] of moves) for (const failure of ["conflict", "io:Other"] as const) {
    it(`${label}: a delayed destination holds the source; ${failure} leaves its saved copy`, async () => {
      const source = label === "feed boundary downward" ? "Newer" : "Older";
      const dest = source === "Older" ? "Newer" : "Older";
      const moved = block("portable task");
      const keeper = block("source keeper");
      loadFeed(source === "Older"
        ? [page("Newer", [block("destination keeper")]), page("Older", [moved, keeper])]
        : [page("Newer", [keeper, moved]), page("Older", [block("destination keeper")])]);
      const disk = new Map([[source, ["portable task", "source keeper"]], [dest, ["destination keeper"]]]);
      const pending = deferred<string>();
      const save = vi.spyOn(backend(), "savePage").mockImplementation(async (_id, dto) => {
        if (dto.name === dest) {
          const rev = await pending.promise;
          disk.set(dest, dto.blocks.map((b) => b.raw));
          return rev;
        }
        disk.set(dto.name, dto.blocks.map((b) => b.raw));
        return "source-rev";
      });
      await act(moved.id);
      await vi.waitFor(() => expect(save.mock.calls.some(([, dto]) => dto.name === dest)).toBe(true));
      expect(raws(dest)).toContain("portable task");
      expect(raws(source)).not.toContain("portable task");
      setRaw(keeper.id, "source keeper edited");
      expect(await flushPage(source)).toBe(false);
      expect(savedRaws(disk, source)).toContain("portable task");
      expect(savedRaws(disk, dest)).not.toContain("portable task");
      pending.reject(new Error(failure));
      await vi.waitFor(() => expect(failure === "conflict" ? isConflicted(dest) : isDirty(dest)).toBe(true));
      expect(isConflicted(dest)).toBe(failure === "conflict");
      expect(savedRaws(disk, source)).toContain("portable task");
      expect(savedRaws(disk, dest)).not.toContain("portable task");
    });
  }

  it("a successful destination save releases the source removal and leaves one saved copy", async () => {
    const moved = block("portable task");
    loadFeed([page("Newer", [block("destination keeper")]), page("Older", [moved, block("source keeper")])]);
    const disk = new Map([["Newer", ["destination keeper"]], ["Older", ["portable task", "source keeper"]]]);
    const pending = deferred<string>();
    vi.spyOn(backend(), "savePage").mockImplementation(async (_id, dto) => {
      if (dto.name === "Newer") await pending.promise;
      disk.set(dto.name, dto.blocks.map((b) => b.raw));
      return `saved-${dto.name}`;
    });
    await moveBlock(moved.id, null, 0, "Newer");
    expect(savedRaws(disk, "Older")).toContain("portable task");
    pending.resolve("saved-Newer");
    expect(await flushAll()).toBe(true);
    expect(savedRaws(disk, "Newer").filter((raw) => raw === "portable task")).toHaveLength(1);
    expect(savedRaws(disk, "Older")).not.toContain("portable task");
  });

  it("an unflushable source aborts before any memory move and tells the user", async () => {
    const moved = block("portable task");
    loadFeed([page("Newer", []), page("Older", [moved])]);
    markDirty("Older");
    markConflict("Older");
    expect(await prepareCrossPageSources(["Older"])).toBe(false);
    expect(await moveBlockFeed(moved.id, -1)).toBe("none");
    await moveBlock(moved.id, null, 0, "Newer");
    expect(raws("Older")).toEqual(["portable task"]);
    expect(raws("Newer")).toEqual([]);
    expect(toasts().some((toast) => toast.message.includes("Couldn't move"))).toBe(true);
  });
});

describe("carry chooses source days and persists the addition first", () => {
  const date = (offset: number) => { const d = new Date(); d.setDate(d.getDate() - offset); return d; };
  const title = (offset: number) => journalTitle(date(offset));
  const key = (d: Date) => d.getFullYear() * 10000 + (d.getMonth() + 1) * 100 + d.getDate();

  it("carryDaysBack skips absent days and leaves source files intact until today saves", async () => {
    const today = title(0), yesterday = title(1), third = title(3);
    loadFeed([page(today, [block("today note")])]);
    const sources = new Map([[yesterday, page(yesterday, [block("TODO yesterday")])], [third, page(third, [block("TODO third")])]]);
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) => sources.get(name) ?? null);
    const disk = new Map([[today, ["today note"]], [yesterday, ["TODO yesterday"]], [third, ["TODO third"]]]);
    const pending = deferred<string>();
    vi.spyOn(backend(), "savePage").mockImplementation(async (_id, dto) => {
      if (dto.name === today) await pending.promise;
      disk.set(dto.name, dto.blocks.map((b) => b.raw));
      return `saved-${dto.name}`;
    });
    const carrying = carryDaysBack(3);
    await vi.waitFor(() => expect(raws(today)).toContain("TODO yesterday"));
    expect(raws(today)).toContain("TODO third");
    expect(savedRaws(disk, yesterday)).toContain("TODO yesterday");
    expect(savedRaws(disk, third)).toContain("TODO third");
    pending.resolve("saved-today");
    await carrying;
    expect(savedRaws(disk, today).filter((x) => x.startsWith("TODO"))).toEqual(["TODO yesterday", "TODO third"]);
    expect(savedRaws(disk, yesterday)).not.toContain("TODO yesterday");
    expect(savedRaws(disk, third)).not.toContain("TODO third");
  });

  it("carryPrevDay selects the latest content day before today", async () => {
    const today = title(0), recent = title(2), old = title(5);
    loadFeed([page(today, [block("today note")])]);
    vi.spyOn(backend(), "journalContentDays").mockResolvedValue([key(date(5)), key(date(2)), key(date(0))]);
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) =>
      name === recent ? page(recent, [block("TODO recent")]) : name === old ? page(old, [block("TODO old")]) : null);
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev");
    await carryPrevDay();
    expect(raws(today)).toContain("TODO recent");
    expect(raws(today)).not.toContain("TODO old");
    expect(raws(recent)).not.toContain("TODO recent");
    expect(pageByName(old)).toBeUndefined();
  });

  it("a failed today save keeps the task in its source's saved file and in today's editor", async () => {
    const today = title(0), source = title(1);
    loadFeed([page(today, []), page(source, [block("TODO safe")])]);
    const disk = new Map([[today, [] as string[]], [source, ["TODO safe"]]]);
    vi.spyOn(backend(), "savePage").mockImplementation(async (_id, dto) => {
      if (dto.name === today) throw new Error("conflict");
      disk.set(dto.name, dto.blocks.map((b) => b.raw));
      return "rev";
    });
    await carryDay(source);
    expect(raws(today)).toContain("TODO safe");
    expect(savedRaws(disk, source)).toContain("TODO safe");
    expect(savedRaws(disk, today)).not.toContain("TODO safe");
    expect(isConflicted(today)).toBe(true);
    expect(toasts().some((toast) => toast.message.includes("kept in the editor"))).toBe(true);
  });
});

describe("small document intents retain visible and saved outcomes", () => {
  it("moveItem reorders siblings on the same page and saves that order", async () => {
    const a = block("first"), b = block("second");
    loadSingle(page("Notes", [a, b], "page"));
    const saved: string[][] = [];
    vi.spyOn(backend(), "savePage").mockImplementation(async (_id, dto) => { saved.push(dto.blocks.map((x) => x.raw)); return "rev"; });
    moveItem(b.id, -1);
    expect(raws("Notes")).toEqual(["second", "first"]);
    expect(isDirty("Notes")).toBe(true);
    expect(await flushPage("Notes")).toBe(true);
    expect(saved.at(-1)).toEqual(["second", "first"]);
  });

  it("promotePagePreamble preserves properties and turns prose into the first saved block", async () => {
    loadSingle(page("Notes", [block("body")], "page", "tags:: project\n\nIntroduction"));
    const id = promotePagePreamble("Notes");
    expect(id).toBeTruthy();
    expect(raws("Notes")).toEqual(["Introduction", "body"]);
    expect(isDirty("Notes")).toBe(true);
    expect(pageToDto("Notes")).toMatchObject({ pre_block: "tags:: project", blocks: [{ raw: "Introduction" }, { raw: "body" }] });
  });

  it("extendSelectionTo exposes the inclusive visible range; outdentSelection saves siblings", async () => {
    const a = block("parent", [block("child one"), block("child two")]);
    loadSingle(page("Notes", [a], "page"));
    selectBlock(a.children[0].id);
    extendSelectionTo(a.children[1].id);
    expect(selectedIds()).toEqual(a.children.map((x) => x.id));
    outdentSelection();
    expect(raws("Notes")).toEqual(["parent", "child one", "child two"]);
    expect(pageToDto("Notes")!.blocks.map((x) => x.raw)).toEqual(["parent", "child one", "child two"]);
    expect(isDirty("Notes")).toBe(true);
  });

  it("persistBlockRefTarget stamps the unloaded target once while a referring page keeps its ref", async () => {
    const uuid = "48ae2a7a-e09b-4a21-aa3a-010101010101";
    loadSingle(page("Referer", [block(`see ((${uuid}))`)], "page"));
    vi.spyOn(backend(), "getPage").mockResolvedValue(page("Target", [{ ...block("target"), id: uuid }], "page"));
    const writes: PageDto[] = [];
    vi.spyOn(backend(), "savePage").mockImplementation(async (_id, dto) => { writes.push(structuredClone(dto)); return "rev"; });
    await persistBlockRefTarget(uuid, "Target", "page");
    expect(await flushPage("Target")).toBe(true);
    expect(writes.at(-1)?.blocks[0].raw.match(/id::/g)).toHaveLength(1);
    expect(raws("Referer")).toEqual([`see ((${uuid}))`]);
    expect(isDirty("Referer")).toBe(false);
    await persistBlockRefTarget(uuid, "Target", "page");
    expect(pageToDto("Target")!.blocks[0].raw.match(/id::/g)).toHaveLength(1);
  });
});

describe("history invalidation through page lifecycle", () => {
  it("forgetting a page drops its undo entry so undo cannot restore removed content", () => {
    const a = block("original");
    loadSingle(page("Notes", [a], "page"));
    setRaw(a.id, "edited");
    forgetPage("Notes");
    undo();
    expect(pageByName("Notes")).toBeUndefined();
    expect(doc.byId[a.id]).toBeUndefined();
  });

  it("deleting a page drops history and never recreates its file through undo", async () => {
    const a = block("original");
    loadSingle(page("Notes", [a], "page"));
    setRaw(a.id, "edited");
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev");
    const removed = vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined);
    expect(await deletePage("Notes", "page")).toBe(true);
    undo();
    expect(pageByName("Notes")).toBeUndefined();
    expect(removed).toHaveBeenCalled();
  });

  it("resetStore for a graph switch clears history before a same-name page loads", () => {
    const old = block("old graph");
    loadSingle(page("Notes", [old], "page"));
    setRaw(old.id, "old edited");
    resetStore();
    loadSingle(page("Notes", [block("new graph")], "page"));
    undo();
    expect(raws("Notes")).toEqual(["new graph"]);
  });
});

describe("save conflict set as observed through flushAll", () => {
  it("marks a conflict after a refused save and clears it after a successful retry", async () => {
    const a = block("original");
    loadSingle(page("Notes", [a], "page"));
    const save = vi.spyOn(backend(), "savePage").mockRejectedValueOnce(new Error("conflict")).mockResolvedValue("rev");
    setRaw(a.id, "edited");
    expect(await flushAll()).toBe(false);
    expect(isConflicted("Notes")).toBe(true);
    expect(conflicts()).toContain("Notes");
    clearConflict("Notes");
    markDirty("Notes");
    expect(await flushAll()).toBe(true);
    expect(isConflicted("Notes")).toBe(false);
    expect(save.mock.calls.at(-1)?.[1].blocks[0].raw).toBe("edited");
  });
});
