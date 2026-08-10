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

  it("allows the replacement when the incumbent is clean", () => {
    expect(ensurePageLoaded(page("Note", "pages/Note.md", "incumbent"))).toBeNull();
    expect(ensurePageLoaded(page("Note", "pages/other/Note.md", "replacement"))).toBeNull();
    expect(doc.pages.find((p) => p.name === "Note")?.path).toBe("pages/other/Note.md");
  });
});
