// og storage.qnt guarantee B (typed input is always durably promised by a
// draft or by a Published save covering that exact buffer version) and mutant
// MS: only a Published reply for the buffer version a draft covers may retire
// that draft. Driven through the literal edit (setRaw → markDirty → debounced
// save → backend.savePages), watcher (applyGraphChange) and draft-store
// (installDraftStore → backend.storeDraft / retireDraft) paths; the backend is
// the only fake. Conformance map GAP-1 / GAP-2, Martin's ruling 2026-10-05 #4.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend, type Backend, type GraphChange, type SavePageEntry } from "./backend";
import { initParser } from "./render/parse";
import { applyGraphChange, flushAll, flushPage, installPageIdentityNavigation, setPageProperty, installExternalChangeUiHandler, isConflicted, isDirty, loadFeed, moveBlock, pageByName, resetStore, setRaw } from "./document";
import { baseRevFor, forceSave } from "./document/save/engine";
import { doc } from "./document/model";
import { installDraftStore, REFRESH_MS, writeAtRisk } from "./draftStore";
import { setToasts, toasts } from "./toasts";
import type { BlockDto, DraftRecord, PageDto } from "./types";

const store = new Map<string, DraftRecord>();
const records = () => [...store.values()];
const draftText = () => records().map((r) => r.page.blocks.map((b) => b.raw).join("|"));
const block = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const page = (name: string, rev: string, blocks: BlockDto[]): PageDto & { id: string; rev: string } =>
  ({ id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev, blocks });
const gate = () => {
  let open!: () => void;
  const promise = new Promise<void>((resolve) => { open = resolve; });
  return { promise, open };
};

beforeAll(() => initParser());
beforeEach(() => {
  vi.useFakeTimers();
  resetStore();
  setToasts([]);
  store.clear();
  const api = backend() as Required<Backend>;
  vi.spyOn(api, "storeDraft").mockImplementation(async (r) => { store.set(r.id, structuredClone(r)); return null; });
  vi.spyOn(api, "retireDraft").mockImplementation(async (id) => { store.delete(id); return null; });
  vi.spyOn(api, "loadDrafts").mockImplementation(async () => ({ drafts: records(), set_aside: null }));
  installDraftStore();
});
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

describe("MS: a Published reply retires risk only for the buffer version it covers", () => {
  it("doSave: an edit typed while the retry is in flight keeps its draft after the retry publishes", async () => {
    loadFeed([page("P", "r1", [block("p1", "v1")]) as never]);
    const slow = gate();
    const saved: string[] = [];
    let calls = 0;
    vi.spyOn(backend(), "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
      calls += 1;
      if (calls === 1) throw new Error("io:Full");
      if (calls === 2) await slow.promise;
      saved.push(entries[0].page.blocks[0].raw);
      return { ok: entries.map(() => `saved-${calls}`) };
    });
    setRaw("p1", "v2");
    await vi.advanceTimersByTimeAsync(450);   // first save fails: at risk, retry armed
    await vi.advanceTimersByTimeAsync(150);   // the retry of v2 is in flight
    expect(calls).toBe(2);
    setRaw("p1", "v3");                        // typed while v2 is being written
    await writeAtRisk();
    expect(draftText()).toEqual(["v3"]);
    slow.open();                               // v2 Published
    await vi.advanceTimersByTimeAsync(0);
    expect(saved).toEqual(["v2"]);
    expect(isDirty("P")).toBe(true);
    expect(draftText(), "v3 exists only in memory unless its draft stays").toEqual(["v3"]);
    // v3's own Published reply retires it.
    await vi.advanceTimersByTimeAsync(REFRESH_MS + 450);
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(saved).toEqual(["v2", "v3"]);
    expect(records()).toEqual([]);
  });

  it("forceSave (Keep mine): an edit typed during the overwrite keeps the conflict draft", async () => {
    loadFeed([page("P", "r1", [block("p1", "v1")]) as never]);
    const slow = gate();
    let calls = 0;
    vi.spyOn(backend(), "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
      calls += 1;
      if (calls === 1) throw new Error("conflict");
      await slow.promise;
      return { ok: entries.map(() => `saved-${calls}`) };
    });
    setRaw("p1", "v2");
    await vi.advanceTimersByTimeAsync(450);
    expect(isConflicted("P")).toBe(true);
    await writeAtRisk();
    const keeping = forceSave("P");
    await vi.advanceTimersByTimeAsync(0);
    setRaw("p1", "v3");
    await writeAtRisk();
    slow.open();
    await keeping;
    await vi.advanceTimersByTimeAsync(0);
    expect(isConflicted("P")).toBe(false);
    expect(draftText()).toEqual(["v3"]);
    await vi.advanceTimersByTimeAsync(REFRESH_MS + 450);
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(records()).toEqual([]);
  });

  it("runGroup: a cross-page move whose retry publishes while the source is edited keeps the source's draft", async () => {
    loadFeed([page("A", "ra", [block("x", "moved"), block("a2", "stays")]) as never, page("B", "rb", [block("b1", "b")]) as never]);
    const slow = gate();
    let calls = 0;
    vi.spyOn(backend(), "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
      calls += 1;
      // The source page's write fails with a disk error: it is the page at risk.
      if (calls === 1) return { failed: { index: entries.findIndex((entry) => entry.page.name === "A"), family: "io", undoFailed: [] } };
      if (calls === 2) await slow.promise;
      return { ok: entries.map((_, i) => `saved-${calls}-${i}`) };
    });
    await moveBlock("x", null, 0, "B");
    await vi.advanceTimersByTimeAsync(450);    // the group write fails: both pages at risk
    expect(calls).toBe(1);
    const retry = flushPage("B");              // Retry saving: the group is written again
    await vi.advanceTimersByTimeAsync(0);
    expect(calls).toBe(2);
    setRaw("a2", "typed during the group write");
    await writeAtRisk();
    expect(records().map((r) => r.page_name)).toEqual(["A"]);
    slow.open();
    await retry;
    await vi.advanceTimersByTimeAsync(0);
    expect(isDirty("A")).toBe(true);
    expect(records().find((r) => r.page_name === "A")?.page.blocks.map((b) => b.raw)).toEqual(["typed during the group write"]);
    await vi.advanceTimersByTimeAsync(REFRESH_MS + 450);
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(records()).toEqual([]);
  });
});

