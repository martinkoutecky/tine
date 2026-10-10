import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { carryDay, carryDaysBack, carryPrevDay } from "./carry";
import { journalTitle } from "./journal";
import { resetStore, loadFeed, pageByName, setRaw, moveBlock, moveBlockFeed, moveSelectionItems, moveItem, selectBlock, extendSelectionTo, selectedIds, outdentSelection, promotePagePreamble, persistBlockRefTarget, ensureBlockId, markDirty, flushPage, flushAll, isDirty, deletePage, undo } from "./document";
import { forgetPage } from "./document/workingSet";
import { loadSingle } from "./document/workingSet";
import { pageToDto } from "./document/convert";
import { doc } from "./document/model";
import { conflicts, isConflicted, resolveConflict, unsavedDrafts } from "./document";
import { unpublishedPages } from "./document/host/wiring";
import { bindTestHost } from "./document/host/wiring.test.support";
import { installDiskHost } from "./diskHost.test.support";
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
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
};

/** The pages' files on a fake disk served by the page host (bind after loading
 * the window's pages). `saved(name)` is the page's file as top-level raws. */
async function bindDisk(pages: PageRead[]) {
  const pathOf = new Map(pages.map((p) => [p.name, p.id] as const));
  const files = new Map<string, PageDto>(pages.map((p) => [p.id, structuredClone(p)]));
  const writes: [string, PageDto][] = [];
  const failing = new Set<string>();
  let n = 0;
  const spies = installDiskHost(await bindTestHost(), {
    read: (key) => files.get(key) ?? null,
    write: (key, dto) => {
      if (failing.has(key)) throw new Error("io:Other");
      writes.push([key, structuredClone(dto)]);
      const rev = `w${++n}`;
      files.set(key, { ...structuredClone(dto), rev });
      return rev;
    },
  });
  const path = (name: string) => pathOf.get(name)!;
  return {
    spies, files, writes,
    saved: (name: string) => files.get(path(name))?.blocks.map((b) => b.raw) ?? [],
    /** Another program rewrote the file (a later write onto the old revision is stale). */
    changeOnDisk: (name: string) => { files.get(path(name))!.rev = `theirs-${++n}`; },
    /** Writing the file fails (a disk error) from now on. */
    failWrites: (name: string) => failing.add(path(name)),
    /** The next host move waits for `gate`, then runs `before` and the move. */
    holdNextMove(before: () => void = () => {}) {
      const gate = deferred<void>();
      const run = spies.move.getMockImplementation()!;
      spies.move.mockImplementationOnce(async (...args) => { await gate.promise; before(); return run(...args); });
      return gate;
    },
    /** The next host submit waits for `gate`. */
    holdNextSubmit() {
      const gate = deferred<void>();
      const run = spies.submit.getMockImplementation()!;
      spies.submit.mockImplementationOnce(async (...args) => { await gate.promise; return run(...args); });
      return gate;
    },
  };
}

beforeEach(() => {
  serial = 0;
  resetStore();
  setToasts([]);
});
afterEach(() => vi.restoreAllMocks());

