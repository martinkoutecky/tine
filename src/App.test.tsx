import { afterEach, describe, expect, it, vi } from "vitest";
import { backend, SaveConflictError } from "./backend";
import {
  acceptColdReturnManagedStorage,
  handleGraphChange,
  handleSparseV2Changed,
  installMobileExternalLinkHandler,
  safeClose,
} from "./App";
import { resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import {
  doc,
  pageByName,
  reloadPage,
  resetStore,
  setBlockMoving,
  setDoc,
  holdPageMutationUi,
  takeEditorLease,
  type FeedPage,
  type Node as StoreNode,
} from "./store";
import { endEdit, startEditing } from "./editorController";
import { clearConflict, isConflicted, pageInventoryRev } from "./ui";
import { flushPage, forceSave, isDirty, markDirty, resetSaveState } from "./persistence";
import { managedStorageRuntime } from "./managedStorageRuntime";
import {
  applyHeldExternalChange,
  clearHeldExternalChanges,
  dismissHeldExternalChange,
  heldExternalChangeCount,
  heldExternalChangeFor,
  setConflictPolicyAlwaysAskForTest,
} from "./conflictPolicy";
import type { SparseV2CancelResult } from "./types";

function addAnchor(href: string): HTMLAnchorElement {
  const a = document.createElement("a");
  a.href = href;
  a.textContent = href;
  document.body.appendChild(a);
  return a;
}

function click(el: Element): MouseEvent {
  const event = new MouseEvent("click", { bubbles: true, cancelable: true });
  el.dispatchEvent(event);
  return event;
}

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
  managedStorageRuntime.clear();
  resetStore();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
});

describe("cold managed recovery route installation", () => {
  it("identifies the affected page before asking to discard a failed save (GH #540)", async () => {
    setDoc({ pages: [page("Days of notes", "page", ["draft"])], feed: [],
      byId: { draft: node("draft", "Days of notes") }, loaded: true });
    markDirty("Days of notes");
    vi.spyOn(backend(), "savePage").mockRejectedValue(new Error("disk full"));
    const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    safeClose.reset();
    await expect(safeClose.prepare()).resolves.toBe("rejected");
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("Days of notes"), "Unsaved changes");
    expect(isDirty("Days of notes")).toBe(true);
  });
  it("binds the native Direct admission together with the returned generation", () => {
    const result: SparseV2CancelResult = {
      binding_generation: 42,
      recovery_statement: "managed state archived",
      status: {
        state: "legacy_default",
        runtime: null,
        can_activate: true,
        can_retry: false,
        can_cancel: false,
        cancel_reason: null,
        binding_generation: 42,
        application_page_admission: {
          binding_generation: 42,
          authority: "direct",
        },
      },
    };

    acceptColdReturnManagedStorage(result);

    expect(managedStorageRuntime.snapshot().bindingGeneration).toBe(42);
    expect(managedStorageRuntime.snapshot().applicationPageAdmission).toEqual({
      binding_generation: 42,
      authority: "direct",
    });
    expect(managedStorageRuntime.snapshot().status).toBe(result.status);
  });
});

function page(name: string, kind: "page" | "journal", roots: string[]): FeedPage {
  return { name, kind, title: name, preBlock: null, roots, format: "md", readOnly: false, guide: false };
}

function node(id: string, pageName: string): StoreNode {
  return { id, raw: "loaded elsewhere", collapsed: false, parent: null, page: pageName, children: [] };
}

describe("mobile external link delegation", () => {
  it("opens external links through the OS browser on Android", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("android");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const a = addAnchor("https://x.test/path");
      const targetClick = vi.fn();
      a.addEventListener("click", targetClick);

      const event = click(a);

      expect(event.defaultPrevented).toBe(true);
      expect(targetClick).not.toHaveBeenCalled();
      expect(openExternal).toHaveBeenCalledTimes(1);
      expect(openExternal).toHaveBeenCalledWith("https://x.test/path");
    } finally {
      uninstall();
    }
  });

  it("does not intercept external links on desktop", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("desktop");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const a = addAnchor("https://x.test/path");
      a.target = "_blank";
      const event = click(a);

      expect(event.defaultPrevented).toBe(false);
      expect(openExternal).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });

  it("leaves internal hash links untouched on Android", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("android");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const event = click(addAnchor("#x"));

      expect(event.defaultPrevented).toBe(false);
      expect(openExternal).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });

  it("fails closed on a non-allowlisted anchor scheme on Android", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("android");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const event = click(addAnchor("javascript:alert(1)"));

      expect(event.defaultPrevented).toBe(true);
      expect(openExternal).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });
});

