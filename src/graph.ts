// Opening / switching the active graph from the UI (native folder picker),
// persisting the choice so it reopens next launch.

import { backend } from "./backend";
import { captureBinding, stillBound } from "./binding";
import { graphOwner, readOwned, writeOwned } from "./owned";
import { setGraphMeta, bumpGraphEpoch, graphMeta, graphEpoch } from "./graphSession";
import { setWorkflow, setRightSidebar, seedFavorites, pruneSidebarBlocks, refreshJournalConflicts, refreshSyncConflicts, clearRecent, graphTransitioning, setGraphTransitioning, renamePageInNavigation, resetLeftSidebarSections, closePdf, closePageProps, setAudioPlayer } from "./ui";
import { pushToast } from "./toasts";
import { resetStore, flushAll, createPage, journalTemplatePage, demoJournalPage, installRenameRefreshHandler, renamePageOnDisk } from "./document";
import { clearAssetBlobCache } from "./assetCache";
import { resetTabsToJournals, openPage, restoreSession, flushSession, type PageTarget } from "./router";
import { resetPaneLayoutToSingle, removePageTargetAcrossPanes } from "./panes";
import { journalTitle, localDayKey, setJournalTitleFormat } from "./journal";
import { applyTemplateVars, prepareTemplateVars } from "./editor/templateVars";
import { resetPageIndex } from "./pageIndex";
import { CUSTOM_CSS_STYLE_ID, ensureLsShimStyle } from "./lsShim";
import { ensureThemeStyle } from "./themeGallery";
import { isMobile, platformKind } from "./platform";
import type { BlockDto } from "./types";
import { maybeShowGuideAnnouncement } from "./guide";
import { endEdit } from "./editorController";
import { journalHasContent } from "./journalContent";
import { activatePdfOwnership, drainPdfWork, retirePdfOwnership } from "./pdfOwnership";
import { clearWorkspaces } from "./workspaces";

const GRAPH_KEY = "tine.graphPath";

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
  | { kind: "loaded" | "already_current"; root: string }
  | { kind: "focused_existing" | "aborted" };

/** Inspect a graph's external-assets target. Return true immediately when no
 * external target exists or this device has already approved it. Otherwise
 * show the resolved target and request consent, persisting approval on this
 * device. Denial or stale graph ownership returns false; inspection or
 * approval errors reject. Cost follows path inspection and one approval write. */
