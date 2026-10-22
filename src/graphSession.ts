import { msg } from "./i18n";
import { createSignal, untrack } from "solid-js";
import { pageIdentityKey } from "./pageIdentity";
import { type GraphMeta } from "./types";
import { backend } from "./backend";
import { pushToast } from "./toasts";
import { captureBinding, bindingCurrent } from "./binding";
import { bindingOwner, advanceRevision, currentRevision, serializeDurable } from "./owned";

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

// Ordered, revisioned config writes for the journal template (I-20/I-21). The
// queue keeps two quick choices from reaching config.edn out of order; the
// revision lets only the NEWEST choice roll the optimistic UI back; `confirmed`
// is the last value known to be on disk for that graph, so a rollback restores
// what is really persisted rather than a neighbouring optimistic value.
const journalTemplateWrites = {};
let confirmedTemplate: { root: string; value: string | null } | null = null;

/** Set (or clear, with null) the template applied to new journal days, persisting
 *  it to config.edn `:default-templates {:journals "Name"}` and updating the live
 *  meta so the UI reflects it immediately. Writes run one at a time in call
 *  order; a failure always shows a sticky error, and reverts the UI to the last
 *  persisted value only when it belongs to the newest choice in the same graph.
 *  Cost: one config write per call. */
export function setJournalTemplate(name: string | null) {
  const binding = captureBinding();
  const m = graphMeta();
  if (m && confirmedTemplate?.root !== m.root) {
    confirmedTemplate = { root: m.root, value: m.default_journal_template ?? null };
  }
  const root = m?.root;
  const revision = advanceRevision(journalTemplateWrites);
  if (m) setGraphMeta({ ...m, default_journal_template: name });
  void serializeDurable(journalTemplateWrites, bindingOwner(), () => backend().setDefaultJournalTemplate(name))
    .then((result) => {
      if (result.kind === "current" && root !== undefined && confirmedTemplate?.root === root) confirmedTemplate = { root, value: name };
    })
    .catch((e) => {
      // A durable failure is reported even when the graph has since changed.
      pushToast(msg("graph-session-couldn-t-save-the-journal-template-setting", { error: String(e) }), "error");
      if (!bindingCurrent(binding) || currentRevision(journalTemplateWrites) !== revision) return;
      const cur = graphMeta();
      if (cur && cur.root === root && confirmedTemplate?.root === root) {
        setGraphMeta({ ...cur, default_journal_template: confirmedTemplate.value });
      }
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
// GH #746: the last two distinct pages a save-driven bump published, and the
// last bump with no page list, so a page's own references can tell a save of
// only that page apart. O(1) per bump and per question.
let latestPage = { key: "", rev: 0 };
let earlierPage = { key: "", rev: 0 };
let unscopedRev = 0;
/** `pages`: the pages this bump published, when that is all that changed. */
export function bumpDataRev(pages?: readonly string[]) {
  const rev = untrack(dataRev) + 1;
  if (!pages?.length) unscopedRev = rev;
  for (const name of pages ?? []) {
    const key = pageIdentityKey(name);
    if (key !== latestPage.key) earlierPage = latestPage;
    latestPage = { key, rev };
  }
  setDataRev(rev);
}
/** Tracks `dataRev`; the latest revision that changed anything but page `name`. */
export function dataRevBeyond(name: string): number {
  dataRev();
  const other = latestPage.key !== pageIdentityKey(name) ? latestPage : earlierPage;
  return Math.max(unscopedRev, other.rev);
}
// Page-name inventory changes are much rarer than ordinary content saves. Keep
// their invalidation separate so navigation can refresh canonical names after a
// create/delete without turning every keystroke save into a whole-page-list IPC.
export const [pageInventoryRev, setPageInventoryRev] = createSignal(0);
export function bumpPageInventoryRev() {
  setPageInventoryRev((n) => n + 1);
}
