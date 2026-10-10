// Opening / switching the active graph from the UI (native folder picker),
// persisting the choice so it reopens next launch.

import { closeAllWorkspaceWindows } from "./workspaceWindows";
import { backend, type CustomCssChange, type GraphConfigChange } from "./backend";
import { captureBinding, bindingCurrent } from "./binding";
import { bindingOwner, graphOwner, readOwned, writeOwned, type Owner } from "./owned";
import { setGraphMeta, bumpGraphEpoch, bumpDataRev, graphMeta, graphEpoch } from "./graphSession";
import { setWorkflow, setRightSidebar, seedFavorites, favorites, pruneSidebarBlocks, refreshJournalConflicts, refreshSyncConflicts, clearRecent, graphTransitioning, setGraphTransitioning, renamePageInNavigation, resetLeftSidebarSections, closePageProps, setAudioPlayer } from "./ui";
import { createSignal } from "solid-js";
import { dismissToast, pushToast } from "./toasts";
import { restartTine } from "./update";
import { bindHost, resetStore, flushAll, createPage, journalTemplatePage, demoJournalPage, installRenameRefreshHandler, renamePageOnDisk, favoritesArrangementPage, favoritesArrangementBlocks, reloadHlsIfLoaded, tryFreezeGraphRewrite } from "./document";
import { installFavoritesPageDoor } from "./favorites";
import { clearAssetBlobCache } from "./assetCache";
import { resetTabsToJournals, openPage, restoreSession, flushSession, type PageTarget } from "./router";
import { resetPaneLayoutToSingle, removePageTargetAcrossPanes } from "./panes";
import { journalTitle, localDayKey, setJournalTitleFormat, appNow } from "./journal";
import { applyTemplateVars, prepareTemplateVars } from "./editor/templateVars";
import { resetPageIndex } from "./pageIndex";
import { applyCustomCss } from "./customCss";
import { isMobile, platformKind } from "./platform";
import type { BlockDto, GraphMeta } from "./types";
import type { DraftStatus } from "./document";
import { maybeShowGuideAnnouncement } from "./guide";
import { endEdit } from "./editorController";
import { journalHasContent } from "./journalContent";
import { activatePdfOwnership, drainPdfWork, retirePdfOwnership } from "./pdfOwnership";
import { openConfiguredHomePage } from "./homePage";
import { isPublishedExport } from "./publishedBackend";
import { clearWorkspaces } from "./workspaces";
import { reportUiFailure } from "./uiFailure";
import { reportGraphOpenFailure } from "./graphOpenFailure";
import { activeElement, isHTMLElementNode } from "./windowRealm";
import { askGraphName } from "./graphNamePrompt";
export const [graphConfigProblem, setGraphConfigProblem] = createSignal<unknown>(null);

const GRAPH_KEY = "tine.graphPath";

/** Apply the store's fresh config snapshot without reopening the graph. A
 * superseded binding or other root is ignored; matching visible favorites
 * keep their arrangement. A moved journal title format or new-page format
 * installs the title format before publishing meta or epoch, so journal
 * observers use the new format and in-flight results dated under the old one
 * are dropped (master: same ordering rule as graph bind). Cost: O(favorites),
 * plus one arrangement page read only when membership changes. No write or
 * observable error here. */
export function applyGraphConfigChange(change: GraphConfigChange): void {
  const previous = graphMeta();
  if (!previous || previous.root !== change.meta.root
      || captureBinding().backendGeneration !== change.binding_generation) return;
  const meta = change.meta;
  applyConfigDerivedState(meta, previous);
  bumpDataRev();
}

/** `logseq/custom.css` changed on disk (an editor, Syncthing, another window):
 *  re-read and re-apply it, no graph reopen. An event from an older graph
 *  binding is ignored; the read itself is graph-owned. */
export function applyCustomCssChange(change: CustomCssChange): void {
  if (change.binding_generation !== undefined && captureBinding().backendGeneration !== change.binding_generation) return;
  void injectCustomCss(true);
}

/** Publish one config snapshot and its derived state (I-12). The journal
 *  title format lands before meta or epoch can wake journal observers; the
 *  epoch invalidates older reads before favorites start their arrangement
 *  read. Task workflow, title format, meta publication and favorites have
 *  this one producer. Graph open passes `previous = null` (apply everything); a live
 *  config change passes the meta it replaces, so only what moved is applied,
 *  and favorites the user is already shown are not re-seeded (Tine's own
 *  settings writes reach here too). Everything else on GraphMeta (shortcuts,
 *  macros, hidden properties, start of week, …) updates from the signal. Cost: O(favorites), plus one arrangement page
 *  read when membership or the arrangement page moved. */
