// Opening / switching the active graph from the UI (native folder picker),
// persisting the choice so it reopens next launch.

import { backend } from "./backend";
import { managedStorageRuntime } from "./managedStorageRuntime";
import { favorites, setGraphMeta, setWorkflow, bumpGraphEpoch, setRightSidebar, graphMeta, graphEpoch, setAliasMap, seedFavorites, pruneSidebarBlocks, pushToast, refreshJournalConflicts, refreshSyncConflicts, restoreLiveSaveConflicts, clearRecent, graphTransitioning, setGraphTransitioning, renamePageInNavigation, resetLeftSidebarSections, pageIdentityKey } from "./ui";
import { loadFavoritesLayout } from "./favoritesStore";
import { resetStore, flushAll } from "./store";
import { clearAssetBlobCache } from "./assetCache";
import { resetTabsToJournals, openPage, restoreSession, flushSession, route, sameRoute, type PageTarget } from "./router";
import { resetPaneLayoutToSingle, removePageTargetAcrossPanes } from "./panes";
import { journalTitle, localDayKey, setJournalTitleFormat } from "./journal";
import { applyTemplateVars, prepareTemplateVars } from "./editor/templateVars";
import { waitForWarmCache } from "./warmCache";
import { CUSTOM_CSS_STYLE_ID, ensureLsShimStyle } from "./lsShim";
import { ensureThemeStyle } from "./themeGallery";
import { isMobile, platformKind } from "./platform";
import type { BlockDto, GraphMeta } from "./types";
import { maybeShowGuideAnnouncement } from "./guide";
import { endEdit } from "./editorController";
import { activatePdfOwnership, drainPdfWork, retirePdfOwnership } from "./pdfOwnership";
import { openConfiguredHomePage } from "./homePage";
import { safeManagedErrorDetail } from "./managedDiagnostics";
import { beginGraphOpenTrace, markGraphOpen } from "./graphOpenTrace";

const GRAPH_KEY = "tine.graphPath";
export const PARTIAL_PROVIDER_REFUSAL =
  "Tine-managed storage sync data appears to still be arriving or is incomplete. Tine left this graph unchanged. Let your file-sync provider finish, then Retry.";

/** Keep a graph-open refusal visible until the user can retry the exact same
 * target. A picker has already returned its target at this point, so reopening
 * the picker would be a lossy and surprising substitute for Retry. */
