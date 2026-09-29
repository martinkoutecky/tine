import { createSignal } from "solid-js";
import { type GraphMeta } from "./types";
import { backend } from "./backend";
import { pushToast } from "./toasts";
import { captureBinding, stillBound } from "./binding";

export const [graphMeta, setGraphMeta] = createSignal<GraphMeta | null>(null);

// True once the startup graph-load attempt has finished (success OR failure). The
// onboarding Welcome screen shows only when this is set AND no graph loaded — so a
// fresh install with no configured graph gets the wizard, but a normal startup
// never flashes it while the graph is still loading.
export const [firstLoadDone, setFirstLoadDone] = createSignal(false);

/** Why the graph chosen at launch could not be opened (Welcome recovery card).
 * Set by App's launch open and by the card's retry; cleared when a retry loads. */
export const [startupOpenFailure, setStartupOpenFailure] =
  createSignal<{ path: string; message: string } | null>(null);

/** Set (or clear, with null) the template applied to new journal days, persisting
 *  it to config.edn `:default-templates {:journals "Name"}` and updating the live
 *  meta so the UI reflects it immediately. */
export function setJournalTemplate(name: string | null) {
  const binding = captureBinding();
  const m = graphMeta();
  const prev = m?.default_journal_template ?? null;
  if (m) setGraphMeta({ ...m, default_journal_template: name });
  // On a config-write failure, revert the optimistic UI + tell the user, rather
  // than silently showing a template that wasn't actually persisted.
  void backend()
    .setDefaultJournalTemplate(name)
    .catch((e) => {
      if (!stillBound(binding)) return;
      const cur = graphMeta();
      if (cur) setGraphMeta({ ...cur, default_journal_template: prev });
      pushToast(`Couldn't save the journal template setting. (${String(e)})`, "error");
    });
}
// Bumped when the open graph changes, so views reload against the new graph.
export const [graphEpoch, setGraphEpoch] = createSignal(0);
export function bumpGraphEpoch() {
  setGraphEpoch((n) => n + 1);
}

// Bumped after a save batch lands (the Rust cache now reflects the edit), so
// derived whole-graph views — {{query}} results, backlinks — can recompute.
// This is Tine's stand-in for OG's reactive-DB query invalidation.
export const [dataRev, setDataRev] = createSignal(0);
export function bumpDataRev() {
  setDataRev((n) => n + 1);
}
// Page-name inventory changes are much rarer than ordinary content saves. Keep
// their invalidation separate so navigation can refresh canonical names after a
// create/delete without turning every keystroke save into a whole-page-list IPC.
export const [pageInventoryRev, setPageInventoryRev] = createSignal(0);
export function bumpPageInventoryRev() {
  setPageInventoryRev((n) => n + 1);
}