describe("cross-page moves save as ordered host moves", () => {
  const moves = [
    ["drag", async (id: string) => { await moveBlock(id, null, 0, "Newer"); }],
    ["cross-day root shortcut", async (id: string) => { expect(await moveBlockFeed(id, -1)).toBe("crossed"); }],
    ["feed boundary downward", async (id: string) => { expect(await moveBlockFeed(id, 1)).toBe("crossed"); }],
    ["selection boundary", async (id: string) => { selectBlock(id); await moveSelectionItems(-1); }],
  ] as const;

  for (const [label, act] of moves) for (const failure of ["conflict", "io:Other"] as const) {
    it(`${label}: a failed move leaves its saved source copy (${failure})`, async () => {
      const source = label === "feed boundary downward" ? "Newer" : "Older";
      const dest = source === "Older" ? "Newer" : "Older";
      const moved = block("portable task");
      const keeper = block("source keeper");
      const pages = source === "Older"
        ? [page("Newer", [block("destination keeper")]), page("Older", [moved, keeper])]
        : [page("Newer", [keeper, moved]), page("Older", [block("destination keeper")])];
      loadFeed(pages);
      const disk = await bindDisk(pages);
      const gate = disk.holdNextMove(() => failure === "conflict" ? disk.changeOnDisk(dest) : disk.failWrites(dest));
      const acting = act(moved.id);
      await vi.waitFor(() => expect(disk.spies.move).toHaveBeenCalledTimes(1));
      // One move: the receiver half first (STEP3 §8), then the source half.
      const [, , sourceHalf, receiverHalf] = disk.spies.move.mock.calls[0];
      expect(receiverHalf[1].name).toBe(dest);
      expect(sourceHalf[1].name).toBe(source);
      expect(receiverHalf[1].blocks.map((b) => b.raw)).toContain("portable task");
      expect(sourceHalf[1].blocks.map((b) => b.raw)).not.toContain("portable task");
      expect(raws(dest)).toContain("portable task");
      expect(raws(source)).not.toContain("portable task");
      expect(disk.saved(source)).toContain("portable task");
      expect(disk.saved(dest)).not.toContain("portable task");
      gate.resolve();
      await acting;
      // The host took the move; its receiver half failed, so the destination is
      // listed (conflicted, or not saved after a disk error) and nothing was written.
      await vi.waitFor(() => expect(unsavedDrafts().find((d) => d.name === dest)?.state)
        .toBe(failure === "conflict" ? "Conflict" : "Not saved"));
      expect(isConflicted(dest)).toBe(failure === "conflict");
      // The moved task is the receiver's host text (drafted before the source
      // published, STEP3 §8): still owed to the destination's file, and shown once.
      expect(await flushAll()).toBe(false);
      expect((await unpublishedPages())?.map((p) => p.key)).toContain(pages.find((p) => p.name === dest)!.id);
      expect(disk.saved(dest)).not.toContain("portable task");
      expect(raws(dest)).toContain("portable task");
      expect([...raws(source), ...raws(dest)].filter((raw) => raw === "portable task")).toHaveLength(1);
    });
  }

  it("a successful move leaves one saved copy", async () => {
    const moved = block("portable task");
    const pages = [page("Newer", [block("destination keeper")]), page("Older", [moved, block("source keeper")])];
    loadFeed(pages);
    const disk = await bindDisk(pages);
    const gate = disk.holdNextMove();
    await moveBlock(moved.id, null, 0, "Newer");
    const all = flushAll();
    await vi.waitFor(() => expect(disk.spies.move).toHaveBeenCalledTimes(1));
    const [, , sourceHalf, receiverHalf] = disk.spies.move.mock.calls[0];
    expect([receiverHalf[1].name, sourceHalf[1].name]).toEqual(["Newer", "Older"]);
    expect(disk.saved("Older")).toContain("portable task");
    gate.resolve();
    expect(await all).toBe(true);
    expect(disk.writes.map(([path]) => path)).toEqual(["journals/Newer.md", "journals/Older.md"]);
    expect(disk.saved("Newer").filter((raw) => raw === "portable task")).toHaveLength(1);
    expect(disk.saved("Older")).not.toContain("portable task");
  });

  // Restated (step 3b P2, relaxation ledger): a move's endpoints are frozen
  // while it runs, so the edit is refused (nothing shown, nothing written) or,
  // if accepted, kept on screen and on disk; never shown and then dropped.
  it("an edit typed into a move's source while the move runs is refused or kept, never dropped", async () => {
    const moved = block("portable task");
    const keeper = block("source keeper");
    const pages = [page("Newer", [block("destination keeper")]), page("Older", [moved, keeper])];
    loadFeed(pages);
    const disk = await bindDisk(pages);
    const gate = disk.holdNextMove();
    const acting = moveBlock(moved.id, null, 0, "Newer");
    await vi.waitFor(() => expect(disk.spies.move).toHaveBeenCalledTimes(1));
    setRaw(keeper.id, "source keeper edited");
    const accepted = raws("Older").includes("source keeper edited");
    gate.resolve();
    await acting;
    expect(await flushAll()).toBe(true);
    const kept = accepted ? "source keeper edited" : "source keeper";
    expect(raws("Older")).toEqual([kept]);
    expect(disk.saved("Older")).toEqual([kept]);
    expect(disk.saved("Newer").filter((raw) => raw === "portable task")).toHaveLength(1);
  });

  it("a conflicted source aborts before any memory move and tells the user", async () => {
    const moved = block("portable task");
    const pages = [page("Newer", []), page("Older", [moved])];
    loadFeed(pages);
    const disk = await bindDisk(pages);
    disk.changeOnDisk("Older");
    markDirty("Older", "save-block");
    expect(await flushPage("Older")).toBe(false);
    expect(isConflicted("Older")).toBe(true);
    expect(await moveBlockFeed(moved.id, -1)).toBe("none");
    await moveBlock(moved.id, null, 0, "Newer");
    expect(raws("Older")).toEqual(["portable task"]);
    expect(raws("Newer")).toEqual([]);
    expect(disk.spies.move).not.toHaveBeenCalled();
    expect(toasts().some((toast) => toast.message.includes("Resolve the conflict"))).toBe(true);
  });
});