export function applyConfigDerivedState(meta: GraphMeta, previous: GraphMeta | null): void {
  if (!previous || previous.preferred_workflow !== meta.preferred_workflow)
    setWorkflow(meta.preferred_workflow === "todo" ? "todo" : "now");
  if (!previous || previous.journal_page_title_format !== meta.journal_page_title_format)
    setJournalTitleFormat(meta.journal_page_title_format);
  setGraphMeta(meta);
  if (!previous || previous.journal_page_title_format !== meta.journal_page_title_format
      || previous.preferred_format !== meta.preferred_format) bumpGraphEpoch();
  const incoming = meta.favorites ?? [];
  const alreadyShown = previous !== null && previous.favorites_page === meta.favorites_page
    && sameNames(favorites().map((item) => item.name), incoming);
  if (!alreadyShown) seedFavorites(incoming, meta.favorites_page ?? null);
}

function sameNames(shown: string[], incoming: string[]): boolean {
  return shown.length === incoming.length && shown.every((name, index) => name === incoming[index]);
}

export function persistedGraphPath(): string {
  try {
    return localStorage.getItem(GRAPH_KEY) ?? "";
  } catch {
    return "";
  }
}

/** Load a graph by path ("" → backend uses env/CLI). Updates meta, persists a
 *  non-empty path, and reloads the views. */
export type LoadGraphPathOutcome =
  | { kind: "loaded"; root: string }
  | { kind: "already_current"; root: string }
  | { kind: "focused_existing" }
  | { kind: "aborted" };

/** Inspect a graph's external-assets target. Return true immediately when no
 * external target exists or this device has already approved it. Otherwise
 * show the resolved target and request consent, persisting approval on this
 * device. Denial or stale graph ownership returns false; inspection or
 * approval errors reject. Cost follows path inspection and one approval write. */
export async function authorizeGraphAccess(path: string): Promise<boolean> {
  const owner = bindingOwner();
  const inspected = await readOwned(owner, backend().inspectGraphAccess(path));
  if (inspected.kind === "stale") return false;
  const access = inspected.value;
  const external = access.external_assets_path;
  if (!external || access.approved) return true;
  const confirmation = await readOwned(owner, backend().confirm(
    `This graph's assets folder points outside the graph to:\n\n${external}\n\nAllow Tine to read and write assets in this directory? This approval is stored only on this device.`,
    "Allow external assets directory?"
  ));
  if (confirmation.kind === "stale") return false;
  if (!confirmation.value) {
    pushToast(
      `Graph not opened: its external assets directory was not approved (${external}).`,
      "error",
      { sticky: true }
    );
    return false;
  }
  const approved = await writeOwned(owner, backend().approveExternalAssets(access.graph_root, external));
  return approved.kind === "current";
}

