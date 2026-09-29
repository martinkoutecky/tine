// og 21a (Concord 8e): a live editor draft whose save was refused because its
// file changed on disk is reviewed and resolved at the page, through the
// guarded backend command, and survives a kill as a `live-conflict` capsule.
// Real document engine and draft store; the backend is stubbed. Synthetic content.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend, type Backend } from "../backend";
import { invalidateBinding } from "../binding";
import { initParser } from "../render/parse";
import { flushAll, flushPage, installExternalChangeUiHandler, isConflicted, isDirty, loadFeed, pageByName, resetStore, setRaw } from "../document";
import { conflictReason } from "../document/save/engine";
import { doc } from "../document/model";
import { earlierDrafts, installDraftStore, REFRESH_MS, writeAtRisk } from "../draftStore";
import { bumpGraphEpoch, setGraphMeta } from "../graphSession";
import { liveConflictForPage, liveConflictObjects } from "../liveConflicts";
import { ConflictOverview } from "./ConflictOverview";
import { ConflictQueueBadge } from "./Sidebar";
import type { PaneRouter } from "../router";
import { setToasts, toasts } from "../toasts";
import { PageConflictResolution } from "./ConflictResolution";
import type { BlockDto, ConflictObject, DiffRow, DraftRecord, GraphMeta, PageDto, SyncConflictDiff } from "../types";

const PATH = "pages/P.md";
const store = new Map<string, DraftRecord>();
const records = () => [...store.values()];
const block = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const page = (rev: string, raw: string) => ({ id: PATH, name: "P", title: "P", kind: "page" as const, pre_block: null, rev, blocks: [block("p1", raw)] });
const view = (text: string) => ({ uuid: "", text, child_count: 0 });
const rows: DiffRow[] = [{ id: "0", kind: "modified", mine: view("mine"), theirs: view("theirs"), children: [] }];
const review = (conflictRev: string): SyncConflictDiff => ({ base_rev: "r1", conflict_rev: conflictRev, rows,
  mine_pre: null, theirs_pre: null, pre_differs: false, blocks_identical: false });
const raw = () => doc.byId[pageByName("P")!.roots[0]]?.raw;

let saveFails = true;
let api: Required<Backend>;
beforeAll(() => initParser());
beforeEach(() => {
  vi.useFakeTimers();
  resetStore();
  setToasts([]);
  store.clear();
  saveFails = true;
  api = backend() as Required<Backend>;
  vi.spyOn(api, "storeDraft").mockImplementation(async (r) => { store.set(r.id, structuredClone(r)); });
  vi.spyOn(api, "retireDraft").mockImplementation(async (id) => { store.delete(id); });
  vi.spyOn(api, "loadDrafts").mockImplementation(async () => records());
  // Another editor wrote r2 after the page loaded at r1: every guarded save refuses.
  vi.spyOn(api, "savePages").mockImplementation(async (entries) => saveFails
    ? { failed: { index: 0, family: "conflict", diskRev: "r2", undoFailed: [] } }
    : { ok: entries.map(() => "r9") });
  vi.spyOn(api, "liveConflictDiff").mockImplementation(async () => review("r2"));
  installDraftStore();
  loadFeed([page("r1", "first")] as never);
});
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); setGraphMeta(null); document.body.innerHTML = ""; });

const settle = async () => { await vi.advanceTimersByTimeAsync(REFRESH_MS + 450); };
const tick = async () => { for (let i = 0; i < 8; i++) await vi.advanceTimersByTimeAsync(0); };

async function conflictedDraft(text = "mine") {
  setRaw("p1", text);
  expect(await flushPage("P")).toBe(false);
  expect(conflictReason("P")).toMatchObject({ kind: "disk-changed", observedRev: "r2" });
}

function mount(conflict: ConflictObject) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <PageConflictResolution conflict={conflict} />, host);
  return { host, dispose };
}
const apply = (host: HTMLElement) => (host.querySelector(".settings-btn-primary") as HTMLButtonElement).click();

/** Simulate a kill: this session's records become an earlier session's, and
 *  the page is reopened from disk (clean, at the external revision). */
async function killAndReopen(disk: PageDto & { id: string }) {
  await writeAtRisk();
  const kept = records();
  store.clear();
  for (const r of kept) store.set(`earlier:${r.page_name}`, { ...r, id: `earlier:${r.page_name}`, session: "earlier" });
  resetStore();
  if (disk) loadFeed([disk] as never);
  setGraphMeta({ root: "/g", name: "g" } as unknown as GraphMeta);
  bumpGraphEpoch();
  await tick();
}

