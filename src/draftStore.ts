// Crash-surviving unsaved drafts (og ADR 0061, family 9). While a page holds
// edits that cannot currently be saved (a conflict, or a save that failed), a
// copy of its draft is kept in the app-data draft store, refreshed at most every
// REFRESH_MS while it changes, and retired the moment the page is safe again.
// An ordinary save never writes here. Records from an earlier session (a crash,
// a kill, a power cut) are offered on the next open of that graph for review;
// only the user dismisses them.
import { createEffect, createRoot, createSignal } from "solid-js";
import { backend } from "./backend";
import { captureBinding, clearOnBindingInvalidated, graphScopedSignal, refuseStaleWrite, bindingCurrent, type Binding } from "./binding";
import { installDraftKeeper, unsavedDrafts } from "./document";
import { graphEpoch, graphMeta } from "./graphSession";
import { bindingOwner, graphOwner, ownedWhen, readOwned, serializeDurable, writeOwned } from "./owned";
import { pushToast } from "./toasts";
import { openUnsavedRecovery } from "./unsavedRecovery";
import type { DraftRecord } from "./types";

export const REFRESH_MS = 500;
const newSessionId = () => typeof crypto !== "undefined" && "randomUUID" in crypto
  ? crypto.randomUUID()
  : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
const session = newSessionId();
const idFor = (name: string) => `${session}:${name}`;

// `supersedes`: the name this page had before its file was renamed while at
// risk; that record retires only once this one is written.
type Kept = { binding: Binding; written: string | null; risky: boolean; supersedes?: string };
const atRisk = new Map<string, Kept>();
let timer: ReturnType<typeof setTimeout> | null = null;

// Pages whose crash-safe write was refused: said once per page until a write
// for it succeeds again (a refusal of another page is said too).
const refused = new Set<string>();
// §8.5 (G37): an unreadable store is set aside with its bytes kept and the
// store starts empty. Every draft-store call reports that path; say it once per
// path and graph binding, never silently treat the store as empty.
const reportedAside = new Set<string>();

// I-21: page queues belong to one binding. A switch preserves already-started
// capsules for recovery but cannot start their retirement in the next graph.
clearOnBindingInvalidated(() => {
  if (timer) clearTimeout(timer);
  timer = null;
  atRisk.clear();
  refused.clear();
  reportedAside.clear();
});

function noteSetAside(aside: string | null | undefined): void {
  if (!aside || reportedAside.has(aside)) return;
  reportedAside.add(aside);
  pushToast(`Tine's store of crash-safe copies for this graph could not be read. It was moved aside to ${aside} (nothing was deleted) and a new one was started; copies it held are not offered.`, "error", { sticky: true });
}
const stored = (write: Promise<string | null> | undefined) => (write ?? Promise.resolve(null)).then(noteSetAside);

// An earlier session's drafts belong to the graph they were read for.
const [earlier, setEarlier] = graphScopedSignal<DraftRecord[]>();
/** Drafts an earlier session kept for the open graph, newest first. */
export const earlierDrafts = (): DraftRecord[] => earlier() ?? [];

function schedule() {
  if (timer || ![...atRisk.values()].some((kept) => kept.risky)) return;
  timer = setTimeout(() => { timer = null; void writeAtRisk(); }, REFRESH_MS);
}

/** Write changed at-risk drafts, serialized with retirement for each page.
 * O(at-risk pages); a graph switch skips queued work, while started writes and
 * their failures are observed. Saving queues retirement after the first write. */
export async function writeAtRisk(): Promise<void> {
  const current = new Map(unsavedDrafts().map((d) => [d.name, d]));
  for (const [name, kept] of atRisk) {
    if (!bindingCurrent(kept.binding)) { atRisk.delete(name); continue; }
    if (!kept.risky) continue;
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
      const owner = ownedWhen(() => bindingCurrent(kept.binding));
      await serializeDurable(kept, owner, async () => {
        if (!kept.risky || text === kept.written) return;
        const written = await writeOwned(owner, stored(backend().storeDraft?.(record)));
        if (written.kind === "stale" || !owner()) return;
        // Record completion before the queued retirement examines it. Already
        // started writes finish even when the page becomes safe meanwhile.
        kept.written = text;
        refused.delete(name);
      });
      const old = kept.supersedes !== undefined ? atRisk.get(kept.supersedes) : undefined;
      if (kept.written === text && kept.supersedes !== undefined) {
        const from = kept.supersedes;
        kept.supersedes = undefined;
        if (old && atRisk.get(from) === old) { old.risky = false; void retire(from, old); }
      }
    } catch (error) {
      // Refused past the store's bound, or a disk error: the draft stays in this
      // window (recovery panel); say once that it will not survive a crash.
      // The page stays at risk and the write is retried on the next refresh.
      if (!refused.has(name)) pushToast(`Couldn't keep a crash-safe copy of “${name}” — ${String(error)}. It is still open in this window.`, "error", { sticky: true });
      refused.add(name);
    }
  }
  schedule();
}

