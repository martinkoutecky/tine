// Opening / switching the active graph from the UI (native folder picker),
// persisting the choice so it reopens next launch.

import { backend } from "./backend";
import { setGraphMeta, setWorkflow, bumpGraphEpoch, setRightSidebar, graphMeta, graphEpoch, setAliasMap, seedFavorites, pruneSidebarBlocks, pushToast, refreshJournalConflicts, refreshSyncConflicts, clearRecent, graphTransitioning, setGraphTransitioning } from "./ui";
import { resetStore, flushAll } from "./store";
import { clearAssetBlobCache } from "./assetCache";
import { resetTabsToJournals, openPage, restoreSession, flushSession } from "./router";
import { resetPaneLayoutToSingle } from "./panes";
import { journalTitle, setJournalTitleFormat } from "./journal";
import { applyTemplateVars } from "./editor/templateVars";
import { waitForWarmCache } from "./warmCache";
import { CUSTOM_CSS_STYLE_ID, ensureLsShimStyle } from "./lsShim";
import { ensureThemeStyle } from "./themeGallery";
import { isMobile, platformKind } from "./platform";
import type { BlockDto } from "./types";
import { maybeShowGuideAnnouncement } from "./guide";
import { endEdit } from "./editorController";

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

export async function loadGraphPath(
  path: string,
  options: { forceRefresh?: boolean; transitionHeld?: boolean } = {}
): Promise<LoadGraphPathOutcome> {
  const ownsTransition = !options.transitionHeld;
  if (graphTransitioning() && ownsTransition) return { kind: "aborted" };
  if (ownsTransition) {
    setGraphTransitioning(true);
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
    endEdit("graph-switch");
    // Let the textarea blur handler commit its final buffer before we inspect dirty.
    await Promise.resolve();
  }
  try {
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
  const flushed = await flushAll();
  if (hadGraph && !flushed) {
    pushToast("Some pages couldn't be saved — resolve conflicts before switching graphs.", "error");
    return { kind: "aborted" };
  }
  if (hadGraph) await flushSession();
  const result = await backend().loadGraph(path);
  if (result.kind === "focused_existing") return { kind: "focused_existing" };
  const meta = result.meta;
  if (result.kind === "already_current" && hadGraph && !options.forceRefresh) {
    return { kind: "already_current", root: meta.root };
  }
  resetStore();
  clearAssetBlobCache(); // old graph's image blob URLs must not leak into the new one
  if (switching) {
    // A graph switch is a full workspace reset (OG opens one graph at a time):
    // drop the old graph's right-sidebar items and its recent-pages list so they
    // don't linger in the sidebar / quick-switch. Tabs are reset further below.
    setRightSidebar([]);
    clearRecent();
  }
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
  await ensureJournalTemplate();
  bumpGraphEpoch();
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
  return { kind: result.kind, root: meta.root };
  } finally {
    if (ownsTransition) setGraphTransitioning(false);
  }
}

/** Load the graph's alias:: index so link/navigation can resolve aliases.
 *  Guarded by the graph epoch so a slow response after a graph switch can't
 *  install the old graph's aliases. Exposed as `refreshAliases` so it can be
 *  re-run after edits (an alias:: change would otherwise leave nav stale). */
export async function refreshAliases(): Promise<void> {
  const epoch = graphEpoch();
  try {
    const pairs = await backend().pageAliases();
    if (epoch !== graphEpoch()) return;
    setAliasMap(Object.fromEntries(pairs));
  } catch {
    if (epoch === graphEpoch()) setAliasMap({});
  }
}
async function loadAliases(): Promise<void> {
  const epoch = graphEpoch();
  if (!(await waitForWarmCache(epoch))) return;
  if (epoch !== graphEpoch()) return;
  await refreshAliases();
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
export function refreshAfterRename(): void {
  resetStore();
  bumpGraphEpoch();
  void refreshAliases();
}

// If config.edn sets :default-templates {:journals "X"}, create today's journal
// from that template when it doesn't exist yet (or is empty). No-op when unset,
// so default behaviour is unchanged.
async function ensureJournalTemplate(): Promise<void> {
  const tname = graphMeta()?.default_journal_template;
  if (!tname) return;
  const title = journalTitle(new Date());
  try {
    const existing = await backend().getPage(title, "journal");
    if (existing && existing.blocks.some((b) => b.raw.trim() !== "")) return; // already has content
    const tmpl = (await backend().listTemplates()).find((t) => t.name === tname);
    if (!tmpl) return;
    const resolve = (b: BlockDto): BlockDto => ({
      id: "",
      raw: applyTemplateVars(b.raw, title),
      collapsed: false,
      children: b.children.map(resolve),
    });
    await backend().savePage(
      {
        name: title,
        kind: "journal",
        title,
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
  } catch {
    // ignore — never block graph open on template insertion
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
export async function switchGraph(): Promise<void> {
  const platform = await platformKind();
  if (platform === "android") {
    let result;
    try {
      result = await backend().pickGraphFolder();
    } catch (e) {
      pushToast(`Couldn't open the Android folder picker. (${String(e)})`, "error");
      return;
    }
    // Diagnostic breadcrumbs (visible in `adb logcat`, chromium console channel):
    // an intermittent first-run stall on "Opening…" — these pin down whether the
    // native picker returned and whether the graph parse completed or hung.
    console.info(`[tine/android] pickGraphFolder → ${result.status}`);
    if (result.status === "picked") {
      if (result.path) {
        console.info("[tine/android] loadGraphPath: start");
        await loadGraphPath(result.path);
        console.info("[tine/android] loadGraphPath: done");
      }
      return;
    }
    if (result.status === "permission-requested" || result.status === "permission-needed") {
      pushToast('Grant "All files access" for Tine, then tap Open again.', "info");
    }
    return;
  }
  if (platform === "ios") {
    pushToast(
      "Opening an existing graph on iOS is coming soon. For now, tap “Create a new graph” to try Tine.",
      "info"
    );
    return;
  }
  const path = await backend().pickFolder();
  if (path) await loadGraphPath(path);
}

/** Onboarding "create a new graph": pick where to put it, scaffold a small
 *  narrated demo graph there, open it, and land on the "Welcome to Tine" tour.
 *  No-op if the folder picker is cancelled. */
export async function createNewGraph(): Promise<void> {
  const dir = (await isMobile())
    ? await backend().defaultGraphParent()
    : await backend().pickFolder("Choose where to create your new graph");
  if (!dir) return;
  let root: string;
  try {
    root = await backend().createGraph(dir);
  } catch (e) {
    pushToast(`Couldn't create the graph. (${String(e)})`, "error");
    return;
  }
  const loaded = await loadGraphPath(root);
  if (loaded.kind !== "loaded" || loaded.root !== root) {
    pushToast(`Created the graph at ${root}, but kept the current graph open.`, "info");
    return;
  }
  await seedTodayJournal();
  openPage("Welcome to Tine", "page"); // land on the tour, not the empty journal feed
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