describe("journal watcher feed reconciliation", () => {
  it("restarts a live Journals feed when the changed journal was already loaded in another pane", async () => {
    const name = "15th July, 2030";
    restorePaneLayout(
      { kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }] },
      new Map([
        ["main", { tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 }],
        ["pane-2", { tabs: [{ history: [{ kind: "page", name, pageKind: "journal" }], pos: 0, pinned: false }], activeIndex: 0 }],
      ]),
      "main"
    );
    setDoc({ byId: { loaded: node("loaded", name) }, pages: [page(name, "journal", ["loaded"])], feed: [], loaded: true });
    vi.spyOn(backend(), "getPage").mockResolvedValue({ name, kind: "journal", title: name, pre_block: null, blocks: [] });
    const now = new Date();
    const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate(),
    });

    await handleGraphChange({ name, kind: "journal", created: false, removed: false });
    await Promise.resolve();
    expect(feed).toHaveBeenCalledTimes(1);
    expect(feed).toHaveBeenCalledWith(3, null);
  });
});

describe("watcher page inventory invalidation", () => {
  it("bumps the rare page-inventory revision for an external create", async () => {
    const before = pageInventoryRev();
    await handleGraphChange({ name: "Created Elsewhere", kind: "page", created: true, removed: false });
    expect(pageInventoryRev()).toBeGreaterThan(before);
  });
});

describe("managed watcher reconciliation", () => {
  it("reloads a visible page and invalidates inventory after an admitted aggregate change", async () => {
    const name = "Managed External";
    resetPaneLayoutToSingle({
      tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
      activeIndex: 0,
    });
    setDoc({ byId: { old: node("old", name) }, pages: [page(name, "page", ["old"])], feed: [name], loaded: true });
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue({
      name,
      kind: "page",
      title: name,
      pre_block: null,
      blocks: [
        { id: "one", raw: "accepted first", collapsed: false, children: [], breadcrumb: [] },
        { id: "two", raw: "accepted external", collapsed: false, children: [], breadcrumb: [] },
      ],
    });
    const before = pageInventoryRev();

    await handleSparseV2Changed();

    expect(getPage).toHaveBeenCalledWith(name, "page");
    expect(pageInventoryRev()).toBeGreaterThan(before);
    expect(pageByName(name)?.roots).toEqual(["one", "two"]);
  });
});