export async function loadGraphPath(
  requestedPath: string,
  options: { forceRefresh?: boolean; transitionHeld?: boolean } = {}
): Promise<LoadGraphPathOutcome> {
  // The path Rust opens and localStorage remembers. On iOS the native boundary
  // may rebase it below (app-container relocation after an update).
  let path = requestedPath;
  const startingBinding = captureBinding();
  const ownsTransition = !options.transitionHeld;
  if (graphTransitioning() && ownsTransition) return { kind: "aborted" };
  if (ownsTransition) {
    setGraphTransitioning(true);
  }
  let unfreeze: (() => void) | null = null;
  try {
  if (ownsTransition) {
    const active = activeElement();
    if (isHTMLElementNode(active)) active.blur();
    endEdit("graph-switch");
    // Let the textarea blur handler commit its final buffer before we inspect dirty.
    await Promise.resolve();
    if (!bindingCurrent(startingBinding)) return { kind: "aborted" };
  }
  // Persist the current graph's pending edits BEFORE opening another graph —
  // otherwise the debounced save would either fire against the new graph or be
  // dropped by resetStore. No-op on first load (nothing dirty). If something
  // couldn't be saved (conflict / disk error), abort so resetStore doesn't
  // discard that edit — gated on whether a graph is actually loaded now, NOT on
  // the persisted path (which is empty on a TINE_GRAPH/CLI launch).
  const hadGraph = !!graphMeta();
  const flushed = await flushAll();
  if (!bindingCurrent(startingBinding)) return { kind: "aborted" };
  if (hadGraph && !flushed) {
    pushToast("Some pages couldn't be saved — resolve conflicts before switching graphs.", "error");
    return { kind: "aborted" };
  }
  if (hadGraph) {
    // Session layout is best effort across a graph switch: flushSession keeps its
    // Retry toast, but a full or unwritable app-data dir must not trap the user here.
    try { await flushSession(); }
    catch { console.warn("Session not saved before graph switch"); }
  }
  if (!bindingCurrent(startingBinding)) return { kind: "aborted" };
  if (path && (await platformKind()) === "ios") {
    // iOS opens only folders inside the app's own Documents or iCloud
    // containers (master 027b5ae15, fa78c7b74). The native boundary downloads
    // an iCloud graph before Rust enumerates it and rebases a remembered
    // Documents-relative path onto the current container after an app update.
    let prepared;
    try {
      const result = await readOwned(bindingOwner(), backend().prepareGraphFolder(path));
      if (result.kind === "stale") return { kind: "aborted" };
      prepared = result.value;
    } catch (error) {
      pushToast(`Couldn't prepare the iCloud graph. (${String(error)})`, "error", { sticky: true });
      return { kind: "aborted" };
    }
    if (!bindingCurrent(startingBinding)) return { kind: "aborted" };
    if (prepared.status !== "ready") {
      pushToast(
        "TineOutline can only open folders inside On My iPhone or iCloud Drive → TineOutline.",
        "error",
        { sticky: true }
      );
      return { kind: "aborted" };
    }
    path = prepared.path ?? path;
  }
  // Whether we're switching to a *different* graph than last time. Only then do
  // we drop the persisted right-sidebar items; reopening the same graph at
  // startup keeps them (and we prune stale block refs below). Computed after
  // the iOS rebase so a container relocation is not mistaken for a switch.
  const prev = graphMeta()?.root || persistedGraphPath();
  const switching = !!prev && !!path && prev !== path;
  const rebindsPdfOwner = hadGraph && (switching || options.forceRefresh === true);
  if (!(await authorizeGraphAccess(path))) return { kind: "aborted" };
  if (!bindingCurrent(startingBinding)) return { kind: "aborted" };
  // This is the last await before the backend graph binding can change.  Flush
  // delayed view state plus complete highlight/area mutations under A; only a
  // successful drain permits us to invalidate that authority and unmount it.
  if (rebindsPdfOwner && !(await drainPdfWork())) {
    pushToast("PDF changes couldn't be saved — the current graph is still open.", "error");
    return { kind: "aborted" };
  }
  if (!bindingCurrent(startingBinding)) return { kind: "aborted" };
  if (rebindsPdfOwner) {
    retirePdfOwnership();
  }
  // The saved session lists this graph's workspace windows; on a switch they
  // close because their panes belong to the graph being left (a same-graph
  // refresh keeps them, like main's panes). Here, after the access prompt and
  // the PDF drain could still abort (review F11), and before the last flush, so
  // an edit a window's disposal commits is still written.
  if (hadGraph && switching) closeAllWorkspaceWindows("graph-switch");
  // STEP3 §6 step 1: freeze every window's input, workspace windows included,
  // until the old working set is reset (or the switch aborts), so nothing typed
  // after the last flush can be left without custody (storage.qnt MX). The
  // focused editor commits first, as the rename freeze does.
  if (hadGraph) {
    const focused = activeElement();
    if (isHTMLElementNode(focused)) focused.blur();
    unfreeze = tryFreezeGraphRewrite();
    if (!unfreeze) {
      if (rebindsPdfOwner && prev) activatePdfOwnership(prev);
      pushToast("A rename is still running — switch graphs once it finishes.", "error");
      return { kind: "aborted" };
    }
    endEdit("graph-switch");
  }
  // An edit can land during the awaits since the first flush (session save,
  // access prompt, PDF drain); resetStore would discard it with the old
  // working set. Flush once more as the last await before the binding moves.
  if (hadGraph && !(await flushAll())) {
    if (rebindsPdfOwner && prev) activatePdfOwnership(prev);
    pushToast("Some pages couldn't be saved — resolve conflicts before switching graphs.", "error");
    return { kind: "aborted" };
  }
  if (!bindingCurrent(startingBinding)) {
    if (rebindsPdfOwner && prev) activatePdfOwnership(prev);
    return { kind: "aborted" };
  }

  let result;
  // An overlapping graph still saving its pages (closing elsewhere) makes the
  // open wait for it (F6); say so while it does.
  let waitingToast: number | null = null;
  const stopWaiting = await backend().onGraphOpenWaiting((saving) => {
    waitingToast ??= pushToast(`Waiting for Tine to finish saving “${saving}” before opening this graph…`, "info", { sticky: true });
  }).catch(() => () => {});
  try {
    result = await backend().loadGraph(path);
  } catch (error) {
    // load_graph failed before installing a replacement binding.  Publish a new
    // local generation for the still-bound old graph; the retired viewer stays
    // closed, so no callback can regain its former authority.
    if (rebindsPdfOwner && prev) activatePdfOwnership(prev);
    throw error;
  } finally {
    stopWaiting();
    if (waitingToast !== null) dismissToast(waitingToast);
  }
  if (result.kind === "focused_existing") {
    if (rebindsPdfOwner && prev) activatePdfOwnership(prev);
    return { kind: "focused_existing" };
  }
  const meta = result.meta;
  if (result.kind === "already_current" && hadGraph && !options.forceRefresh) {
    return { kind: "already_current", root: meta.root };
  }
  if (!hadGraph || rebindsPdfOwner) activatePdfOwnership(meta.root);
  resetStore();
  unfreeze?.();
  unfreeze = null;
  await bindHost();
  reportDraftStatus(result.draft_status ?? null);
  void reportLegacyDrafts();
  clearWorkspaces();
  closePageProps();
  setAudioPlayer(null);
  resetPageIndex();
  clearAssetBlobCache(); // old graph's image blob URLs must not leak into the new one
  if (switching) {
    // A graph switch is a full workspace reset (OG opens one graph at a time):
    // drop the old graph's right-sidebar items and its recent-pages list so they
    // don't linger in the sidebar / quick-switch. Tabs are reset further below.
    setRightSidebar([]);
    clearRecent();
  }
  if (switching || !hadGraph) resetLeftSidebarSections();
  // The shared config door publishes the title format before meta/epoch,
  // then starts derived reads in that epoch (including force refresh).
  applyConfigDerivedState(meta, null);
  const configProblem = "config_problem" in result ? result.config_problem : null;
  setGraphConfigProblem(configProblem);
  if (configProblem) reportUiFailure("config-read", configProblem);
  void refreshJournalConflicts(); // duplicate days surface through the conflict queue, not a toast
  void refreshSyncConflicts(); // conflict copies + VCS markers feed the sidebar badge
  if (path) {
    try {
      localStorage.setItem(GRAPH_KEY, path);
    } catch {
      // ignore
    }
  }
  // The default journal template is not materialized here: the visible
  // Journals surface owns that and awaits it before its feed read. Awaiting it
  // here made every open pay getPage + listTemplates, which wait for the
  // whole-graph parse (master 5bb8ce020, GH #266).
  void injectCustomCss();
  if (!switching) void pruneSidebarBlocks();
  maybeShowGuideAnnouncement();
  // On a genuine graph SWITCH, close ALL the old graph's tabs (their histories
  // point at pages that don't exist in the new graph) and land on a single fresh
  // Journals tab. On the initial startup load of the same graph, `restoreSession()`
  // has already set up the tabs and focused one — leave that untouched, else a
  // restored pinned page tab would revert to Journals after every relaunch.
  if (switching) {
    resetTabsToJournals();
    resetPaneLayoutToSingle();
    await restoreSession();
  } else if (!hadGraph) {
    // Upgrade/first-bind fallback: main.tsx may have probed the old global
    // session before the backend knew which graph this webview would own.
    await restoreSession();
  }
  // A configured home page (config.edn `:default-home`) replaces the landing
  // on an ordinary open — first bind or switch, never a same-graph refresh —
  // unless a rebind or navigation lands first.
  if (result.kind === "loaded" && (switching || !hadGraph)) {
    if (isPublishedExport()) await openConfiguredHomePage();
    else void openConfiguredHomePage();
  }
  return { kind: result.kind, root: meta.root };
  } finally {
    unfreeze?.();
    if (ownsTransition) setGraphTransitioning(false);
  }
}

