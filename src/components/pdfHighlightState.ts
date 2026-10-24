import { msg, t } from "../i18n";
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { errorFamily } from "../errorFamily";
import { flushPage, isConflicted, isDirty, reloadHlsIfLoaded, trackAssetWrite } from "../document";
import { bindingOwner, latestOwner, readOwned, serializeOwned, writeOwned, type Owner } from "../owned";
import { hlsPageName } from "../pdf";
import { isPdfOwnershipCurrent, pdfOwnershipKey, registerPdfParticipant, trackPdfMutation, type PdfOwnership } from "../pdfOwnership";
import { pushToast } from "../toasts";
import type { Highlight } from "../types";

type Crop = { page: number; stamp: number };

/** Rebase local field edits and deletions from the committed set onto a fresh
 * disk set. Disk-only additions survive; a changed local highlight survives a
 * disk deletion. Cost is linear in the number of highlights. */
export function rebasePdfHighlights(committed: Highlight[], visible: Highlight[], disk: Highlight[]): Highlight[] {
  const old = new Map(committed.map((h) => [h.id, h]));
  const local = new Map(visible.map((h) => [h.id, h]));
  const diskIds = new Set(disk.map((h) => h.id));
  const next = disk.filter((h) => !old.has(h.id) || local.has(h.id)).map((h) => {
    const before = old.get(h.id), edit = local.get(h.id);
    if (!before || !edit) return edit ?? h;
    const geometryChanged = edit.page !== before.page || JSON.stringify(edit.position) !== JSON.stringify(before.position);
    return {
      ...h,
      page: geometryChanged ? edit.page : h.page,
      position: geometryChanged ? edit.position : h.position,
      color: edit.color !== before.color ? edit.color : h.color,
      text: edit.text !== before.text ? edit.text : h.text,
      image: edit.image !== before.image ? edit.image : h.image,
    };
  });
  for (const h of visible) {
    if (!diskIds.has(h.id) && (!old.has(h.id) || JSON.stringify(h) !== JSON.stringify(old.get(h.id)))) next.push(h);
  }
  return next;
}

/** Return pending area crops absent from both the committed response and the
 * current optimistic set. A later queued edit can still use a crop, so its
 * presence in either set prevents retirement. Cost is O(crops × highlights). */
export function unusedPdfCrops(crops: ReadonlyMap<string, Crop>, committed: Highlight[], visible: Highlight[]): [string, Crop][] {
  return [...crops].filter(([id, crop]) =>
    !committed.some((h) => h.id === id && h.image === crop.stamp) &&
    !visible.some((h) => h.id === id && h.image === crop.stamp));
}

type PrepareHighlights = (items: Highlight[]) => Promise<Highlight[]>;

const needsPageGeometry = (h: Highlight) =>
  [h.position.bounding, ...h.position.rects].some((r) => r.source_width == null || r.source_height == null);

/** With no live document, only highlights that already carry their source page
 * size can be written; the rest wait until a view of the PDF is open again. */
const prepareWithoutDocument: PrepareHighlights = async (items) => {
  if (items.some(needsPageGeometry)) throw new Error("open the PDF again to save its highlights: some need its page sizes");
  return items;
};

export interface PdfHighlightStateOptions {
  filename: string;
  label: string;
  backendGeneration: number;
  owner: PdfOwnership;
  /** Fallback when no view has attached one (tests, or every view gone). */
  prepare?: PrepareHighlights;
  /** Called whenever the state may have become idle (see idle()). */
  onSettle?: () => void;
}

/** Own one PDF's highlights, serialized saves, conflict decisions and crop
 * cleanup, for every view of that PDF (see acquirePdfHighlightState).
 * Construction performs no I/O. persist returns false on failure, toasts and
 * keeps edits marked. Conflict or failed crop cleanup blocks editing and drain
 * until a decision, discard, or cleanup retry succeeds. Discard clears only
 * local state. Cost is O(highlights + pending crops) plus backend writes. */