async function retire(name: string, kept: Kept) {
  try {
    const owner = ownedWhen(() => bindingCurrent(kept.binding));
    await serializeDurable(kept, owner, async () => {
      if (kept.risky) return;
      if (kept.written !== null) {
        const retired = await writeOwned(owner, stored(backend().retireDraft?.(idFor(name))));
        if (retired.kind === "stale" || !owner()) return;
        kept.written = null;
      }
      if (!kept.risky && atRisk.get(name) === kept) atRisk.delete(name);
    });
  } catch (error) {
    pushToast(`Couldn't remove the crash-safe copy of “${name}” (${String(error)}). The page is saved; the copy may be offered again later.`, "error");
  }
}

function keep(name: string, risky: boolean, renamedFrom?: string) {
  if (risky) {
    // An entry left from another graph binding (a switch while it was at risk)
    // is not this page: start a fresh one, or this draft would never be kept.
    let kept = atRisk.get(name);
    if (!kept || !bindingCurrent(kept.binding)) atRisk.set(name, kept = { binding: captureBinding(), written: null, risky: true });
    else kept.risky = true;
    // A rename while at risk: the old name's record stays until this one is
    // durable, so the typed text always has a draft (storage.qnt guarantee B).
    if (renamedFrom !== undefined && atRisk.has(renamedFrom)) kept.supersedes = renamedFrom;
    schedule();
    return;
  }
  const kept = atRisk.get(name);
  if (!kept) return;
  kept.risky = false;
  void retire(name, kept);
}

/** Drafts of the previous graph's unsaved pages taken at a graph switch and
 *  not (yet) durable in its draft store: this window holds them, and the
 *  recovery panel offers Copy and Dismiss. Window-lifetime, not graph-scoped:
 *  they belong to the graph that was left. */
export type HeldDraft = { root: string; record: DraftRecord };
const [held, setHeld] = createSignal<HeldDraft[]>([]);
export const switchHeldDrafts = (): HeldDraft[] => held();
/** The user's explicit release of a held switch draft. O(held). */
export function dismissHeldDraft(id: string): void {
  setHeld(held().filter((entry) => entry.record.id !== id));
}

/** Keep every page still unsaved at a graph switch in the store of the graph it
 *  belongs to, `root`, before resetStore drops the working set: an edit typed
 *  while the next graph was loading, after the last flush (og T4). The window's
 *  binding has already moved, so the record names its graph explicitly and
 *  cannot land in the next one. Each switch gets its own session tag, so
 *  reopening that graph, even in this window, offers the drafts for review.
 *  Takes the snapshot before returning and moves it into `switchHeldDrafts`,
 *  so the text has a holder from the moment the working set is reset; a record
 *  leaves that holder only once its write is durable (storage.qnt mutant MX).
 *  Resolves to the names whose write failed (disk error or the store's bound;
 *  they stay held). The caller awaits it before the
 *  switch goes on. */
export function keepAtSwitch(root: string): Promise<string[]> {
  // Snapshot synchronously: the caller resets the working set right after.
  const tag = `switch-${newSessionId()}`;
  const drafts = unsavedDrafts();
  const unrepresentable = drafts.filter((draft) => !draft.page).map((draft) => draft.name);
  const records = drafts.flatMap((draft): DraftRecord[] => draft.page ? [{
    id: `${tag}:${draft.name}`, kind: "unsaved", session: tag, page_name: draft.name, path: draft.path,
    reason: draft.state === "Conflict" ? "conflict" : "save-failed", saved_at: Date.now(), page: draft.page,
  }] : []);
  if (records.length) setHeld([...held(), ...records.map((record) => ({ root, record }))]);
  return (async () => {
    // No draft form exists (pageToDto refuses a page header mid-edit): say so
    // rather than pass over it in silence.
    if (unrepresentable.length) pushToast(`Unsaved edits to ${unrepresentable.map((n) => `“${n}”`).join(", ")} could not be copied when the graph was switched.`, "error", { sticky: true });
    const lost: string[] = [];
    for (const record of records) {
      try {
        // The completion belongs to this switch, not to a graph binding (the
        // window's binding has already moved on): nothing can retire it.
        await writeOwned(ownedWhen(), stored(backend().storeDraft?.(record, root)));
        dismissHeldDraft(record.id);
      } catch {
        lost.push(record.page_name);
      }
    }
    return lost;
  })();
}

/** Retire a record an earlier session kept: the user's explicit choice. */
export async function dismissEarlierDraft(id: string): Promise<void> {
  // A panel that outlived its graph must not retire a record in the next one.
  if (earlier() === null) return refuseStaleWrite("Dismissing the kept draft");
  try {
    const result = await writeOwned(bindingOwner(), stored(backend().retireDraft?.(id)));
    if (result.kind === "current") setEarlier(earlierDrafts().filter((r) => r.id !== id));
  } catch (error) {
    pushToast(`Couldn't dismiss the kept draft (${String(error)}). It is still available.`, "error");
  }
}

async function offerEarlier() {
  let result;
  try {
    result = await readOwned(graphOwner(), (backend().loadDrafts?.() ?? Promise.resolve({ drafts: [], set_aside: null }))
      .then((load) => { noteSetAside(load.set_aside); return load.drafts; }));
  } catch (error) {
    // The backend sets an unreadable store aside (reported above), so this is a
    // disk error or a missing app-data dir: opening the graph goes on without
    // earlier drafts.
    pushToast(`Couldn't read drafts kept from an earlier session (${String(error)}).`, "error");
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