/** Crash-draft storage the new host could not use (STEP3 §4): saves keep
 * working; Retry re-probes it, and drafts it found but could not list are
 * recovered at the next launch (M1, Q-P1-2), so the desktop offers Restart.
 * The pages whose text is not on disk yet are named at every open (SPEC-s2
 * §4.11): what launch recovered (STEP3 §9), and what a host kept from a
 * closed window still holds, with why (B-QA). */
function reportDraftStatus(status: DraftStatus | null): void {
  const unsaved = status?.unsaved ?? [];
  const recovered = unsaved.filter((page) => page.recovered);
  if (recovered.length) {
    pushToast(`Recovered unsaved edits on ${recovered.length} page(s) — Tine is saving them: ${recovered.map((page) => page.path).join(", ")}.`,
      "info", { sticky: true });
  }
  const held = unsaved.filter((page) => !page.recovered);
  if (held.length) {
    const why = (page: (typeof held)[number]) => page.conflict ? " (changed on disk; open it to choose)"
      : page.failing ? " (saving fails)" : "";
    pushToast(`${held.length} page(s) still have edits that are not on disk; Tine keeps saving them: ${held.map((page) => page.path + why(page)).join(", ")}.`,
      held.some((page) => page.conflict || page.failing) ? "error" : "info", { sticky: true });
  }
  if (status?.unreadable.length) {
    pushToast(`Tine left ${status.unreadable.length} unreadable crash-recovery file(s) in place: ${status.unreadable.join(", ")}.`, "error", { sticky: true });
  }
  if (!status?.unavailable) return;
  const id: number = pushToast(`Crash recovery is off for this graph: ${status.unavailable}. Your edits still save.`, "error",
    { sticky: true, action: { label: "Retry", run: () => { dismissToast(id); void retryDrafts(); } } });
}