export function createPdfHighlightState(options: PdfHighlightStateOptions) {
  const { filename, label, backendGeneration, owner } = options;
  // Each loaded view supplies page geometry from its own pdf.js document; any
  // live one gives the same source-space dimensions. The latest attached is
  // tried first; a view that fails (its document closed mid-save) falls back
  // to an older one, then to the document-free path.
  const preparers: PrepareHighlights[] = [];
  const prepare: PrepareHighlights = async (items) => {
    let failure: { error: unknown } | null = null;
    for (const fn of [...preparers].reverse()) {
      try { return await fn(items); } catch (error) { failure ??= { error }; }
    }
    try { return await (options.prepare ?? prepareWithoutDocument)(items); }
    catch (error) { throw failure ? failure.error : error; }
  };
  const attachPrepare = (fn: PrepareHighlights) => {
    preparers.push(fn);
    return () => {
      const at = preparers.lastIndexOf(fn);
      if (at >= 0) preparers.splice(at, 1);
    };
  };
  let loaded = false;
  const [highlights, setHighlights] = createSignal<Highlight[]>([]);
  const [unsaved, setUnsaved] = createSignal(false);
  const [conflict, setConflict] = createSignal(false);
  const [decisionBusy, setDecisionBusy] = createSignal(false);
  const [cleanupPending, setCleanupPending] = createSignal(false);
  let committed: Highlight[] = [];
  let running = 0;
  const pendingCrops = new Map<string, Crop>();
  const queue = {}, intents = {};
  const graphCurrent = bindingOwner(() => isPdfOwnershipCurrent(owner));

  const load = (items: Highlight[]) => {
    loaded = true;
    committed = items;
    setHighlights(items);
    setUnsaved(false);
    setConflict(false);
  };
  const edit = (items: Highlight[]) => {
    setHighlights(items);
    setUnsaved(true);
  };
  const addCrop = (id: string, crop: Crop) => pendingCrops.set(id, crop);
  const editBlocked = () => {
    if (decisionBusy() || cleanupPending()) {
      pushToast(msg("pdf-finish-the-highlight-decision-or-area-image-cleanup-before-editing"), "error");
      return true;
    }
    if (!conflict()) return false;
    pushToast(msg("pdf-resolve-the-highlight-conflict-before-editing-more-highlights"), "error");
    return true;
  };
  const drainBlocked = () => {
    if (decisionBusy()) return true;
    if (cleanupPending()) {
      pushToast(msg("pdf-cannot-close-or-switch-graphs-while-has-an-area-image-awaiting-cleanup-choose-retry-cleanup", { name: String(filename) }), "error");
      return true;
    }
    if (!conflict()) return false;
    pushToast(msg("pdf-cannot-close-or-switch-graphs-while-has-a-highlight-conflict-choose-keep-mine-or-use-disk-version-or-discard-my-changes", { name: String(filename) }), "error");
    return true;
  };
  const retireCrop = async (id: string, crop: Crop): Promise<boolean> => {
    const retired = await writeOwned(graphCurrent, trackAssetWrite(backend().rollbackPdfAreaImage(
      filename, crop.page, id, crop.stamp, backendGeneration
    )));
    if (retired.kind === "stale") return false;
    pendingCrops.delete(id);
    return true;
  };

  const retryCleanup = async (): Promise<void> => {
    if (decisionBusy() || !cleanupPending()) return;
    setDecisionBusy(true);
    try {
      for (const [id, crop] of unusedPdfCrops(pendingCrops, committed, highlights())) {
        try { if (!(await retireCrop(id, crop))) return; }
        catch (error) {
          pushToast(msg("pdf-couldn-t-retire-the-unused-area-image-retry-cleanup", { detail: String(String(error)) }), "error");
          return;
        }
      }
      if (!graphCurrent()) return;
      setCleanupPending(false);
      setUnsaved(false);
    } finally {
      setDecisionBusy(false);
      settle();
    }
  };

  const persistOwned = async (landingOwner: Owner): Promise<boolean> => {
    if (conflict() || cleanupPending()) return false;
    const hlsName = hlsPageName(filename);
    if (isDirty(hlsName) || isConflicted(hlsName)) {
      if (!(await flushPage(hlsName))) {
        if (landingOwner()) pushToast(msg("pdf-couldn-t-save-notes-highlight-not-written-resolve-the-conflict-and-retry"), "error");
        return false;
      }
      if (!graphCurrent()) return false;
    }
    try {
      const persisted = await prepare(highlights());
      if (!graphCurrent()) return false;
      const result = await writeOwned(graphCurrent, trackAssetWrite(
        backend().writeHighlights(filename, label, persisted, committed, "replace-page", backendGeneration)
      ));
      if (result.kind === "stale") return false;
      const base = committed;
      committed = result.value;
      // A newer edit is queued behind this write: keep it, but on the merged
      // result, so a highlight the merge added (another device, OG) is not
      // sent back as a deletion and its merged fields are not overwritten.
      if (!landingOwner()) setHighlights(rebasePdfHighlights(base, highlights(), result.value));
      for (const [id, crop] of pendingCrops) {
        if (result.value.some((h) => h.id === id && h.image === crop.stamp)) pendingCrops.delete(id);
      }
      let cleanupFailed = false;
      for (const [id, crop] of unusedPdfCrops(pendingCrops, result.value, highlights())) {
        try { await retireCrop(id, crop); }
        catch (error) {
          cleanupFailed = true;
          if (landingOwner()) pushToast(msg("pdf-highlight-saved-but-an-unused-area-image-couldn-t-move-to-trash", { detail: String(String(error)) }), "error");
        }
      }
      if (landingOwner()) {
        load(result.value);
        if (cleanupFailed) { setCleanupPending(true); setUnsaved(true); }
      }
    } catch (e) {
      if (landingOwner()) {
        setUnsaved(true);
        if (errorFamily(e) === "conflict") {
          setConflict(true);
          pushToast(msg("pdf-highlight-conflict-choose-keep-mine-or-use-disk-version"), "error");
        } else {
          pushToast(msg("pdf-couldn-t-save-highlight-it-remains-unsaved-in-the-pdf", { detail: String(String(e)) }), "error");
        }
      }
      return false;
    }
    try {
      const reloaded = await readOwned(graphCurrent, reloadHlsIfLoaded(hlsName));
      if (reloaded.kind === "stale") return false;
    } catch (error) {
      if (landingOwner()) pushToast(msg("pdf-highlight-saved-but-notes-couldn-t-reload", { detail: String(String(error)) }), "error");
    }
    return !cleanupPending();
  };

  const settle = () => options.onSettle?.();
  const counted = async <T>(work: () => Promise<T>): Promise<T> => {
    running++;
    try { return await work(); } finally { running--; settle(); }
  };
  const newIntent = () => latestOwner(intents, "highlights", graphCurrent);
  const persist = (landingOwner = newIntent()): Promise<boolean> => counted(async () => {
    try {
      const result = await trackPdfMutation(owner, () =>
        serializeOwned(queue, graphCurrent, () => persistOwned(landingOwner))
      );
      return result.kind === "current" && result.value;
    } catch {
      return false;
    }
  });
  const persistInsideMutation = (landingOwner: Owner): Promise<boolean> => counted(async () => {
    const result = await serializeOwned(queue, graphCurrent, () => persistOwned(landingOwner));
    return result.kind === "current" && result.value;
  });

  const useDiskVersion = async () => {
    if (!conflict() || decisionBusy()) return;
    setDecisionBusy(true);
    try {
      const result = await readOwned(graphCurrent, backend().readHighlights(filename));
      if (result.kind === "stale") return;
      for (const [id, crop] of pendingCrops) {
        if (result.value.some((h) => h.id === id && h.image === crop.stamp)) { pendingCrops.delete(id); continue; }
        if (!(await retireCrop(id, crop))) return;
      }
      if (!graphCurrent()) return;
      const latest = await readOwned(graphCurrent, backend().readHighlights(filename));
      if (latest.kind === "stale") return;
      pendingCrops.clear();
      load(latest.value);
      pushToast(msg("pdf-using-disk-highlights-local-changes-were-discarded"), "success");
    } catch (e) {
      pushToast(msg("pdf-couldn-t-use-disk-highlights-or-retire-their-area-image", { detail: String(String(e)) }), "error");
    } finally { setDecisionBusy(false); settle(); }
  };
  const keepMine = async () => {
    if (!conflict() || decisionBusy()) return;
    setDecisionBusy(true);
    try {
      const result = await readOwned(graphCurrent, backend().readHighlights(filename));
      if (result.kind === "stale") return;
      const next = rebasePdfHighlights(committed, highlights(), result.value);
      committed = result.value;
      edit(next);
      setConflict(false);
      await persist();
    } catch (e) {
      pushToast(msg("pdf-couldn-t-reload-disk-highlights-for-keep-mine", { detail: String(String(e)) }), "error");
    } finally { setDecisionBusy(false); settle(); }
  };

  const discardMine = async () => {
    if (!conflict() || decisionBusy()) return;
    setDecisionBusy(true);
    try {
      const answer = await readOwned(graphCurrent, backend().confirm(
        t("pdf-discard-your-local-pdf-highlight-changes-the-sidecar-will-be-left-untouched-any-unreferenced-local-crop-may-remain-on-disk"),
        { title: t("confirm-discard-my-changes"), okLabel: t("confirm-discard-my-changes"), cancelLabel: t("confirm-cancel") },
      ));
      if (answer.kind === "stale" || !answer.value) return;
      pendingCrops.clear();
      load([]);
      pushToast(msg("pdf-local-highlight-changes-discarded-disk-sidecar-was-left-untouched"), "success");
    } catch (error) {
      pushToast(msg("pdf-couldn-t-confirm-highlight-discard", { detail: String(String(error)) }), "error");
    } finally { setDecisionBusy(false); settle(); }
  };

  /** The first successful disk read seeds the state; a view joining later
   * keeps what its siblings already hold (it may be newer, or unsaved). A view
   * whose read failed loads nothing, so a sibling's read can still seed it;
   * edits made meanwhile are kept on top of what that read finds. */
  const loadInitial = (items: Highlight[]): boolean => {
    if (loaded) return false;
    if (!unsaved()) {
      load(items);
      return true;
    }
    loaded = true;
    setHighlights(rebasePdfHighlights(committed, highlights(), items));
    committed = items;
    return true;
  };
  /** Nothing unsaved, undecided, awaiting cleanup or being written: dropping
   * the state loses nothing. */
  const idle = () => running === 0 && !decisionBusy() && !unsaved() && !conflict() && !cleanupPending();
  // Every view registers a drain participant; one flush serves them all, so a
  // drain neither writes twice nor toasts a blocked decision once per view.
  let flushing: Promise<boolean> | null = null;
  const flush = (): Promise<boolean> => {
    if (flushing) return flushing;
    // The body starts a microtask later, after the slot is set, so a flush it
    // triggers joins this one; the slot clears however the body ends.
    const run = Promise.resolve().then(async () => !drainBlocked() && (!unsaved() || await persist()));
    flushing = run;
    const clear = () => { if (flushing === run) flushing = null; };
    run.then(clear, clear);
    return run;
  };

  return { highlights, unsaved, conflict, decisionBusy, cleanupPending, graphCurrent, load, loadInitial, edit, addCrop,
    editBlocked, drainBlocked, persist, persistInsideMutation, useDiskVersion, keepMine, discardMine, retryCleanup, newIntent,
    attachPrepare, flush, idle };
}