describe("Concord live-draft conflicts (og 8e)", () => {
  it("reviews the draft against disk and resolves through the guarded command, then installs the result", async () => {
    await conflictedDraft();
    const conflict = liveConflictForPage("P", PATH)!;
    expect(conflict).toMatchObject({ source: "live-save", page_path: PATH, live: { restored: false } });
    const resolve = vi.spyOn(api, "resolveLiveConflict").mockImplementation(async (_p, draft) =>
      ({ ...draft, rev: "r3", blocks: [block("p1", "mine + theirs")] }));
    const { host, dispose } = mount(conflict);
    await tick();
    expect(api.liveConflictDiff).toHaveBeenCalledWith(PATH, expect.objectContaining({ name: "P" }), "r1");
    expect(vi.mocked(api.liveConflictDiff).mock.calls[0][1].blocks[0].raw).toBe("mine");
    apply(host);
    await tick();
    expect(resolve).toHaveBeenCalledTimes(1);
    const [path, draft, baseRev, conflictRev] = resolve.mock.calls[0];
    expect([path, draft.blocks[0].raw, baseRev, conflictRev]).toEqual([PATH, "mine", "r1", "r2"]);
    expect(raw()).toBe("mine + theirs");
    expect(isConflicted("P") || isDirty("P")).toBe(false);
    // The committed revision is the new baseline: the next save guards on r3.
    saveFails = false;
    setRaw("p1", "after");
    await flushAll();
    expect(vi.mocked(api.savePages).mock.calls.at(-1)![0][0].baseRev).toBe("r3");
    dispose();
  });

  it("a draft typed after the review needs a fresh review and is never replaced", async () => {
    await conflictedDraft();
    const resolve = vi.spyOn(api, "resolveLiveConflict");
    const { host, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    setRaw("p1", "mine, typed after the review");
    apply(host);
    await tick();
    expect(resolve).not.toHaveBeenCalled();
    expect(api.liveConflictDiff).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.liveConflictDiff).mock.calls[1][1].blocks[0].raw).toBe("mine, typed after the review");
    expect(raw()).toBe("mine, typed after the review");
    expect(toasts().some((t) => /draft changed/.test(t.message))).toBe(true);
    dispose();
  });

  it("a newer disk write refuses the apply, writes nothing and refreshes the review", async () => {
    await conflictedDraft();
    vi.spyOn(api, "resolveLiveConflict").mockRejectedValue("conflict");
    const { host, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    apply(host);
    await tick();
    expect(api.liveConflictDiff).toHaveBeenCalledTimes(2);
    expect(raw()).toBe("mine");
    expect(isConflicted("P")).toBe(true);
    dispose();
  });

  it("edits typed while the resolve was in flight are kept and conflict against the resolved revision", async () => {
    await conflictedDraft();
    vi.spyOn(api, "resolveLiveConflict").mockImplementation(async (_p, draft) => {
      setRaw("p1", "typed during the write");
      return { ...draft, rev: "r3", blocks: [block("p1", "resolved")] };
    });
    const { host, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    apply(host);
    await tick();
    expect(raw()).toBe("typed during the write");
    expect(conflictReason("P")).toMatchObject({ kind: "disk-changed", observedRev: "r3" });
    dispose();
  });

  it("kill and reopen: the capsule survives, reviews against its base, applies at the reviewed revision and retires", async () => {
    await conflictedDraft("mine before the kill");
    await settle();
    await writeAtRisk();
    expect(records()[0]).toMatchObject({ kind: "live-conflict", base_rev: "r1", observed_rev: "r2", path: PATH });
    await killAndReopen(page("r2", "theirs on disk"));
    expect(raw()).toBe("theirs on disk");
    expect(earlierDrafts()).toHaveLength(1);
    const conflict = liveConflictForPage("P", PATH)!;
    expect(conflict.live).toMatchObject({ restored: true, record_id: "earlier:P" });
    const resolve = vi.spyOn(api, "resolveLiveConflict").mockImplementation(async (_p, draft) => ({ ...draft, rev: "r3" }));
    // The open page shows the result through the ordinary external-change rule.
    installExternalChangeUiHandler(() => ({ pageOpen: () => true, journalsOpen: false, leaveRemovedPage() {}, restartJournalFeed() {} }));
    vi.spyOn(api, "getPage").mockResolvedValue(page("r3", "merged on disk"));
    const { host, dispose } = mount(conflict);
    await tick();
    expect(vi.mocked(api.liveConflictDiff).mock.calls.at(-1)!.slice(2)).toEqual(["r1"]);
    apply(host);
    await tick();
    const [, draft, baseRev, conflictRev] = resolve.mock.calls[0];
    expect([draft.blocks[0].raw, baseRev, conflictRev]).toEqual(["mine before the kill", "r1", "r2"]);
    expect(store.has("earlier:P")).toBe(false);
    expect(raw()).toBe("merged on disk");
    dispose();
  });

  it("GH #541: a kept draft whose page is not open resolves through Apply alone", async () => {
    await conflictedDraft("detached draft");
    await settle();
    await killAndReopen(undefined as never);
    expect(pageByName("P")).toBeUndefined();
    const conflict = liveConflictForPage("P", undefined)!;
    expect(conflict).toMatchObject({ page_path: PATH, live: { restored: true } });
    const resolve = vi.spyOn(api, "resolveLiveConflict").mockImplementation(async (_p, draft) => ({ ...draft, rev: "r3" }));
    const { host, dispose } = mount(conflict);
    await tick();
    apply(host);
    await tick();
    expect(resolve.mock.calls[0][1].blocks[0].raw).toBe("detached draft");
    expect(store.size).toBe(0);
    dispose();
  });

  it("a resolve that finishes after a graph switch retires nothing in the next graph", async () => {
    await conflictedDraft("graph A draft");
    await settle();
    await killAndReopen(page("r2", "disk"));
    let finish!: (p: PageDto) => void;
    vi.spyOn(api, "resolveLiveConflict").mockImplementation((_p, draft) => new Promise((r) => { finish = () => r({ ...draft, rev: "r3" }); }));
    const { host, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    apply(host);
    await tick();
    // Graph B has its own kept draft of a page with the same name.
    invalidateBinding();
    store.set("earlier:P", { ...records()[0] ?? {}, id: "earlier:P" } as DraftRecord);
    finish(page("r3", "x"));
    await tick();
    expect(api.retireDraft).not.toHaveBeenCalled();
    expect(store.has("earlier:P")).toBe(true);
    dispose();
  });

  it("the sidebar badge counts open and restored live drafts (master: one combined queue)", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <ConflictQueueBadge />, host);
    const badge = () => host.querySelector(".conflict-queue-badge")?.textContent ?? null;
    expect(badge()).toBeNull();
    await conflictedDraft("open draft");
    await tick();
    expect(badge()).toBe("1 conflict");
    await settle();
    await killAndReopen(page("r2", "theirs on disk"));
    expect(badge()).toBe("1 conflict");
    dispose();
  });

  it("a restored draft's Apply saves the reopened page's newer edits, re-reviews, then resolves against them", async () => {
    await conflictedDraft("kept before the kill");
    await settle();
    await killAndReopen(page("r2", "theirs on disk"));
    let diskRev = "r2";
    saveFails = false;
    vi.mocked(api.savePages).mockImplementation(async (entries) => { diskRev = "r9"; return { ok: entries.map(() => "r9") }; });
    vi.mocked(api.liveConflictDiff).mockImplementation(async () => review(diskRev));
    const resolve = vi.spyOn(api, "resolveLiveConflict").mockImplementation(async (_p, draft) => ({ ...draft, rev: "r10" }));
    installExternalChangeUiHandler(() => ({ pageOpen: () => true, journalsOpen: false, leaveRemovedPage() {}, restartJournalFeed() {} }));
    vi.spyOn(api, "getPage").mockResolvedValue(page("r10", "merged on disk"));
    const { host, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    setRaw("p1", "typed after the reopen");
    apply(host);
    await tick();
    expect(resolve).not.toHaveBeenCalled();
    expect(isDirty("P")).toBe(false);
    expect(toasts().at(-1)?.message).toContain("newer edits to this page were saved");
    expect(vi.mocked(api.liveConflictDiff).mock.calls.length).toBeGreaterThanOrEqual(2);
    apply(host);
    await tick();
    expect(resolve).toHaveBeenCalledTimes(1);
    const [, draft, , conflictRev] = resolve.mock.calls[0];
    expect([draft.blocks[0].raw, conflictRev]).toEqual(["kept before the kill", "r9"]);
    dispose();
  });

  it("22a: the overview lists open and restored drafts under Unsaved drafts, and opens the page to review", async () => {
    await conflictedDraft("open draft");
    expect(liveConflictObjects().map((c) => [c.page_name, c.live?.restored])).toEqual([["P", false]]);
    await settle();
    await killAndReopen(page("r2", "theirs on disk"));
    expect(liveConflictObjects().map((c) => [c.page_name, c.live?.restored])).toEqual([["P", true]]);
    const opened: unknown[] = [];
    vi.spyOn(api, "listSyncConflicts").mockResolvedValue([]);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <ConflictOverview router={{ openPageTarget: (t: unknown) => opened.push(t) } as unknown as PaneRouter} />, host);
    const group = host.querySelector('[aria-label="Unsaved drafts"]')!;
    expect(group.textContent).toContain("kept draft from an earlier session");
    (group.querySelector(".conflict-overview-open") as HTMLButtonElement).click();
    expect(opened).toEqual([{ name: "P", pageKind: "page", path: PATH }]);
    dispose();
  });
});