async function retryDrafts(): Promise<void> {
  try {
    if ((await writeOwned(bindingOwner(), backend().pageDraftsRetry())).kind === "stale") return;
    pushToast("Crash recovery is on again.", "success");
  } catch (error) {
    const restart = (await platformKind()) === "desktop";
    pushToast(`Crash recovery is still off: ${String(error)}.`, "error", restart
      ? { sticky: true, action: { label: "Restart Tine", run: () => void restartTine() } }
      : { sticky: true });
  }
}

/** The v1 crash-draft file stays untouched as a backup (D-1): name it once,
 * not again when the same graph reopens in this window. */
let legacyDraftsReported: string | null = null;
async function reportLegacyDrafts(): Promise<void> {
  let read;
  try { read = await readOwned(bindingOwner(), backend().legacyDraftsFile()); }
  catch {
    console.warn("Could not check for crash-recovery copies from an older Tine");
    return;
  }
  const file = read.kind === "current" ? read.value : null;
  if (!file || legacyDraftsReported === file) return;
  legacyDraftsReported = file;
  pushToast(`Crash-recovery copies from an older Tine are kept untouched in ${file}.`, "info");
}

/** Refresh frontend state after a successful page rename. The backend rename
 *  rewrites `[[refs]]` across many files through the self-write guard. The
 *  document intent refreshes the loaded pages it touched (GH #535). Refresh the
 *  app's navigation and graph-derived views, then navigate to the new name. */
export function refreshAfterRename(from: string, to: string, exactTarget?: PageTarget): void {
  if (exactTarget) {
    removePageTargetAcrossPanes(exactTarget);
    renamePageInNavigation(exactTarget, { name: to, pageKind: exactTarget.pageKind });
  } else {
    renamePageInNavigation(from, to);
  }
  // The epoch bump refreshes the page index (`pageIndex.ts`).
  resetPageIndex();
  bumpGraphEpoch();
}

installRenameRefreshHandler(refreshAfterRename);
installFavoritesPageDoor({ createPage, favoritesArrangementPage, favoritesArrangementBlocks, reloadHlsIfLoaded });

export type RenameOutcome = Exclude<Awaited<ReturnType<typeof renamePageOnDisk>>, "stale"> | "cancelled";

/** Rename a page; when `to` reaches one other page (its file, or the owner of
 *  that alias), ask to merge into it as OG Logseq does (GH #327,
 *  `merge-pages!`): the backend appends the source's blocks, unites aliases,
 *  rewrites references and trashes the source in one transaction; a `from`
 *  with no file only has its references repointed. `cancelled`: the user
 *  declined or the graph changed before anything was written. The other
 *  outcomes are `renamePageOnDisk`'s; `renameOutcomeMessage` words them.
 *  `onRefreshed` receives the graph owner captured after the rename's refresh.
 *  The merge target is not re-resolved after the confirm: the backend re-checks
 *  it. A `to` reaching several pages is not offered as a merge; the backend
 *  refuses that rename. Backend errors reject. Cost: one or two name resolutions, a confirm dialog
 *  when merging, plus the rename. */
export async function renameOrMergePage(
  from: string, to: string, target?: PageTarget, onRefreshed?: (owner: Owner) => void,
): Promise<RenameOutcome> {
  const owner = bindingOwner();
  const found = await readOwned(owner, backend().resolvePage(to, "page"));
  if (found.kind === "stale") return "cancelled";
  const reached = found.value.kind === "existing" ? (found.value.others.length ? [] : [found.value.id])
    : found.value.kind === "alias" ? found.value.owners : [];
  let into: string | undefined;
  if (reached.length === 1) {
    const source = target?.path ?? await readOwned(owner, backend().resolvePage(from, target?.pageKind ?? "page"))
      .then((own) => own.kind !== "stale" && own.value.kind === "existing" ? own.value.id : undefined);
    if (source !== reached[0]) into = reached[0];
  }
  if (into) {
    const confirmed = await readOwned(owner, backend().confirm(`Page “${to}” already exists. Merge “${from}” into it?`));
    if (confirmed.kind === "stale" || !confirmed.value) return "cancelled";
  }
  const done = await renamePageOnDisk(from, to, target, into, onRefreshed);
  return done === "stale" ? "cancelled" : done;
}

/** The user-facing message for a rename that did not rename, or null (for
 *  `renamed`, `merged` and `cancelled`). `unchanged` is worded by its cause: a
 *  spelling already in use, or a name no file and no reference uses. */
export function renameOutcomeMessage(outcome: RenameOutcome, from: string, to: string): string | null {
  if (typeof outcome === "object") {
    return outcome.mentions
      ? `Couldn't rename: “${outcome.unsaved}” has changes Tine could not save, and they mention “${from}”, so the rename could not update them. Save or discard those changes, then rename again. Your pending edits are still here.`
      : `Couldn't rename: “${outcome.unsaved}” has changes Tine could not save. Save or discard them, then rename again. Your pending edits are still here.`;
  }
  switch (outcome) {
    case "unchanged": return from.trim() === to.trim()
      ? `Nothing renamed: “${to}” already has that spelling.`
      : `Nothing renamed: no page file or reference uses “${from}” yet.`;
    case "busy": return "Another rename is still rewriting the graph. Try again when it finishes.";
    case "uncertain": return `The graph changed while renaming “${from}”. Check whether “${to}” exists before trying again.`;
    default: return null;
  }
}

