// Opening / switching the active graph from the UI (native folder picker),
// persisting the choice so it reopens next launch.

import { backend } from "./backend";
import { setGraphMeta, setWorkflow, bumpGraphEpoch, setRightSidebar, graphMeta, graphEpoch, setAliasMap, seedFavorites, pruneSidebarBlocks, pushToast, refreshJournalConflicts, clearRecent } from "./ui";
import { resetStore, flushAll } from "./store";
import { clearAssetBlobCache } from "./assetCache";
import { resetTabsToJournals, openPage } from "./router";
import { journalTitle, setJournalTitleFormat } from "./journal";
import { applyTemplateVars } from "./editor/templateVars";
import type { BlockDto } from "./types";

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
export async function loadGraphPath(path: string): Promise<void> {
  // Whether we're switching to a *different* graph than last time. Only then do
  // we drop the persisted right-sidebar items; reopening the same graph at
  // startup keeps them (and we prune stale block refs below).
  const prev = persistedGraphPath();
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
    return;
  }
  const meta = await backend().loadGraph(path);
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
  setWorkflow(meta?.preferred_workflow === "todo" ? "todo" : "now");
  setJournalTitleFormat(meta?.journal_page_title_format); // match this graph's journal titles
  seedFavorites(meta?.favorites ?? []);
  void refreshJournalConflicts(true); // tell the user if any day has duplicate journal files
  if (path) {
    try {
      localStorage.setItem(GRAPH_KEY, path);
    } catch {
      // ignore
    }
  }
  bumpGraphEpoch();
  void injectCustomCss();
  void loadAliases();
  if (!switching) void pruneSidebarBlocks();
  await ensureJournalTemplate();
  // On a genuine graph SWITCH, close ALL the old graph's tabs (their histories
  // point at pages that don't exist in the new graph) and land on a single fresh
  // Journals tab. On the initial startup load of the same graph, `restoreSession()`
  // has already set up the tabs and focused one — leave that untouched, else a
  // restored pinned page tab would revert to Journals after every relaunch.
  if (switching) resetTabsToJournals();
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
const loadAliases = refreshAliases;

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
      { name: title, kind: "journal", title, pre_block: null, blocks: tmpl.blocks.map(resolve) },
      null, // brand-new journal — no baseline
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
  let el = document.getElementById("tine-custom-css");
  if (!el) {
    el = document.createElement("style");
    el.id = "tine-custom-css";
    document.head.appendChild(el);
  }
  el.textContent = css;
}

/** Pick a folder and open it as the graph. No-op if cancelled. */
export async function switchGraph(): Promise<void> {
  const path = await backend().pickFolder();
  if (path) await loadGraphPath(path);
}

/** Onboarding "create a new graph": pick where to put it, scaffold a small
 *  narrated demo graph there, open it, and land on the "Welcome to Tine" tour.
 *  No-op if the folder picker is cancelled. */
export async function createNewGraph(): Promise<void> {
  const dir = await backend().pickFolder("Choose where to create your new graph");
  if (!dir) return;
  let root: string;
  try {
    root = await backend().createGraph(dir);
  } catch (e) {
    pushToast(`Couldn't create the graph. (${String(e)})`, "error");
    return;
  }
  await loadGraphPath(root);
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