describe("carry chooses source days and persists through host moves", () => {
  const date = (offset: number) => { const d = new Date(); d.setDate(d.getDate() - offset); return d; };
  const title = (offset: number) => journalTitle(date(offset));
  const key = (d: Date) => d.getFullYear() * 10000 + (d.getMonth() + 1) * 100 + d.getDate();

  it("carryDaysBack skips absent days and moves every touched day into today", async () => {
    const today = title(0), yesterday = title(1), third = title(3);
    const todayPage = page(today, [block("today note")]);
    loadFeed([todayPage]);
    const sources = new Map([[yesterday, page(yesterday, [block("TODO yesterday")])], [third, page(third, [block("TODO third")])]]);
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) =>
      structuredClone(name === today ? todayPage : sources.get(name) ?? null));
    const disk = await bindDisk([todayPage, ...sources.values()]);
    const gate = disk.holdNextMove();
    const carrying = carryDaysBack(3);
    await vi.waitFor(() => expect(raws(today)).toContain("TODO yesterday"));
    expect(raws(today)).toContain("TODO third");
    expect(disk.saved(yesterday)).toContain("TODO yesterday");
    expect(disk.saved(third)).toContain("TODO third");
    await vi.waitFor(() => expect(disk.spies.move).toHaveBeenCalledTimes(1));
    gate.resolve();
    await carrying;
    expect(await flushAll()).toBe(true);
    // Each source day is one move whose receiver is today.
    const halves = disk.spies.move.mock.calls.map(([, , source, receiver]) => [receiver[1].name, source[1].name]);
    expect(halves.map(([receiver]) => receiver)).toEqual(halves.map(() => today));
    expect(halves.map(([, source]) => source).sort()).toEqual([yesterday, third].sort());
    expect(disk.saved(today).filter((x) => x.startsWith("TODO"))).toEqual(["TODO yesterday", "TODO third"]);
    expect(disk.saved(yesterday)).not.toContain("TODO yesterday");
    expect(disk.saved(third)).not.toContain("TODO third");
  });

  it("carryPrevDay selects the latest content day before today", async () => {
    const today = title(0), recent = title(2), old = title(5);
    const todayPage = page(today, [block("today note")]);
    const recentPage = page(recent, [block("TODO recent")]), oldPage = page(old, [block("TODO old")]);
    loadFeed([todayPage]);
    vi.spyOn(backend(), "journalContentDays").mockResolvedValue([key(date(5)), key(date(2)), key(date(0))]);
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) =>
      structuredClone(name === today ? todayPage : name === recent ? recentPage : name === old ? oldPage : null));
    const disk = await bindDisk([todayPage, recentPage, oldPage]);
    await carryPrevDay();
    expect(raws(today)).toContain("TODO recent");
    expect(raws(today)).not.toContain("TODO old");
    expect(raws(recent)).not.toContain("TODO recent");
    expect(pageByName(old)).toBeUndefined();
    expect(disk.saved(today)).toContain("TODO recent");
    expect(disk.saved(old)).toEqual(["TODO old"]);
  });

  it("a carry whose today's file changed keeps the task in today's editor, owed and conflicted", async () => {
    const today = title(0), source = title(1);
    const pages = [page(today, []), page(source, [block("TODO safe")])];
    loadFeed(pages);
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) => structuredClone(pages.find((p) => p.name === name) ?? null));
    const disk = await bindDisk(pages);
    disk.holdNextMove(() => disk.changeOnDisk(today)).resolve();
    await carryDay(source);
    expect(disk.saved(today)).not.toContain("TODO safe");
    expect(raws(today)).toContain("TODO safe");
    expect(isConflicted(today)).toBe(true);
    expect((await unpublishedPages())?.map((p) => p.key)).toContain(pages[0].id);
    expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("kept in the editor"))).toBe(true);
  });

  it("a carry whose move the host refuses does not report the tasks as carried", async () => {
    const today = title(0), source = title(1);
    const pages = [page(today, []), page(source, [block("TODO stays")])];
    loadFeed(pages);
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) => structuredClone(pages.find((p) => p.name === name) ?? null));
    const disk = await bindDisk(pages);
    disk.spies.move.mockResolvedValueOnce({ reason: "failed", message: "host stopped" });
    await carryDay(source);
    expect(disk.writes).toEqual([]);
    expect(raws(source)).toContain("TODO stays");
    expect(raws(today)).not.toContain("TODO stays");
    expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("move did not happen"))).toBe(true);
    expect(toasts().filter((toast) => toast.message.startsWith("Carried")), "a refused carry is not reported as done").toEqual([]);
  });

  it("a carry onto a today with no file yet writes today's file with the task", async () => {
    const today = title(0), source = title(1);
    const sourcePage = page(source, [block("TODO first")]);
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) => structuredClone(name === source ? sourcePage : null));
    vi.spyOn(backend(), "resolvePage").mockImplementation(async (name) => ({ kind: "absent", id: `journals/${name}.md` }));
    loadFeed([sourcePage]);
    const disk = await bindDisk([sourcePage]);
    await carryDay(source);
    expect(await flushAll()).toBe(true);
    expect(raws(today)).toContain("TODO first");
    expect(disk.writes.map(([path]) => path)).toContain(`journals/${today}.md`);
    expect(disk.files.get(`journals/${today}.md`)?.blocks.map((b) => b.raw)).toContain("TODO first");
    expect(disk.saved(source)).not.toContain("TODO first");
  });
});