export type JournalTemplateEnsureResult = "ready" | "deferred" | "stale" | { kind: "error"; error: unknown };

type JournalTemplateOwner = { root: string; epoch: number; day: number; title: string; template: string };
let journalTemplateFlight: { owner: JournalTemplateOwner; promise: Promise<JournalTemplateEnsureResult> } | null = null;

function templateOwnerCurrent(owner: JournalTemplateOwner): boolean {
  const meta = graphMeta();
  return !!meta && meta.root === owner.root && meta.default_journal_template === owner.template
    && graphEpoch() === owner.epoch && localDayKey() === owner.day;
}

/** Ensure a configured template is present before the feed reads this local day.
 * Reads today's page and the backend template inventory; cost is O(templates in
 * graph + blocks in the chosen template + page save). Concurrent refreshes
 * share a flight. `ready` means no template, existing content, absent named
 * template, or a successful guarded write; it does not prove insertion.
 * `deferred` means a write guard refused or an alias conflicted; `error`
 * carries a read/write failure for the feed's route error/toast policy.
 * `stale` means the graph, template or local day changed, possibly after a
 * successful save. A cold named lookup can scan O(graph pages) preambles;
 * the day read, template inventory, expansion and save add their own cost.
 * The write uses the document's edit-kind and base-revision door. */
export async function ensureJournalTemplateForDay(
  date: Date,
  canWrite: () => boolean = () => true,
): Promise<JournalTemplateEnsureResult> {
  const meta = graphMeta();
  const template = meta?.default_journal_template;
  if (!meta || !template) return "ready";
  const owner: JournalTemplateOwner = {
    root: meta.root, epoch: graphEpoch(), day: localDayKey(date), title: journalTitle(date), template,
  };
  if (!templateOwnerCurrent(owner)) return "stale";
  try { if (!canWrite()) return "deferred"; }
  catch (error) { return { kind: "error", error }; }
  if (journalTemplateFlight &&
      journalTemplateFlight.owner.root === owner.root &&
      journalTemplateFlight.owner.epoch === owner.epoch &&
      journalTemplateFlight.owner.day === owner.day &&
      journalTemplateFlight.owner.template === owner.template) return journalTemplateFlight.promise;

  const binding = captureBinding();
  const promise = (async (): Promise<JournalTemplateEnsureResult> => {
    try {
      const page = await readOwned(graphOwner(), backend().getPage(owner.title, "journal"));
      if (page.kind === "stale" || !templateOwnerCurrent(owner)) return "stale";
      const existing = page.value;
      if (existing && journalHasContent(existing.blocks)) return "ready";
      const templates = await readOwned(graphOwner(), backend().listTemplates());
      if (templates.kind === "stale" || !templateOwnerCurrent(owner)) return "stale";
      const tmpl = templates.value.find((t) => t.name === owner.template);
      if (!tmpl) return "ready";
      await prepareTemplateVars();
      if (!templateOwnerCurrent(owner)) return "stale";
      if (!canWrite()) return "deferred";
      const resolve = (b: BlockDto): BlockDto => ({
        id: "", raw: applyTemplateVars(b.raw, owner.title), collapsed: false,
        children: b.children.map(resolve),
      });
      const resolution = existing?.id ? null : await readOwned(graphOwner(), backend().resolvePage(owner.title, "journal"));
      if (resolution?.kind === "stale" || !templateOwnerCurrent(owner)) return "stale";
      if (!canWrite()) return "deferred";
      const resolved = resolution?.value ?? null;
      if (resolved?.kind === "alias") return "deferred";
      await createPage(owner.title, journalTemplatePage(owner.title, tmpl.blocks.map(resolve), existing), {
        baseRev: existing?.rev ?? null,
        bindingGeneration: binding.backendGeneration,
      });
      return templateOwnerCurrent(owner) ? "ready" : "stale";
    } catch (error) {
      return templateOwnerCurrent(owner) ? { kind: "error", error } : "stale";
    }
  })();
  const flight = { owner, promise };
  journalTemplateFlight = flight;
  try { return await promise; }
  finally { if (journalTemplateFlight === flight) journalTemplateFlight = null; }
}

/** Newest custom.css read: an older read that lands after a newer one is dropped. */
let customCssRead = 0;
/** A live re-read that fails once is tried again after this pause, so an editor's
 *  delete-then-write save (or a Windows editor briefly holding the file) is not
 *  reported. One retry, not a poll. */
const CUSTOM_CSS_LIVE_RETRY_MS = 400;