// A change notification is not per-page divergence evidence. The managed runtime's
// `sparse-v2-changed` tick is a bare aggregate epoch (no page, no origin — it no
// longer fires for an admission that committed nothing, but which page and whose
// write it was are still unknown); the legacy watcher event names a page but
// still cannot tell our own write's echo from someone else's. Declaring a conflict
// on the notification alone blocks `doSave` for that page, so the banner's claim —
// "your unsaved changes weren't written" — comes true only BECAUSE of the banner.
// These cover both directions: a false conflict must not appear, and a REAL external
// change must still surface one.
describe("conflict requires per-page divergence, not just a notification", () => {
  const name = "Managed Racing Edit";

  function liveDirtyPage(rev: string | null) {
    resetPaneLayoutToSingle({
      tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
      activeIndex: 0,
    });
    setDoc({ byId: {}, pages: [], feed: [], loaded: true });
    // Seed the save baseline exactly as a real load does, then dirty the page:
    // the user typed and the debounced save has not landed yet.
    reloadPage({
      name,
      kind: "page",
      title: name,
      pre_block: null,
      rev,
      blocks: [{ id: "one", raw: "typed by the user", collapsed: false, children: [] }],
    });
    markDirty(name);
  }

  function storedPage(rev: string | null) {
    return {
      name,
      kind: "page" as const,
      title: name,
      pre_block: null,
      rev,
      blocks: [{ id: "one", raw: "stored content", collapsed: false, children: [] }],
    };
  }

  afterEach(() => {
    resetSaveState();
    clearConflict(name);
  });

  /** Stand in for the real Direct backend refusing a guarded save over a file
   *  that moved: it mints an observation epoch for the state it refused to
   *  overwrite and returns it as `conflict:<n>`. Returns the recorded calls so a
   *  test can inspect what a later force presented. */
  function refusingBackend(epoch: number) {
    const calls: Array<{ force: boolean; observation: number | null }> = [];
    vi.spyOn(backend(), "savePage").mockImplementation(
      (_dto, _baseRev, force, observation) => {
        calls.push({ force: !!force, observation: observation ?? null });
        return Promise.reject(new SaveConflictError(epoch));
      }
    );
    return calls;
  }

  /** The authority half of the real backend, which the flat mock above cannot
   *  express: a token exists for exactly one observation, every read or external
   *  observation revokes it, and a refusal mints the next one. `observe()` is
   *  what the native watcher does before its event reaches us. */
  function authoritativeBackend() {
    let live: number | null = null;
    let next = 11;
    const calls: Array<{ force: boolean; observation: number | null }> = [];
    const pending: Array<() => void> = [];
    let holding = false;
    const observe = () => {
      live = null;
    };
    vi.spyOn(backend(), "savePage").mockImplementation(
      (_dto, _baseRev, force, observation) => {
        calls.push({ force: !!force, observation: observation ?? null });
        if (force) {
          if (observation === null || observation === undefined || observation !== live) {
            // The exact family the backend uses when a force names an
            // observation that is not the live one.
            return Promise.reject({
              kind: "direct-save-failure",
              reasonCode: "conflict_authority.superseded",
            });
          }
          live = null;
          return Promise.resolve({ revision: "rev-forced" });
        }
        const refuse = () => {
          live = next;
          next += 1;
        };
        if (!holding) {
          refuse();
          return Promise.reject(new SaveConflictError(live));
        }
        return new Promise<{ revision: string }>((_resolve, reject) => {
          pending.push(() => {
            refuse();
            reject(new SaveConflictError(live));
          });
        });
      }
    );
    /** Leave the NEXT guarded refusal in flight instead of answering it. */
    const hold = () => {
      holding = true;
    };
    /** Block until the held refusal has actually reached the backend. */
    const heldReachedBackend = () =>
      vi.waitFor(() => expect(pending.length).toBeGreaterThan(0));
    /** Answer it, and stop holding the ones after it. */
    const settle = async () => {
      await heldReachedBackend();
      holding = false;
      pending.shift()!();
    };
    return { calls, observe, hold, heldReachedBackend, settle };
  }

  it("does not conflict a dirty page when the admitted epoch left it unchanged", async () => {
    liveDirtyPage("rev-1");
    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-1"));

    await handleSparseV2Changed();

    expect(isConflicted(name)).toBe(false);
    // The unsaved edit is still live AND still savable — not replaced by the
    // stored copy, and not frozen behind a conflict.
    expect(isDirty(name)).toBe(true);
    expect(pageByName(name)?.roots).toEqual(["one"]);
  });

  it("still conflicts a dirty page when the stored revision genuinely moved", async () => {
    liveDirtyPage("rev-1");
    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-2"));
    refusingBackend(7);

    await handleSparseV2Changed();

    expect(isConflicted(name)).toBe(true);
  });

  it("still conflicts a dirty page whose file was deleted under it", async () => {
    liveDirtyPage("rev-1");
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    refusingBackend(7);

    await handleSparseV2Changed();

    expect(isConflicted(name)).toBe(true);
  });

  it("leaves an in-flight save alone — its own base_rev guard is the authority", async () => {
    liveDirtyPage("rev-1");
    let release: (result: { revision: string }) => void = () => {};
    vi.spyOn(backend(), "savePage").mockReturnValue(
      new Promise<{ revision: string }>((resolve) => {
        release = resolve;
      })
    );
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-2"));
    const saving = flushPage(name); // in flight: not yet durable, baseline not yet advanced

    await handleSparseV2Changed();

    expect(isConflicted(name)).toBe(false);
    expect(getPage).not.toHaveBeenCalled();
    release({ revision: "rev-2" });
    await saving;
  });

  it("does not conflict a dirty page on a legacy watcher echo of our own write", async () => {
    liveDirtyPage("rev-1");
    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-1"));

    await handleGraphChange({ name, kind: "page", created: false, removed: false });

    expect(isConflicted(name)).toBe(false);
    expect(isDirty(name)).toBe(true);
  });

  it("still conflicts a dirty page on a legacy watcher event that really changed it", async () => {
    liveDirtyPage("rev-1");
    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-2"));
    refusingBackend(7);

    await handleGraphChange({ name, kind: "page", created: false, removed: false });

    expect(isConflicted(name)).toBe(true);
  });

  // Direct Files data-safety audit, finding 17. Nothing but an explicit user
  // click ever cleared a conflict, and a removal event raises one with no
  // divergence proof at all — so an editor writing by temp+rename, or a
  // mid-delivery Syncthing pass, could park a page behind a banner permanently.
  // The page then refuses every ordinary save, and both buttons are destructive:
  // "Use disk version" discards the edit, "Keep mine" overwrites a file it never
  // needed to. The divergence proof that gates RAISING a conflict decides this
  // too: when the disk provably holds the editor's own baseline again, the
  // banner's claim is false and it must go.
  it("clears a conflict once the file is back and provably matches the baseline", async () => {
    liveDirtyPage("rev-1");
    refusingBackend(7);
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    await handleGraphChange({ name, kind: "page", created: false, removed: true });
    expect(isConflicted(name)).toBe(true);

    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-1"));
    await handleGraphChange({ name, kind: "page", created: true, removed: false });

    expect(isConflicted(name)).toBe(false);
    // …and the edit that was frozen behind it is savable again, not discarded.
    expect(isDirty(name)).toBe(true);
    expect(pageByName(name)?.roots).toEqual(["one"]);
  });

  it("keeps the conflict when the file comes back with different content", async () => {
    liveDirtyPage("rev-1");
    refusingBackend(7);
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    await handleGraphChange({ name, kind: "page", created: false, removed: true });
    expect(isConflicted(name)).toBe(true);

    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-2"));
    await handleGraphChange({ name, kind: "page", created: true, removed: false });

    expect(isConflicted(name)).toBe(true);
  });

  // GH #254 increment 2, correction-delta re-verification, HIGH blocker.
  //
  // A banner is only worth raising if its "Keep mine" can work. That force must
  // present the observation epoch it was shown, and only a backend refusal mints
  // one — so a banner the frontend raised by itself presents null, `save_page`
  // refuses it, the retry is forbidden while the page is conflicted, and the one
  // remaining action discards the user's edit. Every path that notices an
  // external change under an unsaved edit therefore goes through the guarded
  // save, whatever noticed it.
  for (const [label, notice] of [
    ["a legacy watcher event", () => handleGraphChange({ name, kind: "page" as const, created: false, removed: false })],
    ["a sparse-v2 epoch", () => handleSparseV2Changed()],
    ["an external delete", () => handleGraphChange({ name, kind: "page" as const, created: false, removed: true })],
  ] as const) {
    it(`gives "Keep mine" real authority after ${label}`, async () => {
      liveDirtyPage("rev-1");
      vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-2"));
      const calls = refusingBackend(11);

      await notice();

      expect(isConflicted(name)).toBe(true);
      // The banner came from a guarded save the backend refused — not from the
      // frontend deciding on its own.
      expect(calls).toEqual([{ force: false, observation: null }]);

      await forceSave(name);

      expect(calls[1]).toEqual({ force: true, observation: 11 });
    });
  }

  // The same blocker one observation later (GH #254 increment 2, SECOND
  // correction-delta re-verification). A banner is up and holding epoch 11 when
  // a second external writer lands. The native watcher revokes that token before
  // its event even reaches us, and the read this handler does to prove divergence
  // revokes again — so by the time the user clicks "Keep mine", epoch 11 names
  // nothing. The page is conflicted and no longer dirty, which is exactly the
  // state an ordinary save declines, so nothing minted a replacement: the banner
  // was permanently disarmed and only the destructive button still worked.
  //
  // Scope note: the sparse-v2 case above models the composition, not a
  // production journey. The managed command maps divergence to a non-banner
  // `managed.conflict` and refuses force outright, which is a separate open
  // limitation rather than something these tests cover.
  it("re-arms the banner when a second external change lands under it", async () => {
    liveDirtyPage("rev-1");
    const { calls, observe } = authoritativeBackend();
    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-2"));

    observe(); // the watcher sees winner A
    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(isConflicted(name)).toBe(true);
    expect(isDirty(name)).toBe(false); // the refused save took it out of `dirty`

    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-3"));
    observe(); // winner B: the token behind the visible banner is now dead
    await handleGraphChange({ name, kind: "page", created: false, removed: false });

    expect(isConflicted(name)).toBe(true);
    // The re-observation reached the backend even though the page was clean and
    // already conflicted, and it was NOT a force.
    expect(calls).toEqual([
      { force: false, observation: null },
      { force: false, observation: null },
    ]);

    expect(await forceSave(name)).toBe(true);
    expect(calls[2]).toEqual({ force: true, observation: 12 });
  });

  // The other side of letting a re-observation write: when the disk has come
  // back to something the guard accepts, the edit lands and the banner must not
  // survive its own resolution.
  it("clears the banner when the re-observation's save succeeds", async () => {
    liveDirtyPage("rev-1");
    const { observe } = authoritativeBackend();
    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-2"));

    observe();
    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(isConflicted(name)).toBe(true);

    vi.spyOn(backend(), "savePage").mockResolvedValue({ revision: "rev-4" });
    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-3"));
    await handleGraphChange({ name, kind: "page", created: false, removed: false });

    expect(isConflicted(name)).toBe(false);
    expect(isDirty(name)).toBe(false);
  });

  // Third correction-delta re-verification, CRITICAL. The banner's epoch used to
  // be read when the queued force reached the backend, not when the user clicked.
  // A re-observation running ahead of it in the per-page queue replaced that
  // entry with an epoch minted for a winner the user never saw — and the force,
  // issued to discard A, was then authorised to discard B.
  it("does not let a queued \"Keep mine\" adopt an epoch minted after the click", async () => {
    liveDirtyPage("rev-1");
    const { calls, observe, hold, heldReachedBackend, settle } = authoritativeBackend();
    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-2"));

    observe();
    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(isConflicted(name)).toBe(true); // winner A refused: epoch 11

    // Winner B lands. Its re-observation is deliberately still in flight when the
    // user clicks the banner they can see, which is still A's.
    vi.spyOn(backend(), "getPage").mockResolvedValue(storedPage("rev-3"));
    observe();
    hold();
    const second = handleGraphChange({ name, kind: "page", created: false, removed: false });
    await heldReachedBackend(); // the re-observation is in flight...
    const clicked = forceSave(name); // ...and the user clicks the banner they see
    await settle(); // the re-observation now refuses B and mints epoch 12
    await second;
    await clicked;

    // The force presented 11 — the observation it was issued under — so the
    // backend refused it and B survives.
    const forced = calls.filter((call) => call.force);
    expect(forced.some((call) => call.observation === 12)).toBe(false);
    expect(forced[0]).toEqual({ force: true, observation: 11 });
    // …and the user is not stranded: the refusal re-observes, so the banner comes
    // back live and describing B. The re-observation is deliberately not awaited
    // by the force (the click is answered as soon as it is refused), so wait for
    // it the way the UI does — by watching the banner become actionable again.
    expect(isConflicted(name)).toBe(true);
    await vi.waitFor(() =>
      expect(calls.filter((call) => !call.force).length).toBeGreaterThanOrEqual(3)
    );
    expect(await forceSave(name)).toBe(true);
  });
});

