import { msg } from "./i18n";
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { graphOwner, readOwned } from "./owned";
import { dataRev, graphEpoch } from "./graphSession";
import { pageByName, readPageProperty } from "./document";
import { pushToast } from "./toasts";

// Inline page-icon lookups, made cheap for icon-heavy pages. Every `icon::`
// requested in the same microtask tick is coalesced into ONE page_icons IPC, each
// name is fetched at most ONCE per publication, and — crucially — a batch whose
// answers match what is shown does NOT touch the signal, so the common case (few/no
// icon:: pages, e.g. a 2000-block page of plain refs) costs one IPC and zero
// re-renders. A publication keeps the shown icons until their fresh answer lands
// (no flash of every icon on each save).
// A plain reactive read (not createResource-per-ref) keeps the per-reference cost
// to a signal read + object lookup for unloaded pages. Loaded pages use the
// reactive parser-owned header (O(header bytes), no IPC), including unsaved
// edits. Unloaded names refresh on publication, in one batch per microtask.
let cacheRev = -1;
let cacheDataRev = -1;
const [iconMap, setIconMap] = createSignal<Record<string, string>>({});
const requested = new Set<string>();
let pending: string[] = [];
let scheduled = false;

function ensureRev() {
  const epoch = graphEpoch();
  const revision = dataRev();
  if (epoch !== cacheRev || revision !== cacheDataRev) {
    if (epoch !== cacheRev) setIconMap({}); // new graph: drop the previous graph's icons
    cacheRev = epoch;
    cacheDataRev = revision;
    requested.clear(); // re-ask on publication; the answer replaces (or removes) each icon
    pending = [];
  }
  return epoch;
}

function flush() {
  scheduled = false;
  if (!pending.length) return;
  const batch = pending;
  pending = [];
  // Before the window's graph is bound (a sidebar ref restored at launch)
  // page_icons refuses with no-graph/missing-graph-binding: a transient state,
  // not a failure. The binding bumps graphEpoch, which re-requests them.
  if (backend().graphBindingGeneration() === 0) {
    batch.forEach((name) => requested.delete(name));
    return;
  }
  const batchRev = cacheRev;
  const batchDataRev = cacheDataRev;
  const owner = graphOwner(() => graphEpoch() === batchRev && dataRev() === batchDataRev);
  void readOwned(owner, backend().pageIcons(batch))
    .then((result) => {
      if (result.kind === "stale") return;
      const map = result.value;
      // Only re-render if an answer differs from what is shown — otherwise the
      // signal (and every reference subscribed to it) stays untouched.
      const prev = iconMap();
      if (!batch.some((n) => (map[n] ?? "") !== (prev[n] ?? ""))) return;
      const next = { ...prev };
      for (const n of batch) {
        if (map[n]) next[n] = map[n];
        else delete next[n];
      }
      setIconMap(next);
    })
    .catch(() => {
      if (!owner()) return;
      batch.forEach((name) => requested.delete(name));
      pushToast(msg("page-icon-ui-could-not-load-page-icons-they-will-retry-when"), "error");
    });
}

/** Reactive: the referenced page's `icon::` (emoji/character) or "". Reading it
 *  lazily enqueues a batched fetch; the value fills in once the batch resolves. */
export function pageIcon(name: string): string {
  if (pageByName(name)) return readPageProperty(name, "icon")?.trim() ?? "";
  ensureRev();
  if (!requested.has(name)) {
    requested.add(name);
    pending.push(name);
    if (!scheduled) {
      scheduled = true;
      queueMicrotask(flush);
    }
  }
  return iconMap()[name] ?? "";
}

/** `pageIcon(name)` as a label prefix ("🏁 "), or "" when the page has no `icon::`.
 *  Sidebar rows (Favorites, Recent, All pages) and right-sidebar item titles label
 *  themselves with it, as OG does (GH #346). */
export function pageIconPrefix(name: string): string {
  const icon = pageIcon(name);
  return icon ? `${icon} ` : "";
}
