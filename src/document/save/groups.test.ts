// Multi-page edits on the page host (STEP3 §8). Save groups are gone: a page
// edit is one host submit; a block move between pages is a planned sequence of
// two-page host moves (`pageMove`), each one host transaction carrying both
// endpoints, with the endpoints frozen while it runs. A refused move writes
// nothing and the display shows the host's text; a crash between moves can
// leave a block in both pages, never in neither. The save-group mechanics
// these tests used to pin (seal points, successor groups, waiting lists,
// released/repeated reasons) left with engine.ts; the ledger names carriers.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { cycleSelectionTasks, deleteSelection, ensurePageLoaded, extendSelectionTo, flushAll, flushPage, insertEmptyChildBlock,
  isConflicted, loadFeed, moveBlock, pageByName, resetStore, selectBlock, setRaw, undo } from "../index";
import { doc } from "../model";
import { carryUnfinished } from "../edits/carry";
import { journalTitle } from "../../journal";
import { setToasts, toasts } from "../../toasts";
import { initParser } from "../../render/parse";
import { startEditing } from "../../editorController";
import { bindTestHost, mailPage, type TestHost } from "../host/wiring.test.support";
import type { EditKinds } from "../../editKind";
import type { BlockDto, PageDto, PageRead } from "../../types";
import type { PageRefusal } from "../host/protocol";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `group-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[], id = `pages/${name}.md`): PageRead => ({
  id, name, title: name, kind: "page", pre_block: null, rev: `initial-${name}`, blocks: raws.map(block),
});
const memory = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];
const raws = (dto: PageDto) => dto.blocks.map((b) => b.raw);
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
};
/** A page as a test loads it: `id` absent for a page with no file yet. */
type Loaded = PageDto & { id?: string };
const refusedMove = (): PageRefusal => ({ reason: "failed", message: "disk full" });

interface Request { request: "submit" | "move"; pages: PageDto[]; kinds: EditKinds; refused: boolean }

/** A host over the test's files: an open serves the file the window names, a
 * taken submit or move writes its pages, a refused request (`control.refuse`,
 * at admission) writes nothing. `log` is every page request, in order; `disk`
 * replays the first `count` taken requests (a crash after that prefix). */
function hostRequests(host: TestHost, initial: Loaded[] = []) {
  const api = backend();
  const open = api.pageOpen.bind(api);
  const files = new Map<string, { dto: PageDto; version: number }>(initial.filter((p) => p.id).map((p) => [p.id!, { dto: p, version: 1 }]));
  const log: Request[] = [];
  const control: { refuse: ((request: Request) => boolean) | null; hold: Promise<void> | null } = { refuse: null, hold: null };
  const record = async (entry: Omit<Request, "refused">) => {
    const refused = !!control.refuse?.({ ...entry, refused: false });
    log.push({ ...entry, refused });
    const hold = control.hold;
    control.hold = null;
    if (hold) await hold;
    return refused;
  };
  const write = (key: string, dto: PageDto, version: number, id: number) => {
    const next = Math.max(files.get(key)?.version ?? 1, version) + 1;
    files.set(key, { dto: { ...dto, rev: `${dto.name}-${next}` }, version: next });
    queueMicrotask(() => host.deliver({ key, page: { version: next, conflict: false, risk: false, text: { kind: "unchanged" } },
      answer: { id, version: next, took: true, outcome: { kind: "applied" } } }));
  };
  vi.spyOn(api, "pageOpen").mockImplementation(async (session, id, target) => {
    const file = target.path === null ? undefined : files.get(target.path);
    if (!file) return open(session, id, target);
    queueMicrotask(() => host.deliver({ key: target.path!, page: mailPage(file.version, file.dto),
      answer: { id, version: file.version, took: false, outcome: { kind: "applied" } } }));
    return { key: target.path!, baselineEntry: true };
  });
  const submit = vi.spyOn(api, "pageSubmit").mockImplementation(async (_session, id, key, dto, version, _resolve, kinds) => {
    if (await record({ request: "submit", pages: [dto], kinds })) return refusedMove();
    write(key, dto, version, id);
    return null;
  });
  const move = vi.spyOn(api, "pageMove").mockImplementation(async (_session, id, source, receiver, kinds) => {
    if (await record({ request: "move", pages: [source[1], receiver[1]], kinds })) return refusedMove();
    write(source[0], source[1], source[2], id);
    write(receiver[0], receiver[1], receiver[2], id);
    return null;
  });
  const disk = (count = Infinity) => {
    const state = new Map(initial.map((p) => [p.name, p.blocks.map((b) => b.raw)]));
    for (const entry of log.filter((request) => !request.refused).slice(0, count))
      for (const dto of entry.pages) state.set(dto.name, raws(dto));
    return Object.fromEntries(state);
  };
  return { log, submit, move, control, disk };
}

beforeAll(() => initParser());
beforeEach(() => { serial = 0; resetStore(); setToasts([]); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); resetStore(); });

/** Load `pages`, bind the window and put the test host over their files. */
async function load(pages: Loaded[]) {
  loadFeed(pages);
  const host = await bindTestHost();
  return { host, ...hostRequests(host, pages) };
}

describe("multi-page edits on the page host", () => {
  it("carries distinct edit kinds in first-seen order through one debounced send", async () => {
    const root = block("initial");
    const { log } = await load([{ ...page("A", []), blocks: [root] }]);
    setRaw(root.id, "first");
    insertEmptyChildBlock(root.id, 0);
    setRaw(root.id, "second");
    expect(await flushPage("A")).toBe(true);
    expect(log.map((request) => request.kinds)).toEqual([["save-block", "insert-blocks"]]);
    setRaw(root.id, "third");
    expect(await flushPage("A")).toBe(true);
    expect(log[1].kinds).toEqual(["save-block"]);
  });

  // The second move starts while the first is in flight, on its endpoint B.
  // STEP3 §8: endpoints are read-only during a sequence, so the window may
  // refuse the second move or run it after the first; either way the display
  // ends equal to the files, and the block is in some page at every crash
  // prefix and in exactly one at the end.
  it("B3/N2: chained moves keep the block in exactly one page, the display matching the files", async () => {
    const moved = block("X");
    const { log, disk } = await load([{ ...page("A", []), blocks: [moved] }, page("B", []), page("C", [])]);
    await moveBlock(moved.id, null, 0, "B");
    await moveBlock(moved.id, null, 0, "C");
    expect(await flushAll()).toBe(true);
    await vi.waitFor(() => expect(log.filter((request) => !request.refused).length).toBeGreaterThanOrEqual(1));
    await vi.waitFor(() => expect({ A: memory("A"), B: memory("B"), C: memory("C") }).toEqual(disk()));
    expect(log.every((request) => request.request === "move")).toBe(true);
    for (let count = 0; count <= log.length; count++) expect(Object.values(disk(count)).flat()).toContain("X");
    expect(Object.values(disk()).flat().filter((raw) => raw === "X")).toHaveLength(1);
    expect(disk().A).toEqual([]);
  });

  it("S1: a single send in flight finishes before the move snapshots that page", async () => {
    const moved = block("X");
    const { log, control, disk } = await load([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const prior = deferred<void>();
    control.hold = prior.promise;
    setRaw(moved.id, "X edited");
    const first = flushPage("A");
    await vi.waitFor(() => expect(log).toHaveLength(1));
    const moving = moveBlock(moved.id, null, 0, "B");
    await Promise.resolve();
    expect(log).toHaveLength(1);
    prior.resolve();
    expect(await first).toBe(true);
    await moving;
    expect(await flushAll()).toBe(true);
    await vi.waitFor(() => expect(log).toHaveLength(2));
    expect(log.map((request) => request.request)).toEqual(["submit", "move"]);
    const [source, receiver] = log[1].pages;
    expect([source.name, raws(source)]).toEqual(["A", []]);
    expect([receiver.name, raws(receiver)]).toEqual(["B", ["X edited"]]);
    expect(disk()).toEqual({ A: [], B: ["X edited"] });
  });

  // STEP3 §8 step 1: a move's endpoints are read-only while it runs, so "no
  // ordinary input exists to lose". Whether the window refuses the edit or
  // sends it after the move, it is never silently dropped.
  it("X3: an edit typed on an endpoint while its move is in flight is refused or saved, never dropped", async () => {
    const moved = block("X"), keeper = block("keeper");
    const { log, control, disk } = await load([{ ...page("A", []), blocks: [moved, keeper] }, page("B", [])]);
    const pending = deferred<void>();
    control.hold = pending.promise;
    const moving = moveBlock(moved.id, null, 0, "B");
    await vi.waitFor(() => expect(log).toHaveLength(1));
    setRaw(keeper.id, "later");
    const accepted = memory("A").includes("later");
    pending.resolve();
    await moving;
    expect(await flushAll()).toBe(true);
    expect(disk().B).toEqual(["X"]);
    if (accepted) {
      expect(memory("A"), "an accepted edit on a move endpoint stays on screen").toEqual(["later"]);
      expect(disk().A, "an accepted edit on a move endpoint reaches the file").toEqual(["later"]);
    } else {
      expect(disk().A).toEqual(["keeper"]);
    }
  });

  it("X1/X2/N3: an undo during an in-flight move waits, and the later undo moves the block back in one request", async () => {
    const moved = block("X");
    const { log, control, disk } = await load([{ ...page("A", []), blocks: [moved] }, page("B", [])]);
    const pending = deferred<void>();
    control.hold = pending.promise;
    const moving = moveBlock(moved.id, null, 0, "B");
    await vi.waitFor(() => expect(log).toHaveLength(1));
    expect(undo()).toBe(false);
    pending.resolve();
    await moving;
    expect(await flushAll()).toBe(true);
    await vi.waitFor(() => expect(memory("B")).toEqual(["X"]));
    expect(undo()).toBe(true);
    await vi.waitFor(() => expect(log).toHaveLength(2));
    expect(await flushAll()).toBe(true);
    expect(log.map((request) => request.request)).toEqual(["move", "move"]);
    expect(log[1].pages.map((dto) => [dto.name, raws(dto)]).sort()).toEqual([["A", ["X"]], ["B", []]]);
    expect(disk()).toEqual({ A: ["X"], B: [] });
    await vi.waitFor(() => expect(memory("A")).toEqual(["X"]));
  });

  // master 0.6.984 "Undoing a move between pages can no longer lose the moved
  // blocks": the page that regains the blocks and the page that loses them land
  // in one host request or not at all (og 20b, contract 4).
  it("undo of a landed cross-page move writes both pages in one request, and a refusal writes neither", async () => {
    const moved = block("X");
    const { log, control, disk } = await load([{ ...page("A", []), blocks: [moved, block("a")] }, page("B", ["b"])]);
    await moveBlock(moved.id, null, 0, "B");
    expect(await flushAll()).toBe(true);
    await vi.waitFor(() => expect(disk()).toEqual({ A: ["a"], B: ["X", "b"] }));

    control.refuse = (request) => request.request === "move";
    expect(undo()).toBe(true);
    await vi.waitFor(() => expect(toasts().some((t) => t.message.startsWith("The move did not happen"))).toBe(true));
    const refused = log.at(-1)!;
    expect(refused).toMatchObject({ request: "move", refused: true });
    expect(refused.pages.map((dto) => [dto.name, raws(dto)]).sort()).toEqual([["A", ["X", "a"]], ["B", ["b"]]]);
    // Refused as a whole: the files are as the first move left them, and the
    // display shows them (the block is in B, not lost).
    expect(disk()).toEqual({ A: ["a"], B: ["X", "b"] });
    expect(memory("A")).toEqual(["a"]);
    expect(memory("B")).toEqual(["X", "b"]);
  });

  it("S3: carry to a pathless today creates today in its move", async () => {
    const today = journalTitle(new Date()), old = "2026-09-20";
    const { log } = await load([
      { ...page(today, []), kind: "journal", id: undefined, rev: undefined },
      { ...page(old, ["TODO X"]), kind: "journal" },
    ]);
    const open = vi.mocked(backend().pageOpen);
    expect(carryUnfinished([old], false, null).moved).toBe(1);
    expect(await flushAll()).toBe(true);
    await vi.waitFor(() => expect(log).toHaveLength(1));
    // Today has no file yet: the host resolves its name at open (path null).
    expect(open.mock.calls.some((call) => call[2].name === today && call[2].path === null)).toBe(true);
    expect(log.map((request) => request.request)).toEqual(["move"]);
    expect(log[0].pages.map((dto) => [dto.name, raws(dto)])).toEqual([[old, []], [today, ["TODO X"]]]);
    expect(memory(today)).toEqual(["TODO X"]);
  });

  it("R2: a refused carry keeps the task on its source", async () => {
    const today = journalTitle(new Date()), old = "2026-09-20";
    const { log, control, disk } = await load([{ ...page(today, []), kind: "journal" },
      { ...page(old, ["TODO X", "keeper"]), kind: "journal" }]);
    control.refuse = (request) => request.request === "move";
    expect(carryUnfinished([old], false, null).moved).toBe(1);
    await vi.waitFor(() => expect(toasts().some((t) => t.message.startsWith("The move did not happen"))).toBe(true));
    expect(log).toHaveLength(1);
    expect(disk()).toEqual({ [today]: [], [old]: ["TODO X", "keeper"] });
    expect(memory(old)).toEqual(["TODO X", "keeper"]);
    expect(memory(today)).toEqual([]);
  });

  it("P2: a conflicted destination refuses a move before memory changes", async () => {
    const moved = block("X"), there = block("b");
    const { host, log } = await load([{ ...page("A", []), blocks: [moved] }, { ...page("B", []), blocks: [there] }]);
    // The user is editing B (the window holds it) when the host finds B's file
    // changed under the user's text.
    startEditing(there.id);
    setRaw(there.id, "b typed");
    await vi.waitFor(() => expect(log).toHaveLength(1));
    host.notice("pages/B.md", { conflictReported: true }, { version: 9, conflict: true, risk: true, disk: { kind: "file", rev: "ext" } });
    await vi.waitFor(() => expect(isConflicted("B")).toBe(true));
    await moveBlock(moved.id, null, 0, "B");
    expect(memory("A")).toEqual(["X"]);
    expect(memory("B")).toEqual(["b typed"]);
    expect(await flushPage("A")).toBe(true);
    expect(log).toHaveLength(1);
    expect(toasts().some((toast) => toast.message === "Resolve the conflict on “B” first.")).toBe(true);
  });

  it("F3: a page whose request is in flight stays loaded", async () => {
    const pages = [page("View", []), page("A", ["a"]), page("B", ["b"])];
    loadFeed([pages[0]]);
    ensurePageLoaded(pages[1]);
    ensurePageLoaded(pages[2]);
    const host = await bindTestHost();
    const { log, control } = hostRequests(host, pages);
    const pending = deferred<void>();
    control.hold = pending.promise;
    setRaw(pageByName("A")!.roots[0], "a edited");
    setRaw(pageByName("B")!.roots[0], "b edited");
    const first = flushPage("A");
    await vi.waitFor(() => expect(log).toHaveLength(1));
    for (let i = 0; i < 85; i++) ensurePageLoaded(page(`Extra ${i}`, []));
    expect(memory("A")).toEqual(["a edited"]);
    expect(memory("B")).toEqual(["b edited"]);
    pending.resolve();
    expect(await first).toBe(true);
    expect(await flushAll()).toBe(true);
  });

  it("F4: a page with unsent input cannot be replaced by a later path-pinned load", async () => {
    await load([page("A", ["local"], "pages/original.md"), page("B", ["b"])]);
    setRaw(pageByName("A")!.roots[0], "local edited");
    ensurePageLoaded(page("A", ["stray"], "pages/stray.md"));
    expect(pageByName("A")?.id).toBe("pages/original.md");
    expect(memory("A")).toEqual(["local edited"]);
  });

  it("R7: an alias draft whose loaded owner has unsent input lands after that input, keeping it", async () => {
    const owner = page("Owner", ["owner"]);
    const api = backend();
    vi.spyOn(api, "getPageByPath").mockResolvedValue(owner);
    const { log } = await load([{ ...page("Draft", ["X"]), id: undefined, rev: undefined }, owner]);
    const open = vi.mocked(api.pageOpen).getMockImplementation()!;
    vi.mocked(api.pageOpen).mockImplementation(async (session, id, target) =>
      target.name === "Draft" && target.path === null ? { reason: "alias" as const, owners: ["pages/Owner.md"] } : open(session, id, target));
    setRaw(pageByName("Owner")!.roots[0], "owner edited");
    setRaw(pageByName("Draft")!.roots[0], "X typed");
    await vi.waitFor(() => expect(pageByName("Draft")).toBeUndefined(), { timeout: 2000 });
    const texts = log.filter((request) => request.pages[0].name === "Owner").map((request) => raws(request.pages[0]));
    expect(texts.at(-1)).toEqual(["owner edited", "X typed"]);
    expect(texts.every((text) => text[0] === "owner edited")).toBe(true);
  });

  it("P3: cycle and delete selection across pages send one request per page", async () => {
    const a = block("TODO A"), b = block("TODO B");
    const { log } = await load([{ ...page("A", []), blocks: [a] }, { ...page("B", []), blocks: [b] }]);
    selectBlock(a.id);
    extendSelectionTo(b.id);
    expect(cycleSelectionTasks()).toBe(true);
    expect(await flushAll()).toBe(true);
    expect(log.map((request) => [request.request, request.pages[0].name]).sort()).toEqual([["submit", "A"], ["submit", "B"]]);
    deleteSelection();
    expect(await flushAll()).toBe(true);
    expect(log.slice(2).map((request) => [request.request, request.pages[0].name]).sort()).toEqual([["submit", "A"], ["submit", "B"]]);
  });

  it("unit cost: measures page bytes and request count for 1 and 60 block drag and carry", async () => {
    const bytes = (dtos: PageDto[]) => dtos.reduce((sum, dto) => sum + new TextEncoder().encode(JSON.stringify(dto)).length, 0);
    for (const count of [1, 60]) for (const intent of ["drag", "carry"] as const) {
      resetStore(); vi.restoreAllMocks(); serial = 0;
      const many = (prefix: string, first = prefix) => Array.from({ length: count }, (_, i) => block(i === 0 ? first : `${prefix} ${i}`));
      const today = journalTitle(new Date()), old = "2026-09-20";
      const source = many("source", intent === "carry" ? "TODO X" : "source");
      const { log } = intent === "drag"
        ? await load([{ ...page("A", []), blocks: source }, { ...page("B", []), blocks: many("destination") }])
        : await load([{ ...page(today, []), kind: "journal", blocks: many("today") }, { ...page(old, []), kind: "journal", blocks: source }]);
      if (intent === "drag") await moveBlock(source[0].id, null, 0, "B");
      else expect(carryUnfinished([old], false, null).moved).toBe(1);
      expect(await flushAll()).toBe(true);
      await vi.waitFor(() => expect(log).toHaveLength(1));
      expect(log.map((request) => request.request)).toEqual(["move"]);
      console.info(`page-host unit cost ${intent} ${count} blocks: ${bytes(log[0].pages)} page bytes in 1 move request (2 pages)`);
    }
  });
});