// Concord P1 (freshness lane): `reloadDisposition` returns "skip" while a block
// on the changed page is being edited or a block move is in flight, and the
// watcher event was then simply dropped — the backend cache is fresh but the
// visible page stays stale indefinitely, because nothing replays the reload when
// the blocking condition clears. These prove the deferred replay: the skip is
// recorded and re-dispatched through `handleGraphChange` (so the disposition is
// re-evaluated at replay time) once the page becomes replaceable.
describe("deferred replay of externally changed pages skipped mid-edit", () => {
  const name = "Externally Edited";

  function loadedStalePage() {
    resetPaneLayoutToSingle({
      tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
      activeIndex: 0,
    });
    setDoc({
      byId: { b1: node("b1", name) },
      pages: [page(name, "page", ["b1"])],
      feed: [],
      loaded: true,
    });
  }

  function diskPage() {
    return {
      name,
      kind: "page" as const,
      title: name,
      pre_block: null,
      rev: "rev-2",
      blocks: [{ id: "b1", raw: "fresh from disk", collapsed: false, children: [] }],
    };
  }

  const visibleRaws = () => pageByName(name)?.roots.map((id) => doc.byId[id].raw);

  it("replays the reload when editing on the page ends", async () => {
    loadedStalePage();
    startEditing("b1");
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(diskPage());

    await handleGraphChange({ name, kind: "page", created: false, removed: false });

    // Mid-edit: the reload is deferred, not applied — the caret must not be yanked.
    expect(getPage).not.toHaveBeenCalled();
    expect(visibleRaws()).toEqual(["loaded elsewhere"]);

    endEdit("blur");

    await vi.waitFor(() => expect(visibleRaws()).toEqual(["fresh from disk"]));
  });

  it("replays the reload when a block move settles", async () => {
    loadedStalePage();
    setBlockMoving(true);
    vi.spyOn(backend(), "getPage").mockResolvedValue(diskPage());

    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(visibleRaws()).toEqual(["loaded elsewhere"]);

    setBlockMoving(false);

    await vi.waitFor(() => expect(visibleRaws()).toEqual(["fresh from disk"]));
  });

  // Concord P5: the one user-visible conflict policy switch (Obsidian 1.9.7).
  // The DEFAULT is unchanged — a clean loaded page reloads silently — and the
  // switch converts only that silent case into an asked one. Everything that
  // already asked or deferred is untouched, which is the property that makes it
  // safe to ship on by choice.
  describe("with always-ask on", () => {
    afterEach(() => {
      clearHeldExternalChanges();
      setConflictPolicyAlwaysAskForTest(false);
    });

    it("holds the change instead of applying it, and applies it on request", async () => {
      loadedStalePage();
      setConflictPolicyAlwaysAskForTest(true);
      const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(diskPage());

      await handleGraphChange({ name, kind: "page", created: false, removed: false });

      // Nothing was read and nothing was replaced: the user asked to be told.
      expect(getPage).not.toHaveBeenCalled();
      expect(visibleRaws()).toEqual(["loaded elsewhere"]);
      expect(heldExternalChangeFor(name)).toBeTruthy();
      expect(heldExternalChangeCount()).toBe(1);

      applyHeldExternalChange(name);

      await vi.waitFor(() => expect(visibleRaws()).toEqual(["fresh from disk"]));
      expect(heldExternalChangeFor(name)).toBeUndefined();
    });

    it("keeps mine without writing anything, and stops holding it", async () => {
      loadedStalePage();
      setConflictPolicyAlwaysAskForTest(true);
      vi.spyOn(backend(), "getPage").mockResolvedValue(diskPage());

      await handleGraphChange({ name, kind: "page", created: false, removed: false });
      dismissHeldExternalChange(name);

      expect(visibleRaws()).toEqual(["loaded elsewhere"]);
      expect(heldExternalChangeFor(name)).toBeUndefined();
    });

    it("does not take over the paths that already ask or defer", async () => {
      // Mid-edit: P1's deferral still wins, so the caret is safe and the change
      // is replayed rather than parked behind a bar the user must click.
      loadedStalePage();
      setConflictPolicyAlwaysAskForTest(true);
      startEditing("b1");
      vi.spyOn(backend(), "getPage").mockResolvedValue(diskPage());

      await handleGraphChange({ name, kind: "page", created: false, removed: false });
      expect(heldExternalChangeFor(name)).toBeUndefined();
      expect(visibleRaws()).toEqual(["loaded elsewhere"]);

      endEdit("blur");
      // Still not held: the replay re-enters with the page clean, and only then
      // does the policy see it.
      await vi.waitFor(() => expect(heldExternalChangeFor(name)).toBeTruthy());
      expect(visibleRaws()).toEqual(["loaded elsewhere"]);
    });

    it("is off by default, so a clean page still reloads silently", async () => {
      loadedStalePage();
      vi.spyOn(backend(), "getPage").mockResolvedValue(diskPage());

      await handleGraphChange({ name, kind: "page", created: false, removed: false });

      expect(heldExternalChangeFor(name)).toBeUndefined();
      await vi.waitFor(() => expect(visibleRaws()).toEqual(["fresh from disk"]));
    });
  });

  it("replays a reload declined by an editor lease once the lease is released", async () => {
    loadedStalePage();
    const release = takeEditorLease(name);
    vi.spyOn(backend(), "getPage").mockResolvedValue(diskPage());

    // The disposition is "reload" (a lease is not "skip"), so the handler fetches
    // the DTO — but `reloadPageIfStillSafe` refuses to replace the instance while
    // the lease holds uncommitted input. Same hole, different guard.
    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(visibleRaws()).toEqual(["loaded elsewhere"]);

    release();

    await vi.waitFor(() => expect(visibleRaws()).toEqual(["fresh from disk"]));
  });

  it("replays the winner self-write declined during an explicit Concord mutation as soon as ownership releases", async () => {
    loadedStalePage();
    const release = holdPageMutationUi([name]);
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(diskPage());

    // The winner write can arrive while Concord owns the exact page. It is
    // correctly deferred, but it must not remain parked until the user's next
    // keystroke turns that stale event into a false dirty-page conflict.
    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(getPage).toHaveBeenCalledTimes(1);
    expect(visibleRaws()).toEqual(["loaded elsewhere"]);

    release();

    await vi.waitFor(() => expect(getPage).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(visibleRaws()).toEqual(["fresh from disk"]));
  });
});