/** Load the graph's logseq/custom.css into a <style> tag (user theming).
 *  `live` is a re-read after an outside edit: a read that fails keeps the last
 *  applied stylesheet (it is not blanked), is retried once, and is reported only
 *  if the retry fails too. The graph-open read blanks on failure (nothing of a
 *  previous graph may stay applied). Latest read wins. */
export async function injectCustomCss(live = false): Promise<void> {
  const seq = ++customCssRead;
  const owner = graphOwner();
  for (let attempt = 0; ; attempt++) {
    try {
      const result = await readOwned(owner, backend().readCustomCss());
      if (result.kind === "stale") return;
      if (!owner()) return;
      if (seq !== customCssRead) return;
      applyCustomCss(result.value);
      return;
    } catch (error) {
      if (!owner()) return;
      if (seq !== customCssRead) return;
      if (live && attempt === 0) {
        await new Promise((resolve) => setTimeout(resolve, CUSTOM_CSS_LIVE_RETRY_MS));
        if (!owner()) return;
        if (seq !== customCssRead) return;
        continue;
      }
      reportUiFailure("custom-css", error);
      if (!live) applyCustomCss("");
      return;
    }
  }
}

/** Open a graph chosen with the desktop folder picker or the Android/iOS graph
 * picker. Android may request all-files access and return aborted until
 * granted; iOS refuses folders outside the app's On My iPhone / iCloud Drive
 * containers with an info toast and returns aborted. Cancellation,
 * permission refusal and stale ownership return aborted; a successful pick
 * delegates to loadGraphPath, which flushes the old graph before switching.
 * Desktop picker errors reject; graph-load cost follows graph files. */
export async function switchGraph(): Promise<LoadGraphPathOutcome> {
  const owner = bindingOwner();
  const platform = await platformKind();
  if (!owner()) return { kind: "aborted" };
  if (platform === "android" || platform === "ios") {
    let result;
    try {
      const picked = await readOwned(owner, backend().pickGraphFolder());
      if (picked.kind === "stale") return { kind: "aborted" };
      result = picked.value;
    } catch (e) {
      const platformName = platform === "android" ? "Android" : "iOS";
      pushToast(`Couldn't open the ${platformName} folder picker. (${String(e)})`, "error");
      return { kind: "aborted" };
    }
    // Diagnostic breadcrumbs (visible in `adb logcat`, chromium console channel):
    // an intermittent first-run stall on "Opening…" — these pin down whether the
    // native picker returned and whether the graph parse completed or hung.
    // I-5: console text stays fixed, so each platform spells its own line.
    if (platform === "android") console.info("[tine/android] pickGraphFolder completed");
    else console.info("[tine/ios] pickGraphFolder completed");
    if (result.status === "picked") {
      if (result.path) {
        if (platform === "android") console.info("[tine/android] loadGraphPath: start");
        else console.info("[tine/ios] loadGraphPath: start");
        const outcome = await openPickedGraphPath(result.path);
        if (platform === "android") console.info("[tine/android] loadGraphPath: done");
        else console.info("[tine/ios] loadGraphPath: done");
        return outcome;
      }
      return { kind: "aborted" };
    }
    if (platform === "android" && result.status === "local-folder-required") {
      pushToast(
        "Tine needs a folder on the device's own storage. This provider folder can't be opened directly. Use your provider's sync-to-local-folder feature, then pick that local folder in Open graph.",
        "error"
      );
    }
    if (
      platform === "android" &&
      (result.status === "permission-requested" || result.status === "permission-needed")
    ) {
      pushToast('Grant "All files access" for Tine, then tap Open again.', "info");
    }
    if (platform === "ios" && result.status === "refused") {
      pushToast(
        "Choose a folder inside On My iPhone or iCloud Drive → TineOutline. Other Files providers aren't supported yet.",
        "info"
      );
    }
    return { kind: "aborted" };
  }
  const picked = await readOwned(owner, backend().pickFolder());
  if (picked.kind === "stale") return { kind: "aborted" };
  return picked.value ? openPickedGraphPath(picked.value) : { kind: "aborted" };
}

/** Open a folder the picker returned; a failure becomes a sticky toast whose
 * Retry reopens this same path (never the picker) and the call resolves
 * aborted. Cost follows loadGraphPath. */
async function openPickedGraphPath(path: string): Promise<LoadGraphPathOutcome> {
  try {
    return await loadGraphPath(path);
  } catch (error) {
    reportGraphOpenFailure(error, () => void openPickedGraphPath(path));
    return { kind: "aborted" };
  }
}

/** Create a demo graph under a desktop-picked folder, the Android default graph
 * parent, or an iOS-picked folder inside the app's On My iPhone / iCloud Drive
 * containers (anything else is refused with a toast), then open it and navigate to Welcome to Tine. Cancellation
 * returns aborted. A created directory can remain if opening fails; today's
 * narrated journal seed is best effort and its failure does not change a
 * loaded outcome. Creation/opening errors toast and return aborted; picker/parent errors
 * reject. Cost follows graph creation, templates and the graph load. */
