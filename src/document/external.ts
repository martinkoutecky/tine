import { msg } from "../i18n";
import { applyGraphAnswers } from "../graphAnswers";
import { beginResultsChange } from "../queryResultsEpoch";
import { changeSummary } from "../changeSummary";
import { changeKeyPending } from "../changeKeys";
import { backend, type GraphChange, type GraphAnswersChange } from "../backend";
import { captureBinding, bindingCurrent } from "../binding";
import { conflictPolicyAlwaysAsk, holdExternalChange, installHeldExternalChangeApplier } from "../conflictPolicy";
import { pushToast } from "../toasts";
import { bindingOwner, latestOwner, readOwned } from "../owned";
import { reportUiFailure } from "../uiFailure";
import { bumpDataRev, bumpPageInventoryRev } from "../graphSession";
import { toLoadablePage } from "./convert";
import { readLoadedPublicationPages, loadedPageByPath, pageByName } from "./model";
import { admitRead, invalidatePageRead, invalidatePageReadKey } from "./readAdmission";
import { acceptHeldPush, hostHolds, isConflicted } from "./host/wiring";
import { deferExternalReload, installDeferredReloadReplay } from "./deferredReload";
import { forgetPage, rekeyPageIdentityByPath, reloadDisposition, reloadPageIfStillSafe, reportPageLoadRefusal, restoreTodayJournalInFeed } from "./workingSet";

/** Route and feed actions belong to the app; the document module owns the
 * decision to call them. The snapshot keeps one watcher event on one UI view. */
export interface ExternalChangeUi {
  pageOpen(name: string): boolean;
  journalsOpen: boolean;
  leaveRemovedPage(name: string): void;
  restartJournalFeed(): void;
}

// A change declined below is recorded for replay; the replay re-enters
// `applyGraphChange`, so the disposition is re-decided with whatever state holds
// then (deferredReload.ts; GH #337).
installDeferredReloadReplay({
  ready: (page) => reloadDisposition(page) === "reload",
  run: (change, own) => void (own ? applyObservedChange(change, captureExternalChangeUi?.(), true, true) : applyGraphChange(change))
    .catch(error => reportUiFailure("page-refresh", error)),
});

// "Always ask" (conflictPolicy.ts): Reload from disk re-enters this handler with
// the policy bypassed for that one change.
installHeldExternalChangeApplier((change) => void applyGraphChange(change, true));

let captureExternalChangeUi: (() => ExternalChangeUi) | null = null;
export function installExternalChangeUiHandler(capture: () => ExternalChangeUi): void {
  captureExternalChangeUi = capture;
}

/** Apply an already-observed watcher change to frontend graph revisions and
 * loaded pages. Reload a safe loaded page, leave a page the host holds to its
 * mail (the host detects conflicts), defer a held one, or
 * notify route/feed UI about a removal. The watcher/backend has already
 * updated disk and its cache; this function does not persist the change.
 * Page reads may reject. Cost follows the affected page and current UI state. */
export async function applyGraphChange(c: GraphChange, bypassPolicy = false): Promise<void> {
  const binding = captureBinding();
  if (c.binding_generation !== undefined && c.binding_generation !== binding.backendGeneration) return;
  invalidatePageRead(c.name);
  const loadedName = c.path && loadedPageByPath(c.path)?.name;
  if (loadedName && loadedName !== c.name) invalidatePageRead(loadedName);
  // The watcher has already updated the backend graph cache. Invalidate even
  // when this page is outside the bounded frontend working set.
  const finish = beginResultsChange();
  try {
    const previous = changeSummary();
    applyGraphAnswers(c.answers);
    // A CLI reload and the publication feed can carry the same evidence in
    // either order. Its second delivery must not turn an already released
    // precise key into the missing-summary full-refresh fallback (GH #907).
    if (!c.answers?.summary || previous !== changeSummary() || changeKeyPending("publication")) bumpDataRev(!!c.answers?.summary);
    if (c.created || c.removed) bumpPageInventoryRev();
    await applyObservedChange(c, captureExternalChangeUi?.(), bypassPolicy);
  }
  finally { finish(); }
}

/** One checkout-sized watcher batch (`graph-changed-bulk`, over 32 pages;
 *  master 1229f32fb): revisions move once, only pages something loads, shows
 *  or holds are applied (each through the same per-page decision), the journal
 *  feed restarts at most once, and one summary toast replaces per-page work.
 *  Cost O(changes) plus one page read per loaded or shown page. */