describe("small document intents retain visible and saved outcomes", () => {
  it("moveItem reorders siblings on the same page and saves that order", async () => {
    const a = block("first"), b = block("second");
    const notes = page("Notes", [a, b], "page");
    loadSingle(notes);
    const disk = await bindDisk([notes]);
    moveItem(b.id, -1);
    expect(raws("Notes")).toEqual(["second", "first"]);
    expect(isDirty("Notes")).toBe(true);
    expect(await flushPage("Notes")).toBe(true);
    expect(disk.saved("Notes")).toEqual(["second", "first"]);
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
    const referer = page("Referer", [block(`see ((${uuid}))`)], "page");
    const target = page("Target", [{ ...block("target"), id: uuid }], "page");
    loadSingle(referer);
    vi.spyOn(backend(), "getPage").mockImplementation(async () => structuredClone(target));
    const disk = await bindDisk([referer, target]);
    await persistBlockRefTarget(uuid, "Target", "page");
    expect(await flushPage("Target")).toBe(true);
    const stamped = disk.saved("Target")[0];
    expect(stamped.split("\n").filter((line) => line.startsWith("id::"))).toEqual([`id:: ${uuid}`]);
    expect(raws("Referer")).toEqual([`see ((${uuid}))`]);
    expect(isDirty("Referer")).toBe(false);
    expect(disk.writes.map(([path]) => path)).toEqual([target.id]);
    await persistBlockRefTarget(uuid, "Target", "page");
    expect(pageToDto("Target")!.blocks[0].raw.split("\n").filter((line) => line.startsWith("id::"))).toHaveLength(1);
  });

  it.each(["A source", "Z source"])("publishes the target ID before the authored reference when %s sorts around it", async (sourceName) => {
    const uuid = "48ae2a7a-e09b-4a21-aa3a-010101010101";
    const source = block("draft");
    const pages = [page(sourceName, [source], "page"), page("M target", [{ ...block("target"), id: uuid }], "page")];
    loadFeed(pages);
    const disk = await bindDisk(pages);
    // A process lost right after the target's write: the source never reaches disk.
    disk.failWrites(sourceName);
    const done = await persistBlockRefTarget(uuid, "M target", "page", undefined, uuid, () => {
      setRaw(source.id, `((${uuid}))`);
      return sourceName;
    });
    expect(done).toBe(true);
    await flushAll();
    expect(disk.writes[0][0], "the target's ID reaches disk first").toBe("pages/M target.md");
    expect(disk.saved("M target")[0]).toContain(`id:: ${uuid}`);
    expect(disk.saved(sourceName)).toEqual(["draft"]);
  });

  it("does not complete a block-reference stamp until the target page is saved", async () => {
    const uuid = "48ae2a7a-e09b-4a21-aa3a-010101010101";
    const target = page("Target", [{ ...block("target"), id: uuid }], "page");
    loadSingle(target);
    const disk = await bindDisk([target]);
    const gate = disk.holdNextSubmit();
    const pending = persistBlockRefTarget(uuid, "Target", "page");
    await vi.waitFor(() => expect(disk.spies.submit).toHaveBeenCalledOnce());
    let completed = false;
    void pending.then(() => { completed = true; });
    await Promise.resolve();
    expect(completed).toBe(false);
    expect(disk.saved("Target")[0]).not.toContain(`id:: ${uuid}`);
    gate.resolve();
    expect(await pending).toBe(true);
    expect(disk.saved("Target")[0]).toContain(`id:: ${uuid}`);
  });

  it("waits for an existing in-memory ID to reach disk before handing out a copied block ref", async () => {
    const uuid = "48ae2a7a-e09b-4a21-aa3a-010101010101";
    const target = page("Target", [{ ...block(`target\nid:: ${uuid}`), id: uuid }], "page");
    loadSingle(target);
    const disk = await bindDisk([{ ...target, blocks: [{ ...target.blocks[0], raw: "target" }] }]);
    markDirty("Target", "save-block");
    const gate = disk.holdNextSubmit();
    const pending = ensureBlockId(uuid);
    await vi.waitFor(() => expect(disk.spies.submit).toHaveBeenCalledOnce());
    let completed = false;
    void pending.then(() => { completed = true; });
    await Promise.resolve();
    expect(completed).toBe(false);
    gate.resolve();
    expect(await pending).toBe(uuid);
    expect(disk.saved("Target")[0]).toContain(`id:: ${uuid}`);
  });

  it("refuses references when a new or already-stamped ID cannot be saved", async () => {
    const uuid = "48ae2a7a-e09b-4a21-aa3a-010101010101";
    const target = page("Target", [{ ...block("target"), id: uuid }], "page");
    loadSingle(target);
    const disk = await bindDisk([target]);
    disk.changeOnDisk("Target");
    expect(await persistBlockRefTarget(uuid, "Target", "page")).toBe(false);
    expect(pageToDto("Target")!.blocks[0].raw).toContain(`id:: ${uuid}`);
    expect(await ensureBlockId(uuid)).toBeNull();
    expect(disk.writes).toEqual([]);
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
    const notes = page("Notes", [a], "page");
    loadSingle(notes);
    const disk = await bindDisk([notes]);
    setRaw(a.id, "edited");
    const removed = vi.spyOn(backend(), "pageDelete").mockImplementation(async () => { disk.files.delete(notes.id); return "applied"; });
    expect(await deletePage("Notes", "page")).toBe(true);
    const sentBeforeUndo = disk.spies.submit.mock.calls.length;
    undo();
    expect(await flushAll()).toBe(true);
    expect(pageByName("Notes")).toBeUndefined();
    expect(removed).toHaveBeenCalled();
    expect(disk.spies.submit.mock.calls.length, "undo after a delete sends nothing").toBe(sentBeforeUndo);
    expect(disk.spies.move).not.toHaveBeenCalled();
    expect(disk.files.has(notes.id)).toBe(false);
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
  it("marks a conflict after a refused save and clears it when keep-mine writes the edit", async () => {
    const a = block("original");
    const notes = page("Notes", [a], "page");
    loadSingle(notes);
    const disk = await bindDisk([notes]);
    disk.changeOnDisk("Notes");
    setRaw(a.id, "edited");
    expect(await flushAll()).toBe(false);
    expect(isConflicted("Notes")).toBe(true);
    expect(conflicts()).toContain("Notes");
    expect(disk.writes).toEqual([]);
    expect(await resolveConflict("Notes", "mine")).toBe(true);
    expect(await flushAll()).toBe(true);
    expect(isConflicted("Notes")).toBe(false);
    expect(disk.saved("Notes")).toEqual(["edited"]);
  });
});
