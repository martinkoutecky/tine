import { msg } from "../i18n";
import { applyGraphAnswers } from "../graphAnswers";
import { beginResultsChange } from "../queryResultsEpoch";
import { changeSummary } from "../changeSummary";
import { changeKeyPending } from "../changeKeys";
import { backend, type GraphChange } from "../backend";
import type { GraphChangeBatch } from "../backendTypes";
import { captureBinding, bindingCurrent } from "../binding";
import { conflictPolicyAlwaysAsk, holdExternalChange, installHeldExternalChangeApplier } from "../conflictPolicy";
import { pushToast } from "../toasts";
import { bindingOwner, latestOwner, readOwned } from "../owned";
import { reportUiFailure } from "../uiFailure";
import { bumpDataRev, bumpPageInventoryRev } from "../graphSession";
import { toLoadablePage } from "./convert";
import { readLoadedPublicationPages, loadedPageByPath, pageByName, feedNames } from "./model";
import { admitRead, invalidatePageRead, invalidatePageReadKey } from "./readAdmission";
import { graphRewriteFrozen } from "./graphRewriteState";
import { acceptHeldPush, hostHolds, isConflicted } from "./host/wiring";
import { deferExternalReload, installDeferredReloadReplay } from "./deferredReload";
import { forgetPage, rekeyPageIdentityByPath, reloadDisposition, reloadPageIfStillSafe, reportPageLoadRefusal, restoreTodayJournalInFeed } from "./workingSet";

/** Routes observed when applying a watcher event. */
export interface ExternalChangeUi {
  pageOpen(name: string): boolean;
  journalsOpen: boolean;
  leaveRemovedPage(name: string): void;
}

// Replays recheck page disposition. The initial observation releases the keys.
installDeferredReloadReplay({
  ready: (page) => !graphRewriteFrozen() && reloadDisposition(page) === "reload",
  run: (change, own, ui) => void (own ? applyObservedChange(change, ui ?? captureExternalChangeUi?.(), true, true)
    : applyExternalChange(change, false, true))
    .catch(error => reportUiFailure("page-refresh", error)),
});

installHeldExternalChangeApplier(change => void applyExternalChange(change, true, true)
  .catch(error => reportUiFailure("page-refresh", error)));

let captureExternalChangeUi: (() => ExternalChangeUi) | null = null;
export function installExternalChangeUiHandler(capture: () => ExternalChangeUi): void {
  captureExternalChangeUi = capture;
}

function invalidateChangedPage(change: GraphChange): void {
  invalidatePageRead(change.name);
  const loadedName = change.path && loadedPageByPath(change.path)?.name;
  if (loadedName && loadedName !== change.name) invalidatePageRead(loadedName);
}

function releasePublication(previous: ReturnType<typeof changeSummary>, hasSummary: boolean): void {
  // Duplicate CLI/watcher evidence still releases staged keys once (GH #907).
  // The full fallback also advances days (GH #987).
  if (!hasSummary || previous !== changeSummary() || changeKeyPending("publication")) bumpDataRev(hasSummary);
}

/** Apply an already-observed watcher change to frontend graph revisions and
 * loaded pages. Reload a safe loaded page, leave a page the host holds to its
 * mail (the host detects conflicts), defer a held one, or
 * notify routed UI about a removal. The watcher/backend has already
 * updated disk and its cache; this function does not persist the change.
 * Page reads may reject. Cost follows the affected page and current UI state. */
export function applyGraphChange(c: GraphChange, bypassPolicy = false): Promise<void> {
  return applyExternalChange(c, bypassPolicy, false);
}

async function applyExternalChange(c: GraphChange, bypassPolicy: boolean, replay: boolean): Promise<void> {
  const binding = captureBinding();
  if (c.binding_generation !== undefined && c.binding_generation !== binding.backendGeneration) return;
  const finish = beginResultsChange();
  try {
    if (!replay) {
      invalidateChangedPage(c);
      const previous = changeSummary();
      applyGraphAnswers(c.answers);
      releasePublication(previous, !!c.answers?.summary);
      if (c.created || c.removed) bumpPageInventoryRev();
    }
    await applyObservedChange(c, captureExternalChangeUi?.(), bypassPolicy);
  }
  finally { finish(); }
}

/** Apply a publication with shared evidence for all its pages. Revisions
 *  move once, and only pages something loads, shows or holds are applied.
 *  Cost O(changes) plus one page read per loaded or shown page. */
