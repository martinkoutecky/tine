import { describe, expect, it, beforeEach, vi } from "vitest";
import {
  clearAllEditorLeases,
  ensurePageLoaded,
  hasEditorLease,
  markDirty,
  mayReplaceInstance,
  resetStore,
  takeEditorLease,
  doc,
} from "./store";
import { backend } from "./backend";
import type { PageDto } from "./types";

/**
 * GH #304 / GH #254 increment 3, contract rule 2: a page's loaded instance is
 * never replaced while it holds unsaved work.
 *
 * These drive the real store, not a mock of it. The failure being guarded is
 * silent: the replacement succeeds, so nothing errors — the edit is simply gone,
 * and (worse) the dirty mark survives and starts describing the replacement's
 * content, so the next save writes the wrong file's bytes under the user's
 * intent.
 */
const page = (name: string, path: string, raw: string): PageDto => ({
  name,
  kind: "page",
  title: name,
  pre_block: null,
  blocks: [{ raw, children: [] } as never],
  format: "md",
  read_only: false,
  path,
  rev: "rev-" + path,
});

describe("replacing a loaded instance (GH #304)", () => {
  beforeEach(() => {
    resetStore();
    clearAllEditorLeases();
  });

  it("refuses a same-name different-path load while the incumbent is dirty", () => {
    expect(ensurePageLoaded(page("Note", "pages/Note.md", "incumbent"))).toBeNull();
    markDirty("Note");

    const refusal = ensurePageLoaded(page("Note", "pages/other/Note.md", "replacement"));

    expect(refusal).toEqual({ reason: "unsaved-changes", page: "Note" });
    const live = doc.pages.find((p) => p.name === "Note");
    expect(live?.path).toBe("pages/Note.md");
    // The half that makes this data loss rather than a cosmetic bug: the dirty
    // mark must still belong to the content it was raised for.
    expect(doc.byId[live!.roots[0]]?.raw).toBe("incumbent");
  });

  it("refuses while a component holds uncommitted input no store predicate can see", () => {
    expect(ensurePageLoaded(page("Note", "pages/Note.md", "incumbent"))).toBeNull();
    // Not dirty, not conflicted, not saving — this is the page-title rename and
    // the IME-composition shape, where the text lives outside the store entirely.
    const release = takeEditorLease("Note");
    expect(mayReplaceInstance("Note")).toBe(false);

    const refusal = ensurePageLoaded(page("Note", "pages/other/Note.md", "replacement"));
    expect(refusal).toEqual({ reason: "unsaved-changes", page: "Note" });

    // And it must not wedge: releasing the lease lets the request through.
    release();
    expect(hasEditorLease("Note")).toBe(false);
    expect(ensurePageLoaded(page("Note", "pages/other/Note.md", "replacement"))).toBeNull();
    expect(doc.pages.find((p) => p.name === "Note")?.path).toBe("pages/other/Note.md");
  });

  it("keeps leases per component, so one surface cannot clear another's", () => {
    ensurePageLoaded(page("Note", "pages/Note.md", "incumbent"));
    const a = takeEditorLease("Note");
    const b = takeEditorLease("Note");

    a();
    expect(hasEditorLease("Note")).toBe(true);
    a(); // idempotent: a double release must not drop B's lease
    expect(hasEditorLease("Note")).toBe(true);

    b();
    expect(hasEditorLease("Note")).toBe(false);
  });

  it("retires the identity it replaces, and the one it evicts", async () => {
    const { setEditorActivation, editorActivationFor } = await import("./store");
    ensurePageLoaded(page("Note", "pages/Note.md", "incumbent"));
    setEditorActivation("Note", 17);
    const retire = vi.spyOn(backend(), "retireEditorActivation").mockResolvedValue(true);

    // A genuine replacement is a NEW editor. Leaving the old identity live means
    // the incoming editor inherits, under same-path Reuse, a token minted for an
    // editor that was shown a different conflict.
    ensurePageLoaded(page("Note", "pages/other/Note.md", "replacement"));

    expect(retire).toHaveBeenCalledWith("pages/Note.md", 17);
    expect(editorActivationFor("Note")).toBeUndefined();
  });

  it("stamps a deferred block ref once the page actually becomes replaceable", async () => {
    const { persistBlockRefTarget, takeEditorLease } = await import("./store");
    ensurePageLoaded(page("Target", "pages/Target.md", "incumbent"));
    const release = takeEditorLease("Target");
    const read = vi
      .spyOn(backend(), "getPageByPath")
      .mockResolvedValue(page("Target", "pages/other/Target.md", "requested"));

    await persistBlockRefTarget("uuid-1", "Target", "page", "pages/other/Target.md");
    expect(read).toHaveBeenCalledTimes(1);

    // The incumbent is freed by a LEASE RELEASE — a route that produces no save at
    // all. Every earlier design polled on unrelated saves and stranded here. The
    // signal is NOT emitted by hand: the release itself must emit it, or this
    // would pass on a build where nothing is wired.
    release();
    await new Promise((r) => queueMicrotask(() => queueMicrotask(() => r(null))));

    expect(read).toHaveBeenCalledTimes(2);
  });

  it("drops a deferred stamp whose graph went away", async () => {
    const { persistBlockRefTarget, takeEditorLease } = await import("./store");
    ensurePageLoaded(page("Target", "pages/Target.md", "incumbent"));
    const releaseB = takeEditorLease("Target");
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(
      page("Target", "pages/other/Target.md", "requested"),
    );
    await persistBlockRefTarget("uuid-2", "Target", "page", "pages/other/Target.md");

    resetStore();
    const read = vi.spyOn(backend(), "getPageByPath");
    read.mockClear();
    releaseB();
    await new Promise((r) => queueMicrotask(() => queueMicrotask(() => r(null))));

    // A retained request belongs to the graph that deferred it; carried across it
    // would load the old path's page into the replacement graph.
    expect(read).not.toHaveBeenCalled();
  });

  it("re-drives a deferred stamp after a save, once the queue entry is gone", async () => {
    const { persistBlockRefTarget } = await import("./store");
    const { markDirty, flushPage } = await import("./persistence");
    const { loadRoutedPage } = await import("./store");
    // `flushPage` returns early unless the store is armed for persistence.
    loadRoutedPage(page("Target", "pages/Target.md", "incumbent"));
    markDirty("Target");
    const read = vi
      .spyOn(backend(), "getPageByPath")
      .mockResolvedValue(page("Target", "pages/other/Target.md", "requested"));
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");

    await persistBlockRefTarget("uuid-3", "Target", "page", "pages/other/Target.md");
    expect(read).toHaveBeenCalledTimes(1);

    // Announcing from the save's success path — even deferred a microtask — ran
    // while its `saveChain` entry was still present, so the re-verification
    // correctly dropped the event and this stayed at one read.
    await flushPage("Target");
    await new Promise((r) => setTimeout(r, 0));

    expect(read).toHaveBeenCalledTimes(2);
  });

  it("re-drives a deferred stamp when the incumbent is forgotten entirely", async () => {
    const { persistBlockRefTarget, forgetPage } = await import("./store");
    ensurePageLoaded(page("Target", "pages/Target.md", "incumbent"));
    // Dirty, which is what `forgetSaveState` clears — a lease is held by a
    // component and would legitimately survive the page being forgotten.
    markDirty("Target");
    const read = vi
      .spyOn(backend(), "getPageByPath")
      .mockResolvedValue(page("Target", "pages/other/Target.md", "requested"));

    await persistBlockRefTarget("uuid-4", "Target", "page", "pages/other/Target.md");
    expect(read).toHaveBeenCalledTimes(1);

    // The externally-deleted "Use disk version" route, and successful deletePage,
    // both end here. Announcing at the TOP of forgetPage ran while the page was
    // still dirty/conflicted, so the announcement was correctly dropped — and
    // nothing swept afterwards, stranding this request forever.
    forgetPage("Target");
    await new Promise((r) => setTimeout(r, 0));

    expect(read).toHaveBeenCalledTimes(2);
  });

  it("never lets an in-flight retry resurrect a page the user deleted", async () => {
    const { persistBlockRefTarget, deletePage, takeEditorLease, doc: liveDoc } = await import(
      "./store"
    );
    ensurePageLoaded(page("Target", "pages/Target.md", "incumbent"));
    const release = takeEditorLease("Target");

    let releaseRead: (dto: unknown) => void = () => {};
    vi.spyOn(backend(), "getPageByPath").mockReturnValueOnce(
      Promise.resolve(page("Target", "pages/other/Target.md", "requested")) as never,
    );
    await persistBlockRefTarget("uuid-5", "Target", "page", "pages/other/Target.md");

    // Retry read 2 starts...
    vi.spyOn(backend(), "getPageByPath").mockReturnValue(
      new Promise((r) => {
        releaseRead = r as (dto: unknown) => void;
      }) as never,
    );
    const saved = vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");
    vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined as never);
    release();
    await new Promise((r) => setTimeout(r, 0));

    // ...and the user deletes the page WHILE it is pending.
    await deletePage("Target", "page");

    // The read now resolves with PRE-DELETE bytes. Installing them puts the page
    // back, `upsertPage` lifts the tombstone as it does so, and the stamp's own
    // save then recreates the file the user just deleted, with stale content.
    releaseRead(page("Target", "pages/other/Target.md", "requested"));
    await new Promise((r) => setTimeout(r, 0));

    expect(liveDoc.pages.find((p) => p.name === "Target")).toBeUndefined();
    expect(saved).not.toHaveBeenCalled();
  });

  it("still resolves the surviving file when its same-named sibling is deleted", async () => {
    const { persistBlockRefTarget, deletePage, loadRoutedPage } = await import("./store");
    const { markConflict } = await import("./ui");
    loadRoutedPage(page("Target", "pages/Target.md", "incumbent"));
    // CONFLICTED, not merely dirty. `deletePage` flushes a dirty page first, and
    // that save frees the incumbent — so the retry would fire before the delete,
    // install the survivor, and then be removed by `forgetPage`'s name-keyed
    // teardown. A conflicted page is deleted without flushing, which is the
    // ordering this guard is actually about.
    markDirty("Target");
    markConflict("Target");
    const read = vi
      .spyOn(backend(), "getPageByPath")
      .mockResolvedValue(page("Target", "pages/other/Target.md", "survivor"));
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");
    vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined as never);

    // The retained request targets the OTHER file of that name.
    await persistBlockRefTarget("uuid-6", "Target", "page", "pages/other/Target.md");
    expect(read).toHaveBeenCalledTimes(1);

    // Deleting the incumbent FILE must not refuse the survivor. A name-level
    // tombstone dropped both, losing the committed reference's durable target.
    await deletePage("Target", "page", "pages/Target.md");
    // The sweep fires synchronously, but the retry then awaits its own read.
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    // The read alone proves nothing: it happens BEFORE the tombstone guard. What
    // must survive the sibling's deletion is the INSTALL of the surviving file.
    expect(read).toHaveBeenCalledTimes(2);
    expect(doc.pages.find((p) => p.name === "Target")?.path).toBe("pages/other/Target.md");
  });

  it("keeps a deferred stamp when the delete it parked behind fails", async () => {
    const { persistBlockRefTarget, deletePage, takeEditorLease, doc: liveDoc } = await import(
      "./store"
    );
    ensurePageLoaded(page("Target", "pages/Target.md", "incumbent"));
    const release = takeEditorLease("Target");

    const read = vi
      .spyOn(backend(), "getPageByPath")
      .mockResolvedValue(page("Target", "pages/other/Target.md", "survivor"));
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");

    await persistBlockRefTarget("uuid-9", "Target", "page", "pages/other/Target.md");
    expect(read).toHaveBeenCalledTimes(1); // refused by the lease, request retained

    // Retry read 2 in flight.
    let releaseRead: (dto: unknown) => void = () => {};
    read.mockReturnValue(
      new Promise((r) => {
        releaseRead = r as (dto: unknown) => void;
      }) as never,
    );
    release();
    await new Promise((r) => setTimeout(r, 0));

    // A by-name delete of a DUPLICATED page name: core rejects it as ambiguous,
    // so the tombstone goes up and comes back down with nothing deleted.
    let rejectDelete: (e: unknown) => void = () => {};
    vi.spyOn(backend(), "deletePage").mockReturnValue(
      new Promise((_, rej) => {
        rejectDelete = rej;
      }) as never,
    );
    const deleting = deletePage("Target", "page");
    await new Promise((r) => setTimeout(r, 0));

    // The retry resolves while that tombstone is up.
    read.mockResolvedValue(page("Target", "pages/other/Target.md", "survivor"));
    releaseRead(page("Target", "pages/other/Target.md", "survivor"));
    await new Promise((r) => setTimeout(r, 0));

    // Only now does the delete fail. Discarding the request under a tombstone
    // that is about to be lifted throws away an already-committed reference's
    // durable target for a deletion that never happened.
    rejectDelete(new Error("ambiguous page name"));
    expect(await deleting).toBe(false);
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    expect(liveDoc.pages.find((p) => p.name === "Target")?.path).toBe("pages/other/Target.md");
  });

  it("does not carry a deleted file's path into the next graph", async () => {
    const { persistBlockRefTarget, deletePage, resetStore, doc: liveDoc } = await import("./store");
    ensurePageLoaded(page("Target", "pages/old/Target.md", "old"));
    vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined as never);
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");
    await deletePage("Target", "page", "pages/old/Target.md");

    resetStore();

    // New graph, same page name, different file. A read is in flight...
    let releaseRead: (dto: unknown) => void = () => {};
    vi.spyOn(backend(), "getPageByPath").mockReturnValue(
      new Promise((r) => {
        releaseRead = r as (dto: unknown) => void;
      }) as never,
    );
    void persistBlockRefTarget("uuid-10", "Target", "page", "pages/new/Target.md");
    await new Promise((r) => setTimeout(r, 0));

    // ...when the user deletes this page by NAME, which means every file of that
    // name. A path left over from the previous graph's tombstone would make the
    // guard compare against the wrong file and admit these pre-delete bytes.
    await deletePage("Target", "page");
    releaseRead(page("Target", "pages/new/Target.md", "new"));
    await new Promise((r) => setTimeout(r, 0));

    expect(liveDoc.pages.find((p) => p.name === "Target")).toBeUndefined();
  });

  it("still stamps a pathless request when a same-named stray is deleted", async () => {
    // Block autocomplete supplies no path — it knows the page name and nothing
    // else. A tombstone for some OTHER file of that name must not park such a
    // request forever: "I cannot prove which file this is" is a reason to read,
    // not a reason to refuse.
    const { persistBlockRefTarget, deletePage, loadRoutedPage } = await import("./store");
    const { markConflict } = await import("./ui");
    loadRoutedPage(page("Target", "pages/stray/Target.md", "incumbent"));
    markDirty("Target");
    markConflict("Target"); // conflicted, so the delete does not flush it first

    const survivor = page("Target", "pages/Target.md", "survivor");
    const read = vi.spyOn(backend(), "getPage").mockResolvedValue(survivor as never);
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");
    vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined as never);

    // No path: the request can only name the page.
    await persistBlockRefTarget("uuid-11", "Target", "page");
    expect(read).toHaveBeenCalledTimes(1); // refused by the conflicted incumbent

    // The stray is deleted by exact path. The survivor is untouched.
    await deletePage("Target", "page", "pages/stray/Target.md");
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    // The read IS the observable here: the defect was the retry returning before
    // it, so the request could never reach the survivor at all. Its own page
    // must then be the one loaded.
    expect(read).toHaveBeenCalledTimes(2);
    expect(doc.pages.find((p) => p.name === "Target")?.path).toBe("pages/Target.md");
  });

  it("never lets an unknown file count as covered by a tombstone", async () => {
    // The invariant behind removing the observedPath cache. A wait may skip its
    // read ONLY when the request itself names the deleted file. Anything weaker
    // strands the request: nothing announces that an UNLOADED page was recreated
    // (no upsert, so no tombstone is ever lifted), so a request that stopped
    // reading on the strength of a remembered path would never discover it.
    const { tombstone, tombstoneCovers, isTombstonedFile, untombstone } = await import(
      "./persistence"
    );
    tombstone("Target", "pages/Target.md");

    expect(tombstoneCovers("Target", "pages/Target.md")).toBe(true);
    expect(tombstoneCovers("Target", "pages/new/Target.md")).toBe(false);
    // The unknown file is the case the two predicates must answer differently:
    // refuse to INSTALL bytes that cannot be identified, but never refuse to READ.
    expect(tombstoneCovers("Target", undefined)).toBe(false);
    expect(isTombstonedFile("Target", undefined)).toBe(true);

    // A pathless tombstone genuinely covers everything of that name.
    untombstone("Target");
    tombstone("Target");
    expect(tombstoneCovers("Target", undefined)).toBe(true);
    expect(tombstoneCovers("Target", "pages/anything.md")).toBe(true);
    untombstone("Target");
  });

  it("tells the user which page is holding the file back", async () => {
    // A refused route that leaves the surface unchanged is a trap, not a
    // safeguard: the user asked for a file, got no file and no explanation, and
    // has nothing to act on. The native journey was carrying this contract
    // alone, and it is quarantined (no UI opens a chosen file by path, so it
    // cannot drive the refusal), which left the promise untested.
    const { loadRoutedPage } = await import("./store");
    const { toasts, setToasts } = await import("./ui");
    setToasts([]);

    expect(loadRoutedPage(page("Note", "pages/Note.md", "incumbent"))).toBeNull();
    markDirty("Note");
    expect(toasts()).toHaveLength(0);

    const refusal = loadRoutedPage(page("Note", "pages/other/Note.md", "requested"));

    expect(refusal).toEqual({ reason: "unsaved-changes", page: "Note" });
    const said = toasts().map((t) => t.message);
    // Names the page, and says what resolves it — an error with neither is not
    // actionable. The wording itself is free to change.
    expect(said.some((m) => m.includes("Note") && /save or resolve/i.test(m))).toBe(true);
    setToasts([]);
  });

  it("allows the replacement when the incumbent is clean", () => {
    expect(ensurePageLoaded(page("Note", "pages/Note.md", "incumbent"))).toBeNull();
    expect(ensurePageLoaded(page("Note", "pages/other/Note.md", "replacement"))).toBeNull();
    expect(doc.pages.find((p) => p.name === "Note")?.path).toBe("pages/other/Note.md");
  });
});