export function reportGraphOpenFailure(error: unknown, retry: () => void): void {
  const detail = safeManagedErrorDetail(error);
  const message = detail === PARTIAL_PROVIDER_REFUSAL
    ? PARTIAL_PROVIDER_REFUSAL
    : `Couldn't open the graph. (${detail})`;
  pushToast(message, "error", {
    sticky: true,
    action: { label: "Retry", run: retry },
  });
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
  | { kind: "loaded" | "already_current"; root: string }
  | { kind: "focused_existing" | "aborted" };

// Frontend continuations may overlap while native storage recovery is being
// superseded (most importantly, emergency Direct Files during a managed open).
// This is not storage authority: it only prevents an obsolete promise from
// repainting/resetting the UI after a newer native operation has won.
let graphLoadContinuation = 0;

/** Establish the one exceptional filesystem capability Tine supports: a graph
 * may point `assets` at an external directory, but only after this installation
 * shows the resolved target and receives explicit consent. */
export async function authorizeGraphAccess(path: string): Promise<boolean> {
  const access = await backend().inspectGraphAccess(path);
  const external = access.external_assets_path;
  if (!external || access.approved) return true;
  const approved = await backend().confirm(
    `This graph's assets folder points outside the graph to:\n\n${external}\n\nAllow Tine to read and write assets in this directory? This approval is stored only on this device.`,
    "Allow external assets directory?"
  );
  if (!approved) {
    pushToast(
      `Graph not opened: its external assets directory was not approved (${external}).`,
      "error",
      { sticky: true }
    );
    return false;
  }
  await backend().approveExternalAssets(access.graph_root, external);
  return true;
}

export async function loadGraphPath(
  path: string,
  options: {
    forceRefresh?: boolean;
    transitionHeld?: boolean;
    supersedeCurrent?: boolean;
  } = {}
): Promise<LoadGraphPathOutcome> {
  beginGraphOpenTrace();
  const startedAt = performance.now();
  let graphPath = path;
  const ownsTransition = !options.transitionHeld;
  if (graphTransitioning() && ownsTransition && !options.supersedeCurrent) {
    return { kind: "aborted" };
  }
  const continuation = ++graphLoadContinuation;
  if (ownsTransition) {
    setGraphTransitioning(true);
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
    endEdit("graph-switch");
    // Let the textarea blur handler commit its final buffer before we inspect dirty.
    await Promise.resolve();
  }
  try {
  console.info(`[tine] frontend graph open: begin path=${graphPath ? "explicit" : "default"}`);
  // Persist the current graph's pending edits BEFORE opening another graph —
  // otherwise the debounced save would either fire against the new graph or be
  // dropped by resetStore. No-op on first load (nothing dirty). If something
  // couldn't be saved (conflict / disk error), abort so resetStore doesn't
  // discard that edit — gated on whether a graph is actually loaded now, NOT on
  // the persisted path (which is empty on a TINE_GRAPH/CLI launch).
  const hadGraph = !!graphMeta();
  const flushed = await flushAll();
  if (hadGraph && !flushed) {
    pushToast("Some pages couldn't be saved — resolve conflicts before switching graphs.", "error");
    return { kind: "aborted" };
  }
  if (hadGraph) await flushSession();
  if (graphPath && (await platformKind()) === "ios") {
    try {
      const prepared = await backend().prepareGraphFolder(graphPath);
      if (prepared.status !== "ready") {
        pushToast(
          "TineOutline can only open folders inside On My iPhone or iCloud Drive → TineOutline.",
          "error",
          { sticky: true }
        );
        return { kind: "aborted" };
      }
      // iOS may move an app's data container during an update. The native
      // boundary rebases a remembered Documents-relative graph onto the
      // current container and returns the path that Rust must actually open.
      graphPath = prepared.path ?? graphPath;
    } catch (error) {
      pushToast(`Couldn't prepare the iCloud graph. (${String(error)})`, "error", { sticky: true });
      return { kind: "aborted" };
    }
  }
  // Whether we're switching to a *different* graph than last time. Compute
  // this after iOS path rebasing so a container relocation is not mistaken for
  // a distinct workspace.
  const prev = graphMeta()?.root || persistedGraphPath();
  const switching = !!prev && !!graphPath && prev !== graphPath;
  const rebindsPdfOwner = hadGraph && (switching || options.forceRefresh === true);
  if (!(await authorizeGraphAccess(graphPath))) return { kind: "aborted" };
  // This is the last await before the backend graph binding can change.  Flush
  // delayed view state plus complete highlight/area mutations under A; only a
  // successful drain permits us to invalidate that authority and unmount it.
  if (rebindsPdfOwner && !(await drainPdfWork())) {
    pushToast("PDF changes couldn't be saved — the current graph is still open.", "error");
    return { kind: "aborted" };
  }
  if (rebindsPdfOwner) {
    retirePdfOwnership();
  }
  if (continuation !== graphLoadContinuation) return { kind: "aborted" };

  let result;
  // Native graph replacement is asynchronous. Stop accepting watcher events
  // from the retired binding before its completion can publish a new generation;
  // otherwise an old runtime error can race the successful switch response.
  const clearedManagedRuntime = hadGraph && (switching || options.forceRefresh === true);
  if (clearedManagedRuntime) managedStorageRuntime.clear();
  try {
    result = await backend().loadGraph(graphPath);
  } catch (error) {
    if (continuation !== graphLoadContinuation) return { kind: "aborted" };
    // load_graph failed before installing a replacement binding.  Publish a new
    // local generation for the still-bound old graph; the retired viewer stays
    // closed, so no callback can regain its former authority.
    if (rebindsPdfOwner && prev) {
      activatePdfOwnership(prev);
    }
    if (clearedManagedRuntime) void managedStorageRuntime.refresh();
    throw error;
  }
  if (continuation !== graphLoadContinuation) return { kind: "aborted" };
  console.info(`[tine] frontend graph open: native binding ready at ${Math.round(performance.now() - startedAt)} ms`);
  markGraphOpen("native_binding_ready");
  if (result.kind === "focused_existing") {
    if (rebindsPdfOwner && prev) {
      activatePdfOwnership(prev);
    }
    if (clearedManagedRuntime) void managedStorageRuntime.refresh();
    return { kind: "focused_existing" };
  }
  managedStorageRuntime.bind(result.binding_generation, result.application_page_admission);
  const meta = result.meta;
  if (result.kind === "already_current" && hadGraph && !options.forceRefresh) {
    return { kind: "already_current", root: meta.root };
  }
  if (!hadGraph || rebindsPdfOwner) {
    activatePdfOwnership(meta.root);
  }
  resetStore();
  resetNavigationIndex();
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
  restoreLiveSaveConflicts(meta.root);
  // Revoke every in-flight result from the previous binding NOW. This is also
  // required for same-root force refresh (restore): root equality cannot
  // distinguish pre-restore DTOs from the freshly rebound graph. A visible
  // Journals surface performs template materialization before fetching its feed,
  // preserving #73's populated-first observation without blocking graph open.
  bumpGraphEpoch();
  applyConfigDerivedState(meta, null);
  // Standing conflicts — duplicate journal days included, as of the day they
  // became queue objects — surface through the calm badge + in-page resolver,
  // not a startup toast; these just derive the inventories.
  void refreshJournalConflicts();
  void refreshSyncConflicts();
  if (graphPath) {
    try {
      localStorage.setItem(GRAPH_KEY, graphPath);
    } catch {
      // ignore
    }
  }
  // A visible Journals surface owns template materialization and awaits it
  // before fetching the feed (Page.tsx). Doing it here as well makes every graph
  // open pay getPage/listTemplates, which can cold-scan a large Direct graph.
  void injectCustomCss();
  void loadAliases();
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
  console.info(`[tine] frontend graph open: session restored at ${Math.round(performance.now() - startedAt)} ms`);
  markGraphOpen("session_restored");
  // GH #245: a configured home page wins over the ordinary landing on an
  // ordinary open (first bind or graph switch) — not on a same-graph reload /
  // watcher refresh. Later explicit intents (quick capture, deep link) still
  // win by navigating after this.
  if (result.kind === "loaded" && (switching || !hadGraph)) {
    const homeEpoch = graphEpoch();
    const landingRoute = { ...route() };
    void openConfiguredHomePage(meta.root, () =>
      graphMeta()?.root === meta.root
      && graphEpoch() === homeEpoch
      && sameRoute(route(), landingRoute)
    );
  }
  console.info(`[tine] frontend graph open: interactive at ${Math.round(performance.now() - startedAt)} ms`);
  markGraphOpen("interactive");
  return { kind: result.kind, root: meta.root };
  } finally {
    if (ownsTransition && continuation === graphLoadContinuation) {
      setGraphTransitioning(false);
    }
  }
}

let navigationEpoch = -1;
let aliasEntries: Record<string, string> = {};
let pageIdentities: Record<string, string> = {};
let aliasRequest = 0;
let pageIdentityRequest = 0;

function resetNavigationIndex(): void {
  navigationEpoch = -1;
  aliasEntries = {};
  pageIdentities = {};
  aliasRequest++;
  pageIdentityRequest++;
  setAliasMap({});
}

function bindNavigationIndex(epoch: number): void {
  if (navigationEpoch === epoch) return;
  navigationEpoch = epoch;
  aliasEntries = {};
  pageIdentities = {};
  aliasRequest++;
  pageIdentityRequest++;
  setAliasMap({});
}

function commitNavigationIndex(): void {
  // Existing files win a colliding alias, matching core `load_named`.
  setAliasMap({ ...aliasEntries, ...pageIdentities });
}

/** Refresh semantic aliases after content saves. Request sequencing prevents an
 *  older same-epoch response from overwriting a newer alias edit. */
export async function refreshAliases(): Promise<void> {
  const epoch = graphEpoch();
  bindNavigationIndex(epoch);
  const request = ++aliasRequest;
  const result = await Promise.allSettled([backend().pageAliases()]);
  if (epoch !== graphEpoch() || navigationEpoch !== epoch || request !== aliasRequest) return;
  aliasEntries = {};
  if (result[0].status === "fulfilled") {
    for (const [alias, owner] of result[0].value) {
      const key = pageIdentityKey(alias);
      // Core returns owners in deterministic path order; preserve its first-wins
      // fallback when duplicate owners contribute the same folded alias.
      if (!Object.prototype.hasOwnProperty.call(aliasEntries, key)) {
        aliasEntries[key] = owner;
      }
    }
  }
  commitNavigationIndex();
}

/** Refresh the real-page identity inventory only after graph bind, create,
 *  delete, or rename. Ordinary content saves refresh aliases but never pay for
 *  this whole-page-list IPC. Real pages override colliding semantic aliases. */
export async function refreshPageIdentities(): Promise<void> {
  const epoch = graphEpoch();
  bindNavigationIndex(epoch);
  const request = ++pageIdentityRequest;
  const result = await Promise.allSettled([backend().listPages()]);
  if (epoch !== graphEpoch() || navigationEpoch !== epoch || request !== pageIdentityRequest) return;
  pageIdentities = result[0].status === "fulfilled"
    ? Object.fromEntries(
        result[0].value
          .filter((entry) => entry.kind === "page")
          .map((entry) => [pageIdentityKey(entry.name), entry.name])
      )
    : {};
  commitNavigationIndex();
}

/** React to native code atomically rebinding the same graph root to a new
 * storage-authority generation. This retires renderer objects from the former
 * generation, but never reopens the graph or independently decides whether the
 * native transition succeeded. */
export function rebindCurrentStorageAuthority(): void {
  if (!graphMeta()) throw new Error("the current graph identity disappeared during the storage transition");
  resetStore();
  resetNavigationIndex();
  clearAssetBlobCache();
  bumpGraphEpoch();
  void loadAliases();
  void pruneSidebarBlocks();
}

async function loadAliases(): Promise<void> {
  const epoch = graphEpoch();
  if (!(await waitForWarmCache(epoch))) return;
  if (epoch !== graphEpoch()) return;
  await Promise.all([refreshAliases(), refreshPageIdentities()]);
}

/** Refresh frontend state after a successful page rename. The backend rename
 *  rewrites `[[refs]]` across many files through the self-write guard, which
 *  SUPPRESSES the watcher reload — so every in-memory page (the renamed page, the
 *  journals feed, satellite/sidebar pages) is potentially stale, and a stale save
 *  of one would silently revert the rename's rewrite on disk. Reset the store
 *  (cancels pending/in-flight saves + clears the shared `byId`) and bump the graph
 *  epoch (drops the block-resolve cache and forces the open view + Linked
 *  References to refetch from the now-correct backend). Aliases may have moved with
 *  the renamed file, so refresh those too. Caller must have run flushAll() first
 *  (so resetStore discards nothing unsaved) and then navigate to the new name. */
export function refreshAfterRename(from: string, to: string, exactTarget?: PageTarget): void {
  if (exactTarget) {
    removePageTargetAcrossPanes(exactTarget);
    renamePageInNavigation(exactTarget, { name: to, pageKind: exactTarget.pageKind });
  } else {
    renamePageInNavigation(from, to);
  }
  resetStore();
  resetNavigationIndex();
  bumpGraphEpoch();
  void Promise.all([refreshAliases(), refreshPageIdentities()]);
}

export async function renameOrMergePage(
  from: string,
  to: string,
  sourcePath?: string,
): Promise<"renamed" | "merged" | "cancelled"> {
  const destination = await backend().getPage(to, "page");
  let exactSourcePath = sourcePath;
  if (!exactSourcePath) exactSourcePath = (await backend().getPage(from, "page"))?.path;
  if (destination?.path && destination.path !== exactSourcePath) {
    if (!globalThis.confirm(`Page “${to}” already exists. Merge “${from}” into it?`)) {
      return "cancelled";
    }
    if (!exactSourcePath) {
      throw new Error(`Couldn't identify the source file for “${from}”.`);
    }
    await backend().mergePages(exactSourcePath, destination.path, { from, to });
    return "merged";
  }
  const outcome = await backend().renamePage(from, to, exactSourcePath);
  const skipped = outcome?.skippedConflictedReferrers ?? [];
  if (skipped.length) {
    // These files are mid-merge, so the rename deliberately left their refs
    // pointing at the old name. Saying nothing would make a partial rename
    // look complete.
    pushToast(
      skipped.length === 1
        ? `1 page with unresolved merge conflicts still refers to “${from}”. Resolve its merge, then fix the link.`
        : `${skipped.length} pages with unresolved merge conflicts still refer to “${from}”. Resolve their merges, then fix the links.`,
      "warn",
      { sticky: true },
    );
  }
  return "renamed";
}

export type JournalTemplateEnsureResult = "ready" | "deferred" | "stale";

interface JournalTemplateOwner {
  root: string;
  epoch: number;
  day: number;
  title: string;
  template: string;
}

let journalTemplateFlight: {
  owner: JournalTemplateOwner;
  promise: Promise<JournalTemplateEnsureResult>;
} | null = null;

function sameJournalTemplateOwner(a: JournalTemplateOwner, b: JournalTemplateOwner): boolean {
  return a.root === b.root && a.epoch === b.epoch && a.day === b.day && a.template === b.template;
}

function journalTemplateOwnerIsCurrent(owner: JournalTemplateOwner): boolean {
  const meta = graphMeta();
  return !!meta
    && meta.root === owner.root
    && meta.default_journal_template === owner.template
    && graphEpoch() === owner.epoch
    && localDayKey() === owner.day;
}

async function materializeJournalTemplate(
  owner: JournalTemplateOwner,
  canWrite: () => boolean,
): Promise<JournalTemplateEnsureResult> {
  try {
    const existing = await backend().getPage(owner.title, "journal");
    if (!journalTemplateOwnerIsCurrent(owner)) return "stale";
    if (existing && existing.blocks.some((b) => b.raw.trim() !== "")) return "ready";
    const tmpl = (await backend().listTemplates()).find((t) => t.name === owner.template);
    if (!journalTemplateOwnerIsCurrent(owner)) return "stale";
    if (!tmpl) return "ready";
    await prepareTemplateVars();
    if (!journalTemplateOwnerIsCurrent(owner)) return "stale";
    if (!canWrite()) return "deferred";
    const resolve = (b: BlockDto): BlockDto => ({
      id: "",
      raw: applyTemplateVars(b.raw, owner.title),
      collapsed: false,
      children: b.children.map(resolve),
    });
    await backend().savePage(
      {
        name: owner.title,
        kind: "journal",
        title: owner.title,
        pre_block: existing?.pre_block ?? null,
        blocks: tmpl.blocks.map(resolve),
        // An empty journal may already exist on disk. Preserve its concrete file
        // and format rather than re-resolving it as a new canonical markdown page.
        path: existing?.path,
        format: existing?.format,
      },
      existing?.rev ?? null,
      false
    );
    return journalTemplateOwnerIsCurrent(owner) ? "ready" : "stale";
  } catch {
    return journalTemplateOwnerIsCurrent(owner) ? "deferred" : "stale";
  }
}

/** If config.edn sets :default-templates {:journals "X"}, create the supplied
 * local day's journal from that template when missing or empty. Concurrent
 * timer/focus/load callers share one graph-generation/day flight. Every await
 * revalidates graph, template identity and local day before another IPC begins.
 * Unconfigured graphs remain lazy and never issue a page write. */
export async function ensureJournalTemplateForDay(
  date: Date,
  canWrite: () => boolean = () => true,
): Promise<JournalTemplateEnsureResult> {
  const meta = graphMeta();
  const template = meta?.default_journal_template;
  if (!meta || !template) return "ready";
  const owner: JournalTemplateOwner = {
    root: meta.root,
    epoch: graphEpoch(),
    day: localDayKey(date),
    title: journalTitle(date),
    template,
  };
  if (!journalTemplateOwnerIsCurrent(owner) || !canWrite()) return "deferred";
  if (journalTemplateFlight && sameJournalTemplateOwner(journalTemplateFlight.owner, owner)) {
    return journalTemplateFlight.promise;
  }
  const promise = materializeJournalTemplate(owner, canWrite);
  const flight = { owner, promise };
  journalTemplateFlight = flight;
  try {
    return await promise;
  } finally {
    if (journalTemplateFlight === flight) journalTemplateFlight = null;
  }
}

/** Load the graph's logseq/custom.css into a <style> tag (user theming). */
async function injectCustomCss(): Promise<void> {
  let css = "";
  try {
    css = await backend().readCustomCss();
  } catch {
    css = "";
  }
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

/** Pick a folder and open it as the graph. No-op if cancelled. */
export async function switchGraph(): Promise<LoadGraphPathOutcome> {
  const platform = await platformKind();
  if (platform === "android" || platform === "ios") {
    let result;
    try {
      result = await backend().pickGraphFolder();
    } catch (e) {
      const platformName = platform === "android" ? "Android" : "iOS";
      pushToast(`Couldn't open the ${platformName} folder picker. (${String(e)})`, "error");
      return { kind: "aborted" };
    }
    // Diagnostic breadcrumbs (visible in `adb logcat`, chromium console channel):
    // an intermittent first-run stall on "Opening…" — these pin down whether the
    // native picker returned and whether the graph parse completed or hung.
    console.info(`[tine/${platform}] pickGraphFolder → ${result.status}`);
    if (result.status === "picked") {
      if (result.path) {
        console.info(`[tine/${platform}] loadGraphPath: start`);
        const outcome = await openPickedGraphPath(result.path);
        console.info(`[tine/${platform}] loadGraphPath: done`);
        return outcome;
      }
      return { kind: "aborted" };
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
  const path = await backend().pickFolder();
  return path ? openPickedGraphPath(path) : { kind: "aborted" };
}

async function openPickedGraphPath(path: string): Promise<LoadGraphPathOutcome> {
  try {
    return await loadGraphPath(path);
  } catch (error) {
    reportGraphOpenFailure(error, () => void openPickedGraphPath(path));
    return { kind: "aborted" };
  }
}

/** Onboarding "create a new graph": pick where to put it, scaffold a small
 *  narrated demo graph there, open it, and land on the "Welcome to Tine" tour.
 *  No-op if the folder picker is cancelled. */
export async function createNewGraph(): Promise<LoadGraphPathOutcome> {
  const platform = await platformKind();
  let dir: string | null;
  if (platform === "ios") {
    const result = await backend().pickGraphFolder();
    if (result.status === "refused") {
      pushToast(
        "Choose a folder inside On My iPhone or iCloud Drive → TineOutline. Other Files providers aren't supported yet.",
        "info"
      );
      return { kind: "aborted" };
    }
    dir = result.status === "picked" ? result.path : null;
  } else {
    dir = (await isMobile())
      ? await backend().defaultGraphParent()
      : await backend().pickFolder("Choose where to create your new graph");
  }
  if (!dir) return { kind: "aborted" };
  if (platform === "ios") {
    try {
      const prepared = await backend().prepareGraphFolder(dir);
      if (prepared.status !== "ready") {
        pushToast(
          "TineOutline can only create graphs inside On My iPhone or iCloud Drive → TineOutline.",
          "error",
          { sticky: true }
        );
        return { kind: "aborted" };
      }
      dir = prepared.path ?? dir;
    } catch (error) {
      pushToast(`Couldn't prepare the iCloud location. (${String(error)})`, "error", { sticky: true });
      return { kind: "aborted" };
    }
  }
  let root: string;
  try {
    root = await backend().createGraph(dir);
  } catch (e) {
    pushToast(`Couldn't create the graph. (${String(e)})`, "error");
    return { kind: "aborted" };
  }
  const loaded = await loadGraphPath(root);
  if (loaded.kind !== "loaded" || loaded.root !== root) {
    pushToast(`Created the graph at ${root}, but kept the current graph open.`, "info");
    return loaded;
  }
  await seedTodayJournal();
  openPage("Welcome to Tine", "page"); // land on the tour, not the empty journal feed
  return loaded;
}

/** Give a freshly-created demo graph a friendly today's-journal entry so the
 *  Journals view isn't empty on first open. Best-effort; never blocks. */
async function seedTodayJournal(): Promise<void> {
  try {
    const title = journalTitle(new Date());
    const existing = await backend().getPage(title, "journal");
    if (existing && existing.blocks.some((b) => b.raw.trim() !== "")) return;
    await backend().savePage(
      {
        name: title,
        kind: "journal",
        title,
        pre_block: null,
        blocks: [
          {
            id: "",
            raw: "👋 This is **today's journal** — your daily notes land here. Try your quick-capture hotkey, or open [[Welcome to Tine]] for the tour.",
            collapsed: false,
            children: [],
          },
        ],
      },
      null,
      false
    );
  } catch {
    // best-effort — never block opening the new graph on the seed
  }
}

/** Re-apply the frontend state that `config.edn` owns.
 *
 *  `previous` is the meta this state was last built from, or `null` to apply
 *  everything. A graph open passes `null`; a live config change passes what the
 *  frontend already had, so an unrelated settings write does not re-seed
 *  favorites and re-fetch the arrangement page for nothing.
 *
 *  Only the settings that are NOT read reactively from `graphMeta()` need to be
 *  here. Everything else — shortcuts, macros, hidden properties, show-brackets,
 *  the logbook flags — already updates for free when the meta signal changes,
 *  and adding it here would be a second producer of the same state. */
export function applyConfigDerivedState(meta: GraphMeta, previous: GraphMeta | null): void {
  const moved = (pick: (m: GraphMeta) => unknown) =>
    previous === null || JSON.stringify(pick(previous)) !== JSON.stringify(pick(meta));
  if (moved((m) => m.preferred_workflow)) {
    setWorkflow(meta?.preferred_workflow === "todo" ? "todo" : "now");
  }
  if (moved((m) => m.journal_page_title_format)) {
    // Match this graph's journal titles.
    setJournalTitleFormat(meta?.journal_page_title_format);
  }
  // Comparing the FILE is not enough for favorites. A managed graph has no
  // retained `Graph` for the watcher to ask, so it re-reads configuration after
  // every settings write including Tine's own — and re-seeding would drop the
  // arrangement and re-fetch its page on every star toggled in the sidebar.
  // Compare what the user is actually being shown instead.
  const incoming = meta?.favorites ?? [];
  const shown = favorites().map((f) => f.name);
  const alreadyShowing =
    previous !== null &&
    previous.favorites_page === meta?.favorites_page &&
    shown.length === incoming.length &&
    shown.every((name, i) => name === incoming[i]);
  if (moved((m) => [m.favorites, m.favorites_page]) && !alreadyShowing) {
    seedFavorites(incoming);
    // The arrangement (nesting and order) lives in a page named by
    // `:tine/favorites-page`; config.edn stays the authority on WHICH pages are
    // favorited, so a change made in Logseq is honoured whether Tine was
    // running at the time or not.
    void loadFavoritesLayout(incoming, meta?.favorites_page ?? null);
  }
}

/** `logseq/config.edn` changed on disk and the backend re-read it.
 *
 *  Before this existed, Tine read the file once per graph open, so an edit made
 *  in Logseq — or delivered by Syncthing — was invisible for the rest of the
 *  session, and the next settings write in Tine was based on the stale copy. */
export function applyGraphConfigChange(meta: GraphMeta): void {
  const previous = graphMeta();
  if (previous && previous.root !== meta.root) return; // a different graph's window
  setGraphMeta(meta);
  // A journal-title or format change re-dates and re-routes what is already on
  // screen, so in-flight results from before it must not land afterwards.
  if (
    previous &&
    (previous.journal_page_title_format !== meta.journal_page_title_format ||
      previous.preferred_format !== meta.preferred_format)
  ) {
    bumpGraphEpoch();
  }
  applyConfigDerivedState(meta, previous);
}
