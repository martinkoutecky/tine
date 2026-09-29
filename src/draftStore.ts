// Crash-surviving unsaved drafts (og ADR 0061, family 9). While a page holds
// edits that cannot currently be saved (a conflict, or a save that failed), a
// copy of its draft is kept in the app-data draft store, refreshed at most every
// REFRESH_MS while it changes, and retired the moment the page is safe again.
// An ordinary save never writes here. Records from an earlier session (a crash,
// a kill, a power cut) are offered on the next open of that graph for review;
// only the user dismisses them.
import { createEffect, createRoot } from "solid-js";
import { backend } from "./backend";
import { captureBinding, graphScopedSignal, refuseStaleWrite, stillBound, type Binding } from "./binding";
import { installDraftKeeper, unsavedDrafts } from "./document";
import { graphEpoch, graphMeta } from "./graphSession";
import { graphOwner, readOwned, writeOwned } from "./owned";
import { pushToast } from "./toasts";
import { openUnsavedRecovery } from "./unsavedRecovery";
import type { DraftRecord } from "./types";

export const REFRESH_MS = 500;
const session = typeof crypto !== "undefined" && "randomUUID" in crypto
  ? crypto.randomUUID()
  : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
const idFor = (name: string) => `${session}:${name}`;

type Kept = { binding: Binding; written: string | null };
const atRisk = new Map<string, Kept>();
let timer: ReturnType<typeof setTimeout> | null = null;
let refusedOnce = false;

// An earlier session's drafts belong to the graph they were read for.
const [earlier, setEarlier] = graphScopedSignal<DraftRecord[]>();
/** Drafts an earlier session kept for the open graph, newest first. */
export const earlierDrafts = (): DraftRecord[] => earlier() ?? [];

function schedule() {
  if (timer || atRisk.size === 0) return;
  timer = setTimeout(() => { timer = null; void writeAtRisk(); }, REFRESH_MS);
}

/** Write every at-risk page whose draft changed since its last write. */
export async function writeAtRisk(): Promise<void> {
  const current = new Map(unsavedDrafts().map((d) => [d.name, d]));
  for (const [name, kept] of atRisk) {
    if (!stillBound(kept.binding)) { atRisk.delete(name); continue; }
    const draft = current.get(name);
    if (!draft?.page) continue;
    // A disk-changed conflict is a live-conflict capsule (og 8e): it also keeps
    // the revisions the in-page resolver needs after a restart. One record id
    // per page either way, so a kind change replaces rather than duplicates.
    const live = draft.state === "Conflict" && draft.live;
    const text = JSON.stringify([draft.page, live, draft.baseRev, draft.observedRev]);
    if (text === kept.written) continue;
    const record: DraftRecord = {
      id: idFor(name), kind: live ? "live-conflict" : "unsaved", session, page_name: name, path: draft.path,
      reason: draft.state === "Conflict" ? "conflict" : "save-failed",
      saved_at: Date.now(), page: draft.page,
      ...(live ? { base_rev: draft.baseRev, observed_rev: draft.observedRev } : {}),
    };
    try {
      const written = await writeOwned(graphOwner(), backend().storeDraft?.(record) ?? Promise.resolve());
      if (written.kind === "current") kept.written = text;
    } catch (error) {
      // Refused past the store's bound, or a disk error: the draft stays in this
      // window (recovery panel); say once that it will not survive a crash.
      if (!refusedOnce) pushToast(`Couldn't keep a crash-safe copy of “${name}” — ${String(error)}. It is still open in this window.`, "warn");
      refusedOnce = true;
    }
  }
  schedule();
}

async function retire(name: string, kept: Kept) {
  if (kept.written === null || !stillBound(kept.binding)) return;
  try {
    await writeOwned(graphOwner(), backend().retireDraft?.(idFor(name)) ?? Promise.resolve());
  } catch (error) {
    pushToast(`Couldn't remove the crash-safe copy of “${name}” (${String(error)}). The page is saved; the copy may be offered again later.`, "warn");
  }
}

function keep(name: string, risky: boolean) {
  if (risky) {
    // An entry left from another graph binding (a switch while it was at risk)
    // is not this page: start a fresh one, or this draft would never be kept.
    const kept = atRisk.get(name);
    if (!kept || !stillBound(kept.binding)) atRisk.set(name, { binding: captureBinding(), written: null });
    schedule();
    return;
  }
  const kept = atRisk.get(name);
  if (!kept) return;
  atRisk.delete(name);
  void retire(name, kept);
}

/** Retire a record an earlier session kept: the user's explicit choice. */
export async function dismissEarlierDraft(id: string): Promise<void> {
  // A panel that outlived its graph must not retire a record in the next one.
  if (earlier() === null) return refuseStaleWrite("Dismissing the kept draft");
  try {
    const result = await writeOwned(graphOwner(), backend().retireDraft?.(id) ?? Promise.resolve());
    if (result.kind === "current") setEarlier(earlierDrafts().filter((r) => r.id !== id));
  } catch (error) {
    pushToast(`Couldn't dismiss the kept draft (${String(error)}). It is still available.`, "error");
  }
}

async function offerEarlier() {
  let result;
  try {
    result = await readOwned(graphOwner(), backend().loadDrafts?.() ?? Promise.resolve([]));
  } catch (error) {
    // The backend sets an unreadable store aside, so this is a disk error or a
    // missing app-data dir: opening the graph goes on without earlier drafts.
    pushToast(`Couldn't read drafts kept from an earlier session (${String(error)}).`, "warn");
    return;
  }
  if (result.kind === "stale") return;
  const mine = result.value.filter((r) => r.session !== session)
    .sort((a, b) => b.saved_at - a.saved_at);
  setEarlier(mine);
  if (mine.length === 0) return;
  const pages = mine.length === 1 ? `“${mine[0].page_name}”` : `${mine.length} pages`;
  pushToast(`Tine kept unsaved drafts of ${pages} from an earlier session.`, "warn", {
    sticky: true, action: { label: "Review", run: openUnsavedRecovery },
  });
}

let installed = false;
/** Start keeping drafts and offer an earlier session's drafts on each graph open. */
export function installDraftStore(): void {
  if (installed) return;
  installed = true;
  installDraftKeeper(keep);
  createRoot(() => createEffect(() => {
    graphEpoch();
    if (!graphMeta()) return;
    void offerEarlier();
  }));
}
