// og ADR 0061 (family 9): a page whose edits cannot be saved keeps a
// crash-surviving copy in the app-data draft store, and the next session of the
// graph offers it until the user dismisses it.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend, type Backend } from "./backend";
import { initParser } from "./render/parse";
import { flushAll, isDirty, loadFeed, resetStore, resolveConflict, setRaw } from "./document";
import { markConflict } from "./document/save/engine";
import { dismissEarlierDraft, earlierDrafts, installDraftStore, keepAtSwitch, REFRESH_MS, writeAtRisk } from "./draftStore";
import { bumpGraphEpoch, setGraphMeta } from "./graphSession";
import { setToasts, toasts } from "./toasts";
import type { BlockDto, DraftRecord, GraphMeta } from "./types";

const store = new Map<string, DraftRecord>();
let failing = true;
const block = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const records = () => [...store.values()];

beforeAll(() => initParser());
beforeEach(() => {
  vi.useFakeTimers();
  resetStore();
  setToasts([]);
  store.clear();
  failing = true;
  const api = backend() as Required<Backend>;
  vi.spyOn(api, "storeDraft").mockImplementation(async (r) => { store.set(r.id, structuredClone(r)); });
  vi.spyOn(api, "retireDraft").mockImplementation(async (id) => { store.delete(id); });
  vi.spyOn(api, "loadDrafts").mockImplementation(async () => records());
  vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
    if (failing) throw new Error("disk full");
    return { ok: entries.map((_, i) => `saved-${i}`) };
  });
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue({ id: "pages/P.md", name: "P", title: "P", kind: "page", pre_block: null, rev: "r2", blocks: [block("d1", "disk")] });
  installDraftStore();
  loadFeed([{ id: "pages/P.md", name: "P", title: "P", kind: "page", pre_block: null, rev: "r1", blocks: [block("p1", "first")] } as never]);
});
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

const settle = async () => { await vi.advanceTimersByTimeAsync(REFRESH_MS + 450); };

describe("crash-surviving drafts (og ADR 0061)", () => {
  it("an ordinary save writes nothing to the draft store", async () => {
    failing = false;
    setRaw("p1", "typed");
    await settle();
    await flushAll();
    await settle();
    expect(isDirty("P")).toBe(false);
    expect(backend().storeDraft).not.toHaveBeenCalled();
  });

  it("keeps and refreshes a failed save's draft, and retires it once the page saves", async () => {
    setRaw("p1", "typed");
    await settle();
    expect(records()).toHaveLength(1);
    expect(records()[0]).toMatchObject({ kind: "unsaved", page_name: "P", reason: "save-failed", path: "pages/P.md" });
    expect(records()[0].page.blocks[0].raw).toBe("typed");

    setRaw("p1", "typed more");
    await settle();
    expect(records()[0].page.blocks[0].raw).toBe("typed more");

    failing = false;
    await flushAll();
    await settle();
    expect(records()).toEqual([]);
  });

  it("keeps a conflicted page's draft until the user resolves it", async () => {
    setRaw("p1", "mine");
    markConflict("P");
    await settle();
    expect(records()[0]).toMatchObject({ reason: "conflict" });
    failing = false;
    await resolveConflict("P", "disk");
    await settle();
    expect(records()).toEqual([]);
  });

  it("L14:89: saving while the first crash-safe write is pending retires that late record", async () => {
    let finish!: () => void;
    vi.mocked(backend().storeDraft!).mockImplementationOnce(async (record) => {
      await new Promise<void>((resolve) => { finish = resolve; });
      store.set(record.id, structuredClone(record));
    });
    setRaw("p1", "mine");
    markConflict("P");
    const writing = writeAtRisk();
    await vi.advanceTimersByTimeAsync(0);
    expect(finish).toBeTypeOf("function");
    failing = false;
    await resolveConflict("P", "disk");
    finish();
    await writing;
    await settle();
    expect(records(), "a saved page must not reappear as an unsaved draft after restart").toEqual([]);
    expect(backend().retireDraft).toHaveBeenCalledTimes(1);
  });

  it("after a kill, the next session offers the kept draft until the user dismisses it", async () => {
    setRaw("p1", "typed before the crash");
    await settle();
    await writeAtRisk();
    // A killed process leaves its records; the next process has another session id.
    const kept = records()[0];
    store.clear();
    store.set("earlier:P", { ...kept, id: "earlier:P", session: "earlier" });
    setGraphMeta({ root: "/g", name: "g" } as unknown as GraphMeta);
    bumpGraphEpoch();
    await vi.advanceTimersByTimeAsync(0);
    expect(earlierDrafts().map((r) => r.page.blocks[0].raw)).toEqual(["typed before the crash"]);
    expect(toasts().some((t) => t.action?.label === "Review" && t.sticky)).toBe(true);
    await dismissEarlierDraft("earlier:P");
    expect(store.has("earlier:P")).toBe(false);
    expect(earlierDrafts()).toEqual([]);
    setGraphMeta(null);
  });

  it("a draft kept at a graph switch is offered when that graph reopens in this window (og T4)", async () => {
    setRaw("p1", "typed while the next graph loaded");
    const kept = keepAtSwitch("/g");
    resetStore();
    expect(await kept).toEqual([]);
    expect(vi.mocked(backend().storeDraft!).mock.calls.map((c) => c[1])).toEqual(["/g"]);
    setGraphMeta({ root: "/g", name: "g" } as unknown as GraphMeta);
    bumpGraphEpoch();
    await vi.advanceTimersByTimeAsync(0);
    expect(earlierDrafts().map((r) => r.page.blocks[0].raw)).toEqual(["typed while the next graph loaded"]);
    setGraphMeta(null);
  });

  it("a draft store that cannot be read never blocks open", async () => {
    vi.mocked(backend().loadDrafts!).mockRejectedValueOnce(new Error("unreadable"));
    setGraphMeta({ root: "/g", name: "g" } as unknown as GraphMeta);
    bumpGraphEpoch();
    await vi.advanceTimersByTimeAsync(0);
    expect(earlierDrafts()).toEqual([]);
    setGraphMeta(null);
  });
});
