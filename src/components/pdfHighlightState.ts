import { createSignal } from "solid-js";
import { backend } from "../backend";
import { errorFamily } from "../errorFamily";
import { flushPage, isConflicted, isDirty, reloadHlsIfLoaded, trackAssetWrite } from "../document";
import { bindingOwner, latestOwner, readOwned, serializeOwned, writeOwned, type Owner } from "../owned";
import { hlsPageName } from "../pdf";
import { isPdfOwnershipCurrent, trackPdfMutation, type PdfOwnership } from "../pdfOwnership";
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

/** Own one mounted PDF's highlights, serialized saves, conflict decisions and
 * crop cleanup. Construction performs no I/O. persist returns false on failure,
 * toasts and keeps edits marked. Conflict or failed crop cleanup blocks editing
 * and drain until a decision, discard, or cleanup retry succeeds. Discard clears
 * only local state. Cost is O(highlights + pending crops) plus backend writes. */
export function createPdfHighlightState(options: {
  filename: string;
  label: string;
  backendGeneration: number;
  owner: PdfOwnership;
  prepare: (items: Highlight[]) => Promise<Highlight[]>;
}) {
  const { filename, label, backendGeneration, owner, prepare } = options;
  const [highlights, setHighlights] = createSignal<Highlight[]>([]);
  const [unsaved, setUnsaved] = createSignal(false);
  const [conflict, setConflict] = createSignal(false);
  const [decisionBusy, setDecisionBusy] = createSignal(false);
  const [cleanupPending, setCleanupPending] = createSignal(false);
  let committed: Highlight[] = [];
  const pendingCrops = new Map<string, Crop>();
  const queue = {}, intents = {};
  const graphCurrent = bindingOwner(() => isPdfOwnershipCurrent(owner));

  const load = (items: Highlight[]) => {
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
      pushToast("Finish the highlight decision or area image cleanup before editing.", "error");
      return true;
    }
    if (!conflict()) return false;
    pushToast("Resolve the highlight conflict before editing more highlights.", "error");
    return true;
  };
  const drainBlocked = () => {
    if (decisionBusy()) return true;
    if (cleanupPending()) {
      pushToast(`Cannot close or switch graphs while ${filename} has an area image awaiting cleanup. Choose Retry cleanup.`, "error");
      return true;
    }
    if (!conflict()) return false;
    pushToast(`Cannot close or switch graphs while ${filename} has a highlight conflict. Choose Keep mine or Use disk version, or Discard my changes.`, "error");
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
          pushToast(`Couldn't retire the unused area image. Retry cleanup. (${String(error)})`, "error");
          return;
        }
      }
      if (!graphCurrent()) return;
      setCleanupPending(false);
      setUnsaved(false);
    } finally {
      setDecisionBusy(false);
    }
  };

  const persistOwned = async (landingOwner: Owner): Promise<boolean> => {
    if (conflict() || cleanupPending()) return false;
    const hlsName = hlsPageName(filename);
    if (isDirty(hlsName) || isConflicted(hlsName)) {
      if (!(await flushPage(hlsName))) {
        if (landingOwner()) pushToast("Couldn't save notes — highlight not written. Resolve the conflict and retry.", "error");
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
      committed = result.value;
      for (const [id, crop] of pendingCrops) {
        if (result.value.some((h) => h.id === id && h.image === crop.stamp)) pendingCrops.delete(id);
      }
      let cleanupFailed = false;
      for (const [id, crop] of unusedPdfCrops(pendingCrops, result.value, highlights())) {
        try { await retireCrop(id, crop); }
        catch (error) {
          cleanupFailed = true;
          if (landingOwner()) pushToast(`Highlight saved, but an unused area image couldn't move to trash. (${String(error)})`, "error");
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
          pushToast("Highlight conflict — choose Keep mine or Use disk version.", "error");
        } else {
          pushToast(`Couldn't save highlight — it remains unsaved in the PDF. (${String(e)})`, "error");
        }
      }
      return false;
    }
    try {
      const reloaded = await readOwned(graphCurrent, reloadHlsIfLoaded(hlsName));
      if (reloaded.kind === "stale") return false;
    } catch (error) {
      if (landingOwner()) pushToast(`Highlight saved, but notes couldn't reload. (${String(error)})`, "error");
    }
    return !cleanupPending();
  };

  const newIntent = () => latestOwner(intents, "highlights", graphCurrent);
  const persist = async (landingOwner = newIntent()): Promise<boolean> => {
    try {
      const result = await trackPdfMutation(owner, () =>
        serializeOwned(queue, graphCurrent, () => persistOwned(landingOwner))
      );
      return result.kind === "current" && result.value;
    } catch {
      return false;
    }
  };
  const persistInsideMutation = async (landingOwner: Owner): Promise<boolean> => {
    const result = await serializeOwned(queue, graphCurrent, () => persistOwned(landingOwner));
    return result.kind === "current" && result.value;
  };

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
      pushToast("Using disk highlights; local changes were discarded.", "success");
    } catch (e) {
      pushToast(`Couldn't use disk highlights or retire their area image. (${String(e)})`, "error");
    } finally { setDecisionBusy(false); }
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
      pushToast(`Couldn't reload disk highlights for Keep mine. (${String(e)})`, "error");
    } finally { setDecisionBusy(false); }
  };

  const discardMine = async () => {
    if (!conflict() || decisionBusy()) return;
    setDecisionBusy(true);
    try {
      const answer = await readOwned(graphCurrent, backend().confirm(
        "Discard your local PDF highlight changes? The sidecar will be left untouched. Any unreferenced local crop may remain on disk.",
        "Discard my changes"
      ));
      if (answer.kind === "stale" || !answer.value) return;
      pendingCrops.clear();
      load([]);
      pushToast("Local highlight changes discarded; disk sidecar was left untouched.", "success");
    } catch (error) {
      pushToast(`Couldn't confirm highlight discard. (${String(error)})`, "error");
    } finally { setDecisionBusy(false); }
  };

  return { highlights, unsaved, conflict, decisionBusy, cleanupPending, graphCurrent, load, edit, addCrop,
    editBlocked, drainBlocked, persist, persistInsideMutation, useDiskVersion, keepMine, discardMine, retryCleanup, newIntent };
}
