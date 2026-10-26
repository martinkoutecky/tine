import { msg } from "./i18n";
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { graphOwner, readOwned } from "./owned";
import { graphEpoch } from "./graphSession";
import { changed, iconChangeKey } from "./changeKeys";
import { clearOnBindingInvalidated } from "./binding";
import { pageByName, readPageProperty } from "./document";
import { pushToast } from "./toasts";
import { PAGE_ICON_PROPERTIES } from "./pageIconImage";
import { pageNameForPath, resolvedTarget } from "./pageIndex";

// Inline page-icon lookups, made cheap for icon-heavy pages. Every icon property
// requested in the same microtask tick is coalesced into ONE page_icons IPC, each
// name is fetched at most ONCE per icon-key revision, and a batch whose
// answers match what is shown does NOT touch the signal, so the common case (few/no
// icon pages, e.g. a 2000-block page of plain refs) costs one IPC and zero
// re-renders. A publication keeps the shown icons until their fresh answer lands
// (no flash of every icon on each save).
// A plain reactive read (not createResource-per-ref) keeps the per-reference cost
// to a signal read + object lookup for unloaded pages. Loaded pages use the
// reactive parser-owned header (O(header bytes), no IPC), including unsaved
// edits. Unloaded names refresh on matching property publications or full
// invalidations, in one batch per microtask.
let cacheRev = -1;
const [iconMap, setIconMap] = createSignal<Record<string, string>>({});
const requested = new Map<string, string>();
let pending = new Map<string, string>();
let scheduled = false;
clearOnBindingInvalidated(() => { requested.clear(); pending.clear(); cacheRev = -1; setIconMap({}); });

function ensureRev() {
  const epoch = graphEpoch();
  if (epoch !== cacheRev) {
    setIconMap({}); // new graph: drop the previous graph's icons
    cacheRev = epoch;
    requested.clear();
    pending.clear();
  }
  return epoch;
}

function flush() {
  scheduled = false;
  if (!pending.size) return;
  const revisions = pending;
  const batch = [...revisions.keys()];
  pending = new Map();
  // Before the window's graph is bound (a sidebar ref restored at launch)
  // page_icons refuses with no-graph/missing-graph-binding: a transient state,
  // not a failure. The binding bumps graphEpoch, which re-requests them.
  if (backend().graphBindingGeneration() === 0) {
    batch.forEach((name) => requested.delete(name));
    return;
  }
  const batchRev = cacheRev;
  const owner = graphOwner(() => graphEpoch() === batchRev);
  void readOwned(owner, backend().pageIcons(batch))
    .then((result) => {
      if (result.kind === "stale") return;
      const map = result.value;
      const current = batch.filter(name => changed(iconChangeKey(name)) === revisions.get(name));
      // Only re-render if an answer differs from what is shown — otherwise the
      // signal (and every reference subscribed to it) stays untouched.
      const prev = iconMap();
      if (!current.some((n) => (map[n] ?? "") !== (prev[n] ?? ""))) return;
      const next = { ...prev };
      for (const n of current) {
        if (map[n]) next[n] = map[n];
        else delete next[n];
      }
      setIconMap(next);
    })
    .catch(() => {
      if (!owner()) return;
      batch.forEach((name) => { if (requested.get(name) === revisions.get(name)) requested.delete(name); });
      pushToast(msg("page-icon-ui-could-not-load-page-icons-they-will-retry-when"), "error");
    });
}

/** Reactive: the referenced page's icon (`tine.icon::`, else `icon::`) or "". Reading it
 *  lazily enqueues a batched fetch; the value fills in once the batch resolves. */
export function pageIcon(name: string): string {
  if (pageByName(name)) return readPageIconProperty(name);
  ensureRev();
  const revision = changed(iconChangeKey(name));
  if (requested.get(name) !== revision) {
    requested.set(name, revision);
    pending.set(name, revision);
    if (!scheduled) {
      scheduled = true;
      queueMicrotask(flush);
    }
  }
  return iconMap()[name] ?? "";
}

/** Loaded-page lookup, with the shared policy's property precedence
 *  (`tine.icon::`, then the OG-compatible `icon::`). */
export function readPageIconProperty(name: string): string {
  const property = pageIconPropertyInUse(name);
  return property ? readPageProperty(name, property)?.trim() ?? "" : "";
}

/** The property currently supplying a loaded page's icon (first nonblank in
 *  policy order), or "" when it has none. The menu edits this one in place so a
 *  page written for OG (`icon:: 🏁`) keeps a single, OG-visible property. */
export function pageIconPropertyInUse(name: string): string {
  for (const property of PAGE_ICON_PROPERTIES) {
    if (readPageProperty(name, property)?.trim()) return property;
  }
  return "";
}

/** Relative image links belong to the page supplying the icon. Alias claimants
 * can be in different directories; navigation's first owner need not supply an
 * icon. Reuse the same batched lookup to identify that owner, without a scan. */
export function pageIconOwnerPath(name: string, value: string): string {
  const loaded = pageByName(name)?.id;
  if (loaded) return loaded;
  const target = resolvedTarget(name);
  if (target?.kind === "existing") return target.id;
  if (target?.kind === "alias") {
    for (const path of target.owners) {
      const owner = pageNameForPath(path);
      if (owner && pageIcon(owner) === value) return path;
    }
  }
  return "";
}

/** `pageIcon(name)` as a label prefix ("🏁 "), or "" when the page has no icon property.
 *  Sidebar rows (Favorites, Recent, All pages) and right-sidebar item titles label
 *  themselves with it, as OG does (GH #346). */
export function pageIconPrefix(name: string): string {
  const icon = pageIcon(name);
  return icon ? `${icon} ` : "";
}