describe("a refused crash-safe write (the store's 64-page / 8 MiB bound, a disk error)", () => {
  it("is a sticky error for every page it refuses, and the page stays at risk until a write lands", async () => {
    loadFeed([page("P", "r1", [block("p1", "p")]) as never, page("Q", "r1", [block("q1", "q")]) as never]);
    vi.spyOn(backend(), "savePages").mockRejectedValue(new Error("conflict"));
    vi.mocked(backend().storeDraft!).mockRejectedValue(new Error("the draft store keeps at most 64 pages"));
    setRaw("p1", "typed on P");
    await vi.advanceTimersByTimeAsync(450);
    await writeAtRisk();
    setRaw("q1", "typed on Q");
    await vi.advanceTimersByTimeAsync(450);
    await writeAtRisk();
    const refusals = toasts().filter((t) => t.message.includes("crash-safe copy"));
    expect(refusals.map((t) => [t.kind, t.sticky, /“(.)”/.exec(t.message)?.[1]])).toEqual([["error", true, "P"], ["error", true, "Q"]]);
    // Room again: the next refresh writes both drafts (they stayed at risk).
    vi.mocked(backend().storeDraft!).mockImplementation(async (r) => { store.set(r.id, structuredClone(r)); return null; });
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(records().map((r) => r.page_name).sort()).toEqual(["P", "Q"]);
  });
});

describe("a title rename while at risk moves the risk, never retires it", () => {
  it("the renamed page keeps a draft of the text typed while its title save was in flight", async () => {
    installPageIdentityNavigation(() => {});
    loadFeed([{ ...page("P", "r1", [block("body", "body")]), pre_block: "title:: P" } as never]);
    const slow = gate();
    let calls = 0;
    vi.spyOn(backend(), "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
      calls += 1;
      if (calls === 1) throw new Error("io:Full");
      if (calls === 2) await slow.promise;
      return { ok: entries.map(() => `saved-${calls}`) };
    });
    setPageProperty("P", "title", "Q");
    await vi.advanceTimersByTimeAsync(450);
    await vi.advanceTimersByTimeAsync(150);
    expect(calls).toBe(2);
    const body = pageByName("P")!.roots.find((id) => doc.byId[id].raw === "body")!;
    setRaw(body, "typed while the title saved");
    await writeAtRisk();
    expect(records().map((r) => r.page_name)).toEqual(["P"]);
    slow.open();
    await vi.advanceTimersByTimeAsync(0);
    expect(pageByName("Q")).toBeTruthy();
    expect(isDirty("Q")).toBe(true);
    await writeAtRisk();
    expect(records().map((r) => [r.page_name, r.page.blocks.at(-1)?.raw])).toEqual([["Q", "typed while the title saved"]]);
    await vi.advanceTimersByTimeAsync(REFRESH_MS + 450);
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(records()).toEqual([]);
  });
});