export async function applyGraphChangesBulk(bulk: { changes: GraphChange[]; binding_generation?: number; answers?: GraphAnswersChange | null }): Promise<void> {
  const binding = captureBinding();
  if (bulk.binding_generation !== undefined && bulk.binding_generation !== binding.backendGeneration) return;
  const previous = changeSummary();
  applyGraphAnswers(bulk.answers);
  const changes = bulk.changes;
  if (!changes.length) {
    // Own writes already publish their changed name keys. Only loaded pages
    // need text; the host remains the delivery owner for pages it holds.
    if (previous === changeSummary()) return;
    const summary = bulk.answers?.summary;
    if (!summary) return;
    for (const key of summary.pages) invalidatePageReadKey(key);
    const ui = captureExternalChangeUi?.();
    const loaded = await readOwned(bindingOwner(), readLoadedPublicationPages(summary.pages));
    if (loaded.kind === "stale") return;
    for (const page of loaded.value) {
      if (!bindingCurrent(binding)) return;
      if (pageByName(page.name) !== page || hostHolds(page.name)) continue;
      await applyObservedChange({ name: page.name, kind: page.kind, path: page.id, created: false, removed: false }, ui, true, true);
    }
    return;
  }
  for (const change of changes) {
    invalidatePageRead(change.name);
    const loadedName = change.path && loadedPageByPath(change.path)?.name;
    if (loadedName && loadedName !== change.name) invalidatePageRead(loadedName);
  }
  const finish = beginResultsChange();
  try {
  for (const change of changes) applyGraphAnswers(change.answers);
  const hasSummary = !!bulk.answers?.summary || changes.some(change => !!change.answers?.summary);
  if (!hasSummary || previous !== changeSummary() || changeKeyPending("publication")) bumpDataRev(hasSummary);
  if (changes.some((c) => c.created || c.removed)) bumpPageInventoryRev();
  const ui = captureExternalChangeUi?.();
  let restart = false, conflicts = 0;
  const batchUi = ui && { ...ui, restartJournalFeed: () => { restart = true; } };
  for (const c of changes) {
    if (!bindingCurrent(binding)) return;
    const name = (c.path && loadedPageByPath(c.path)?.name) || c.name;
    // A page nothing loads or shows is refetched on navigation anyway.
    if (!c.removed && !ui?.pageOpen(c.name) && !pageByName(name) && reloadDisposition(name) === "reload") continue;
    await applyObservedChange(c, batchUi, false);
    if (isConflicted(name)) conflicts++;
  }
  if (!bindingCurrent(binding)) return;
  if (restart || (ui?.journalsOpen && changes.some((c) => c.kind === "journal"))) ui?.restartJournalFeed();
  pushToast(msg("document-external-updates", { pages: changes.length, conflicts }), "info");
  } finally { finish(); }
}

async function applyObservedChange(c: GraphChange, ui: ExternalChangeUi | undefined, bypassPolicy: boolean, own = false): Promise<void> {
  const restartJournalFeed = () => {
    if (!own && c.kind === "journal") ui?.restartJournalFeed();
  };
  const loadedName = own ? pageByName(c.name)?.name : c.path ? loadedPageByPath(c.path)?.name : undefined;
  const currentName = loadedName ?? c.name;
  const loaded = pageByName(currentName);
  const owner = latestOwner(loaded ?? c, "publication", bindingOwner(() => pageByName(currentName) === loaded));
  // `c.path` names the loaded file only when `loadedName` was found by it.
  const disp = reloadDisposition(currentName, loadedName ? c.path : undefined);
  // A page the host holds for this window takes the change as host mail (its
  // text when clean, a conflict over input): nothing to reload here. "Reload
  // from disk" for a change held by "always ask" installs what the host holds;
  // a removal waits until the host lets go of the page.
  if (hostHolds(currentName)) {
    if (bypassPolicy) acceptHeldPush(currentName);
    else if (c.removed) deferExternalReload(currentName, c, own);
    restartJournalFeed();
    return;
  }
  // Input not yet with the host ("conflict") or a hold ("skip"): replayed
  // once the page is replaceable.
  if (disp !== "reload") {
    deferExternalReload(currentName, c, own);
    restartJournalFeed();
    return;
  }
  if (c.removed) {
    ui?.leaveRemovedPage(c.name);
    if (c.kind === "journal" && ui?.journalsOpen) {
      // Another file holding today's name keeps it out of the feed (og J1);
      // the restart below retries once that holder is replaceable.
      const refused = restoreTodayJournalInFeed();
      if (refused) reportPageLoadRefusal(refused);
      restartJournalFeed();
    }
    return;
  }

  // "Always ask": reached only after the conflict/skip branches, so it turns
  // the one SILENT case (a loaded, clean page) into an asked one and changes
  // nothing that already asked or deferred.
  if (!bypassPolicy && conflictPolicyAlwaysAsk() && pageByName(currentName)) {
    holdExternalChange(currentName, c);
    return;
  }
  if (pageByName(currentName) || ui?.pageOpen(c.name)) {
    const exact = c.path && loadedName && (own || loadedName !== c.name) ? c.path : null;
    let disappeared = false;
    const result = await admitRead(owner, () => exact ? backend().getPageByPath(exact) : backend().getPage(c.name, c.kind), dto => [currentName, ...(dto ? [dto.name] : [])], dto => {
      if (!dto) { disappeared = !!loaded?.id; return; }
      if (exact && dto.id !== exact) return;
      if (exact && dto.name !== currentName && !rekeyPageIdentityByPath(exact, dto.name, dto.rev ?? null)) return;
      if (!reloadPageIfStillSafe(dto.name, toLoadablePage(dto, dto.name))) deferExternalReload(currentName, c, own);
    });
    if (result.kind === "stale") return;
    if (disappeared) {
      if (!owner()) return;
      // Retirement has a stricter gate than a same-file text replacement:
      // input or a draft acquired during the read must keep its loaded owner.
      if (reloadDisposition(currentName) !== "reload") {
        deferExternalReload(currentName, c, own);
        return;
      }
      forgetPage(currentName);
      // A removed duplicate may leave a surviving day. Existing removal UI
      // leaves a path-pinned route and restarts the feed to resolve that day.
      return applyObservedChange({ ...c, name: currentName, removed: true }, ui, bypassPolicy);
    }
  }
  restartJournalFeed();
}