export async function authorizeGraphAccess(path: string): Promise<boolean> {
  const owner = graphOwner();
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
  path: string,
  options: { forceRefresh?: boolean; transitionHeld?: boolean } = {}
): Promise<LoadGraphPathOutcome> {
  const startingBinding = captureBinding();
  const ownsTransition = !options.transitionHeld;
  if (graphTransitioning() && ownsTransition) return { kind: "aborted" };
  if (ownsTransition) {
    setGraphTransitioning(true);
  }
  try {
  if (ownsTransition) {
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
    endEdit("graph-switch");
    // Let the textarea blur handler commit its final buffer before we inspect dirty.
    await Promise.resolve();
    if (!stillBound(startingBinding)) return { kind: "aborted" };
  }
  // Whether we're switching to a *different* graph than last time. Only then do
  // we drop the persisted right-sidebar items; reopening the same graph at
  // startup keeps them (and we prune stale block refs below).
  const prev = graphMeta()?.root || persistedGraphPath();
  const switching = !!prev && !!path && prev !== path;
  // Persist the current graph's pending edits BEFORE opening another graph —
  // otherwise the debounced save would either fire against the new graph or be
  // dropped by resetStore. No-op on first load (nothing dirty). If something
  // couldn't be saved (conflict / disk error), abort so resetStore doesn't
  // discard that edit — gated on whether a graph is actually loaded now, NOT on
  // the persisted path (which is empty on a TINE_GRAPH/CLI launch).
  const hadGraph = !!graphMeta();
  const rebindsPdfOwner = hadGraph && (switching || options.forceRefresh === true);
  const flushed = await flushAll();
  if (!stillBound(startingBinding)) return { kind: "aborted" };
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
  if (!stillBound(startingBinding)) return { kind: "aborted" };
  if (!(await authorizeGraphAccess(path))) return { kind: "aborted" };
  if (!stillBound(startingBinding)) return { kind: "aborted" };
  // This is the last await before the backend graph binding can change.  Flush
  // delayed view state plus complete highlight/area mutations under A; only a
  // successful drain permits us to invalidate that authority and unmount it.
  if (rebindsPdfOwner && !(await drainPdfWork())) {
    pushToast("PDF changes couldn't be saved — the current graph is still open.", "error");
    return { kind: "aborted" };
  }
  if (!stillBound(startingBinding)) return { kind: "aborted" };
  if (rebindsPdfOwner) {
    retirePdfOwnership();
    closePdf();
  }

  let result;
  try {
    result = await backend().loadGraph(path);
  } catch (error) {
    // load_graph failed before installing a replacement binding.  Publish a new
    // local generation for the still-bound old graph; the retired viewer stays
    // closed, so no callback can regain its former authority.
    if (rebindsPdfOwner && prev) activatePdfOwnership(prev);
    throw error;
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
  setGraphMeta(meta ?? null);
  // Revoke every in-flight result from the previous binding NOW, before the
  // awaited journal-template step. This is also required for same-root force
  // refresh (restore): root equality cannot distinguish pre-restore DTOs from
  // the freshly rebound graph. The second bump below refetches after a default
  // template has been written, preserving #73's populated-first observation.
  bumpGraphEpoch();
  setWorkflow(meta?.preferred_workflow === "todo" ? "todo" : "now");
  setJournalTitleFormat(meta?.journal_page_title_format); // match this graph's journal titles
  seedFavorites(meta?.favorites ?? []);
  void refreshJournalConflicts(true); // tell the user if any day has duplicate journal files
  void refreshSyncConflicts(true); // and flag any Syncthing/Dropbox conflict copies
  if (path) {
    try {
      localStorage.setItem(GRAPH_KEY, path);
    } catch {
      // ignore
    }
  }
  // A default journal template writes today's journal to disk. Do that before
  // invalidating graph-backed resources so the first Journals refetch observes
  // the populated file instead of caching the synthetic blank page (#73).
  await ensureJournalTemplateForDay(new Date());
  bumpGraphEpoch();
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
  return { kind: result.kind, root: meta.root };
  } finally {
    if (ownsTransition) setGraphTransitioning(false);
  }
}

/** Refresh frontend state after a successful page rename. The backend rename
 *  rewrites `[[refs]]` across many files through the self-write guard. The
 *  document intent has already flushed and reset its working set. Refresh the
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

export type RenameOutcome = "renamed" | "merged" | "cancelled" | "failed";

/** Rename a page; when `to` already names another page file, ask to merge into
 *  it as OG Logseq does (GH #327, `merge-pages!`): the backend appends the
 *  source's blocks, unites aliases, rewrites references and trashes the source
 *  in one transaction. `failed` means pending edits could not be flushed;
 *  backend errors reject. Cost: two name resolutions plus the rename. */
export async function renameOrMergePage(from: string, to: string, target?: PageTarget): Promise<RenameOutcome> {
  const owner = graphOwner();
  const found = await readOwned(owner, backend().resolvePage(to, "page"));
  if (found.kind === "stale") return "cancelled";
  let into: string | undefined;
  if (found.value.kind === "existing" && found.value.others.length === 0) {
    const source = target?.path ?? await readOwned(owner, backend().resolvePage(from, target?.pageKind ?? "page"))
      .then((own) => own.kind !== "stale" && own.value.kind === "existing" ? own.value.id : undefined);
    if (source && source !== found.value.id) into = found.value.id;
  }
  if (into) {
    const confirmed = await readOwned(owner, backend().confirm(`Page “${to}” already exists. Merge “${from}” into it?`));
    if (confirmed.kind === "stale" || !confirmed.value) return "cancelled";
  }
  if (!(await renamePageOnDisk(from, to, target, into))) return "failed";
  return into ? "merged" : "renamed";
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
        id: existing?.id ?? resolved!.id,
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

/** Load the graph's logseq/custom.css into a <style> tag (user theming). */
async function injectCustomCss(): Promise<void> {
  const owner = graphOwner();
  let css = "";
  try {
    const result = await readOwned(owner, backend().readCustomCss());
    if (result.kind === "stale") return;
    css = result.value;
  } catch {
    css = "";
  }
  if (!owner()) return;
  ensureLsShimStyle();
  ensureThemeStyle();
  let el = document.getElementById(CUSTOM_CSS_STYLE_ID);
  if (!el) {
    el = document.createElement("style");
    el.id = CUSTOM_CSS_STYLE_ID;
  }
  el.textContent = css;
  document.head.appendChild(el);
}

/** Open a graph chosen with the desktop folder picker or Android graph picker.
 * Android may request all-files access and return aborted until granted. iOS
 * currently shows an unsupported-action toast and returns aborted. Cancellation,
 * permission refusal and stale ownership return aborted; a successful pick
 * delegates to loadGraphPath, which flushes the old graph before switching.
 * Desktop picker errors reject; graph-load cost follows graph files. */
export async function switchGraph(): Promise<LoadGraphPathOutcome> {
  const owner = graphOwner();
  const platform = await platformKind();
  if (!owner()) return { kind: "aborted" };
  if (platform === "android") {
    let result;
    try {
      const picked = await readOwned(owner, backend().pickGraphFolder());
      if (picked.kind === "stale") return { kind: "aborted" };
      result = picked.value;
    } catch (e) {
      pushToast(`Couldn't open the Android folder picker. (${String(e)})`, "error");
      return { kind: "aborted" };
    }
    // Diagnostic breadcrumbs (visible in `adb logcat`, chromium console channel):
    // an intermittent first-run stall on "Opening…" — these pin down whether the
    // native picker returned and whether the graph parse completed or hung.
    console.info("[tine/android] pickGraphFolder completed");
    if (result.status === "picked") {
      if (result.path) {
        console.info("[tine/android] loadGraphPath: start");
        const outcome = await loadGraphPath(result.path);
        console.info("[tine/android] loadGraphPath: done");
        return outcome;
      }
      return { kind: "aborted" };
    }
    if (result.status === "permission-requested" || result.status === "permission-needed") {
      pushToast('Grant "All files access" for Tine, then tap Open again.', "info");
    }
    return { kind: "aborted" };
  }
  if (platform === "ios") {
    pushToast(
      "Opening an existing graph on iOS is coming soon. For now, tap “Create a new graph” to try Tine.",
      "info"
    );
    return { kind: "aborted" };
  }
  const picked = await readOwned(owner, backend().pickFolder());
  if (picked.kind === "stale") return { kind: "aborted" };
  return picked.value ? loadGraphPath(picked.value) : { kind: "aborted" };
}

/** Create a demo graph under a desktop-picked folder or the mobile default
 * graph parent, then open it and navigate to Welcome to Tine. Cancellation
 * returns aborted. A created directory can remain if opening fails; today's
 * narrated journal seed is best effort and its failure does not change a
 * loaded outcome. Creation errors toast and return aborted; picker/parent errors
 * reject. Cost follows graph creation, templates and the graph load. */
export async function createNewGraph(): Promise<LoadGraphPathOutcome> {
  const owner = graphOwner();
  const dirResult = (await isMobile())
    ? await readOwned(owner, backend().defaultGraphParent())
    : await readOwned(owner, backend().pickFolder("Choose where to create your new graph"));
  if (dirResult.kind === "stale") return { kind: "aborted" };
  const dir = dirResult.value;
  if (!dir) return { kind: "aborted" };
  let root: string;
  try {
    const created = await writeOwned(owner, backend().createGraph(dir));
    if (created.kind === "stale") return { kind: "aborted" };
    root = created.value;
  } catch (e) {
    pushToast(`Couldn't create the graph. (${String(e)})`, "error");
    return { kind: "aborted" };
  }
  const loaded = await loadGraphPath(root);
  if (loaded.kind !== "loaded" || loaded.root !== root) {
    pushToast(`Created the graph at ${root}, but kept the current graph open.`, "info");
    return loaded;
  }
  const loadedOwner = graphOwner();
  await seedTodayJournal();
  if (!loadedOwner()) return { kind: "aborted" };
  openPage("Welcome to Tine", "page"); // land on the tour, not the empty journal feed
  return loaded;
}

/** Give a freshly-created demo graph a friendly today's-journal entry so the
 *  Journals view isn't empty on first open. The caller awaits this best-effort seed. */
async function seedTodayJournal(): Promise<void> {
  const binding = captureBinding();
  const owner = graphOwner();
  try {
    const title = journalTitle(new Date());
    const page = await readOwned(owner, backend().getPage(title, "journal"));
    if (page.kind === "stale") return;
    const existing = page.value;
    if (existing && existing.blocks.some((b) => b.raw.trim() !== "")) return;
    const resolution = existing?.id ? null : await readOwned(owner, backend().resolvePage(title, "journal"));
    if (resolution?.kind === "stale") return;
    const resolved = resolution?.value ?? null;
    if (resolved?.kind === "alias") throw new Error("conflict: journal alias");
    await createPage(title, demoJournalPage(title), {
      id: existing?.id ?? resolved!.id,
      baseRev: null,
      bindingGeneration: binding.backendGeneration,
    });
  } catch {
    // Best-effort seed; the caller awaited this attempt.
  }
}