export async function createNewGraph(): Promise<LoadGraphPathOutcome> {
  const owner = bindingOwner();
  const platform = await platformKind();
  if (!owner()) return { kind: "aborted" };
  let dir: string | null;
  if (platform === "ios") {
    const picked = await readOwned(owner, backend().pickGraphFolder());
    if (picked.kind === "stale") return { kind: "aborted" };
    if (picked.value.status === "refused") {
      pushToast(
        "Choose a folder inside On My iPhone or iCloud Drive → TineOutline. Other Files providers aren't supported yet.",
        "info"
      );
      return { kind: "aborted" };
    }
    dir = picked.value.status === "picked" ? picked.value.path : null;
  } else {
    const dirResult = (await isMobile())
      ? await readOwned(owner, backend().defaultGraphParent())
      : await readOwned(owner, backend().pickFolder("Choose where to create your new graph"));
    if (dirResult.kind === "stale") return { kind: "aborted" };
    dir = dirResult.value;
  }
  if (!dir) return { kind: "aborted" };
  if (platform === "ios") {
    let prepared;
    try {
      const result = await readOwned(owner, backend().prepareGraphFolder(dir));
      if (result.kind === "stale") return { kind: "aborted" };
      prepared = result.value;
    } catch (error) {
      pushToast(`Couldn't prepare the iCloud location. (${String(error)})`, "error", { sticky: true });
      return { kind: "aborted" };
    }
    if (prepared.status !== "ready") {
      pushToast(
        "TineOutline can only create graphs inside On My iPhone or iCloud Drive → TineOutline.",
        "error",
        { sticky: true }
      );
      return { kind: "aborted" };
    }
    dir = prepared.path ?? dir;
  }
  let root: string;
  try {
    const suggested = await readOwned(owner, backend().suggestGraphName(dir));
    if (suggested.kind === "stale") return { kind: "aborted" };
    const parent = dir;
    const created = await askGraphName(suggested.value, async name => {
      if (!owner()) return null;
      const result = await writeOwned(owner, backend().createGraph(parent, name));
      return result.kind === "stale" ? null : result.value;
    });
    if (created === null || !owner()) return { kind: "aborted" };
    root = created;
  } catch (e) {
    const kept = graphMeta() ? " The current graph is still open." : "";
    pushToast(`Couldn't create the graph.${kept} (${String(e)})`, "error");
    return { kind: "aborted" };
  }
  const openAction = { label: "Open graph", run: () => void openPickedGraphPath(root) };
  let loaded: LoadGraphPathOutcome;
  try {
    loaded = await loadGraphPath(root);
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    pushToast(`Created the graph at ${root}, but couldn't open it. (${detail})`, "error", {
      action: openAction,
    });
    return { kind: "aborted" };
  }
  if (loaded.kind === "aborted") {
    pushToast(`Created the graph at ${root}, but it wasn't opened. You can open this folder again.`, "info", {
      sticky: true, action: openAction,
    });
    return loaded;
  }
  if (loaded.kind === "focused_existing") {
    pushToast(`Created the graph at ${root} and focused its existing window.`, "success");
    return loaded;
  }
  if (loaded.kind === "already_current") {
    pushToast(`Created the graph at ${loaded.root}. It is already open.`, "success");
    return loaded;
  }
  pushToast(`Created and opened the graph at ${loaded.root}.`, "success");
  // Keep demo seeding scoped to the created root, as before; the message
  // describes the load outcome even when the backend returns a different path.
  if (loaded.root !== root) return loaded;
  const loadedOwner = bindingOwner();
  await seedTodayJournal();
  if (!loadedOwner()) return { kind: "aborted" };
  openPage("Welcome to Tine", "page"); // land on the tour, not the empty journal feed
  return loaded;
}

/** Give a freshly-created demo graph a friendly today's-journal entry so the
 *  Journals view isn't empty on first open. The caller awaits this best-effort seed. */
async function seedTodayJournal(): Promise<void> {
  const binding = captureBinding();
  const owner = bindingOwner();
  try {
    const title = journalTitle(appNow());
    const page = await readOwned(owner, backend().getPage(title, "journal"));
    if (page.kind === "stale") return;
    const existing = page.value;
    if (existing && journalHasContent(existing.blocks)) return;
    const resolution = existing?.id ? null : await readOwned(owner, backend().resolvePage(title, "journal"));
    if (resolution?.kind === "stale") return;
    const resolved = resolution?.value ?? null;
    if (resolved?.kind === "alias") throw new Error("conflict: journal alias");
    await createPage(title, demoJournalPage(title), {
      baseRev: null,
      bindingGeneration: binding.backendGeneration,
    });
  } catch {
    // Best-effort seed; the caller awaited this attempt.
  }
}