export type PdfHighlightState = ReturnType<typeof createPdfHighlightState>;

interface PdfHighlightSession {
  state: PdfHighlightState;
  views: number;
  /** Drop the session once it has no view and is idle (or at once when its
   * graph binding retires). */
  dispose(retired?: boolean): void;
}

const sessions = new Map<string, PdfHighlightSession>();

/** Share one highlight state among every mounted view of a PDF in this graph
 * binding (PDF tabs and splits design, 5.1): a highlight made,
 * recolored or deleted in one view shows in its siblings at once, and their
 * saves are linearized over one base instead of racing stale snapshots. Views
 * keep their own page, zoom and scroll. Release the lease on unmount. The state
 * outlives its last view until it is idle: a save still running, a failed or
 * conflicted save, or a pending crop cleanup stays with the graph's drain and
 * reappears when the PDF is opened again; retiring the binding drops it. Cost
 * O(1) per acquire/release. */
export function acquirePdfHighlightState(options: Omit<PdfHighlightStateOptions, "prepare">): {
  state: PdfHighlightState;
  release: () => void;
} {
  const key = `${pdfOwnershipKey(options.owner)}\0${options.backendGeneration}\0${options.filename}`;
  let session = sessions.get(key);
  if (!session) {
    let unregister = () => {};
    const created: PdfHighlightSession = {
      state: createPdfHighlightState({ ...options, onSettle: () => created.dispose() }),
      views: 0,
      dispose: (retired = false) => {
        if (sessions.get(key) !== created || (!retired && (created.views > 0 || !created.state.idle()))) return;
        sessions.delete(key);
        unregister();
      },
    };
    sessions.set(key, created);
    // The session, not a view, answers the graph's drain, so work left by a
    // closed view still blocks a graph switch or close until it is resolved.
    unregister = registerPdfParticipant(options.owner, { flush: created.state.flush, cancel: () => created.dispose(true) });
    session = created;
  }
  const held = session;
  held.views++;
  let released = false;
  return {
    state: held.state,
    release: () => {
      if (released) return;
      released = true;
      held.views--;
      held.dispose();
    },
  };
}

/** Live shared states (test and probe use). */
export function pdfHighlightSessionCount(): number {
  return sessions.size;
}