describe("watcher observations on a page with unsaved input", () => {
  const NAME = "Synced";
  let disk: (PageDto & { id: string; rev: string }) | null = null;
  const event = (patch: Partial<GraphChange>): GraphChange => ({ name: NAME, kind: "page", created: false, removed: false, ...patch });
  const raws = () => pageByName(NAME)?.roots.map((id) => doc.byId[id].raw) ?? [];
  beforeEach(() => {
    disk = null;
    vi.spyOn(backend(), "getPage").mockImplementation(async () => disk as never);
    vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => disk as never);
    installExternalChangeUiHandler(() => ({ pageOpen: () => true, journalsOpen: false, leaveRemovedPage: () => {}, restartJournalFeed: () => {} }));
  });

  it("GAP-2: the conflict lift keeps the draft until the re-armed save publishes", async () => {
    loadFeed([page(NAME, "rev-1", [block("b1", "original")]) as never]);
    setRaw("b1", "mine");
    await applyGraphChange(event({ removed: true }));
    expect(isConflicted(NAME)).toBe(true);
    await writeAtRisk();
    expect(draftText()).toEqual(["mine"]);
    const slow = gate();
    vi.spyOn(backend(), "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
      await slow.promise;
      return { ok: entries.map(() => "rev-2") };
    });
    disk = page(NAME, "rev-1", [block("d1", "original")]);
    await applyGraphChange(event({ created: true }));   // back to the baseline: lifted
    expect(isConflicted(NAME)).toBe(false);
    await vi.advanceTimersByTimeAsync(450);             // the re-armed save is in flight
    expect(draftText(), "the lift must not retire the only durable copy of the frozen edit").toEqual(["mine"]);
    slow.open();
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(records()).toEqual([]);
    expect(raws()).toEqual(["mine"]);
  });

  it("ruling #4: bytes equal to the buffer advance the base silently, and the input stays Tine's to save", async () => {
    loadFeed([page(NAME, "rev-1", [block("b1", "original")]) as never]);
    const saves = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["rev-3"] } as never);
    setRaw("b1", "mine");
    // A sync client (or another device) delivers the same bytes before Tine's save.
    disk = page(NAME, "rev-2", [block("other-id", "mine")]);
    await applyGraphChange(event({}));
    expect(isConflicted(NAME), "equal bytes are not a conflict").toBe(false);
    expect(baseRevFor(NAME)).toBe("rev-2");
    expect(isDirty(NAME)).toBe(true);
    await expect(flushAll()).resolves.toBe(true);
    expect(saves.mock.calls.at(-1)![0][0].baseRev).toBe("rev-2");
    expect(raws()).toEqual(["mine"]);
  });

  it("ruling #4: a conflicted page whose file now holds the buffer's bytes stays at risk with its draft until it saves", async () => {
    loadFeed([page(NAME, "rev-1", [block("b1", "original")]) as never]);
    setRaw("b1", "mine");
    disk = page(NAME, "rev-2", [block("d1", "theirs")]);
    await applyGraphChange(event({}));
    expect(isConflicted(NAME)).toBe(true);
    await writeAtRisk();
    expect(draftText()).toEqual(["mine"]);
    const slow = gate();
    vi.spyOn(backend(), "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
      await slow.promise;
      return { ok: entries.map(() => "rev-4") };
    });
    disk = page(NAME, "rev-3", [block("d2", "mine")]);
    await applyGraphChange(event({}));
    expect(isConflicted(NAME)).toBe(false);
    expect(baseRevFor(NAME)).toBe("rev-3");
    await vi.advanceTimersByTimeAsync(450);
    expect(draftText()).toEqual(["mine"]);
    slow.open();
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(records()).toEqual([]);
  });

  it("ruling #4: different bytes still raise the conflict", async () => {
    loadFeed([page(NAME, "rev-1", [block("b1", "original")]) as never]);
    setRaw("b1", "mine");
    disk = page(NAME, "rev-2", [block("d1", "mine, then theirs")]);
    await applyGraphChange(event({}));
    expect(isConflicted(NAME)).toBe(true);
    expect(baseRevFor(NAME)).toBe("rev-1");
  });
});