export async function applyGraphChangesBulk(bulk: GraphChangeBatch): Promise<void> {
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
  for (const change of changes) invalidateChangedPage(change);
  const finish = beginResultsChange();
  try {
    releasePublication(previous, !!bulk.answers?.summary);
    if (changes.some(c => c.created || c.removed)) bumpPageInventoryRev();
    const ui = captureExternalChangeUi?.();
    let conflicts = 0;
    const applyPage = async (c: GraphChange) => {
      if (!bindingCurrent(binding)) return;
      const name = (c.path && loadedPageByPath(c.path)?.name) || c.name;
      if (!c.removed && !ui?.pageOpen(c.name) && !pageByName(name) && reloadDisposition(name) === "reload") return;
      await applyObservedChange(c, ui, false);
      if (bulk.show_summary && isConflicted(name)) conflicts++;
    };
    if (!bulk.show_summary) {
      // Small publications keep the concurrency of the former per-page events.
      const outcomes = await Promise.allSettled(changes.map(applyPage));
      const failure = outcomes.find(outcome => outcome.status === "rejected");
      if (failure) throw failure.reason;
      return;
    }
    for (const c of changes) {
      if (!bindingCurrent(binding)) return;
      await applyPage(c);
    }
    if (bindingCurrent(binding)) {
      pushToast(msg("document-external-updates", { pages: changes.length, conflicts }), "info");
    }
  } finally { finish(); }
}

async function applyObservedChange(c: GraphChange, ui: ExternalChangeUi | undefined, bypassPolicy: boolean, own = false): Promise<void> {
  if (own && graphRewriteFrozen()) {
    deferExternalReload(c.name, c, own, ui);
    return;
  }
  const loadedName = own ? pageByName(c.name)?.name : c.path ? loadedPageByPath(c.path)?.name : undefined;
  const currentName = loadedName ?? c.name;
  const loaded = pageByName(currentName);
  if (own && c.path && loaded?.id !== c.path) return;
  const owner = latestOwner(loaded ?? c, "publication", bindingOwner(() => pageByName(currentName) === loaded));
  // `c.path` names the loaded file only when `loadedName` was found by it.
  const disp = reloadDisposition(currentName, loadedName ? c.path : undefined);
  // A page the host holds for this window takes the change as host mail (its
  // text when clean, a conflict over input): nothing to reload here. "Reload
  // from disk" for a change held by "always ask" installs what the host holds;
  // a removal waits until the host lets go of the page.
  if (hostHolds(currentName)) {
    if (bypassPolicy) acceptHeldPush(currentName);
    else if (c.removed) deferExternalReload(currentName, c, own, ui);
    return;
  }
  // Input not yet with the host ("conflict") or a hold ("skip"): replayed
  // once the page is replaceable.
  if (disp !== "reload") {
    deferExternalReload(currentName, c, own, ui);
    return;
  }
  if (c.removed) {
    ui?.leaveRemovedPage(c.name);
    if (c.kind === "journal" && ui?.journalsOpen) {
      const refused = restoreTodayJournalInFeed();
      if (refused) reportPageLoadRefusal(refused);
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
      if (own && graphRewriteFrozen()) { deferExternalReload(currentName, c, own, ui); return; }
      if (!dto) { disappeared = !!loaded?.id; return; }
      if (exact && dto.id !== exact) return;
      if (exact && dto.name !== currentName && !rekeyPageIdentityByPath(exact, dto.name, dto.rev ?? null)) return;
      if (!reloadPageIfStillSafe(dto.name, toLoadablePage(dto, dto.name))) deferExternalReload(currentName, c, own, ui);
    });
    if (result.kind === "stale") return;
    if (disappeared) {
      // An own merge can retire the displayed twin while the day stays listed.
      // Resolve that day alone so the feed can keep its slot.
      if (own && c.kind === "journal" && ui?.journalsOpen && feedNames().includes(currentName)) {
        const replacement = await admitRead(owner, () => backend().getPage(currentName, c.kind), () => [currentName], dto => {
          if (dto?.name !== currentName || dto.kind !== c.kind || !dto.id) return false;
          if (graphRewriteFrozen() || !reloadPageIfStillSafe(currentName, toLoadablePage(dto, currentName))) {
            deferExternalReload(currentName, c, own, ui);
          } else if (dto.id !== exact) {
            ui.leaveRemovedPage(currentName);
          }
          return true;
        });
        if (replacement.kind === "stale" || replacement.value) return;
      }
      if (!owner()) return;
      // Retirement has a stricter gate than a same-file text replacement:
      // input or a draft acquired during the read must keep its loaded owner.
      if (reloadDisposition(currentName) !== "reload") {
        deferExternalReload(currentName, c, own, ui);
        return;
      }
      forgetPage(currentName);
      // A removed duplicate may leave a surviving day. Retire the exact route.
      return applyObservedChange({ ...c, name: currentName, removed: true }, ui, bypassPolicy);
    }
  }
}
