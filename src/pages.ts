import { createMemo, createResource, createRoot } from "solid-js";
import { backend } from "./backend";
import { dataRev, graphEpoch, pageInventoryRev } from "./ui";
import { waitForWarmCache } from "./warmCache";
import type { PageEntry } from "./types";

// ONE graph-wide physical page list and reference-name list, shared by every
// namespace + sidebar consumer. Physical entries keep paths/owners for All Pages;
// reference names only enrich the shared namespace inventory after warm completes.
// The physical list has its own create/delete invalidation lane, so ordinary saves
// never turn into a whole-page-list IPC.
//
// Before this, `NamespaceHierarchy` keyed its `listPages()` resource on the page
// NAME, so it re-pulled the whole page list over IPC (+ an O(allPages) scan) on
// EVERY page navigation, even for a page with no namespace; and the sidebar, the
// namespace tree, and the {{namespace}} macro each ran their own duplicate
// whole-graph fetch. On a large graph that was hundreds of ms + GC churn per nav.
//
// Own root: lives for the app's lifetime by design (mirrors blockRefCounts.ts /
// resolveBatch.ts). The initial epoch (0) is a real resource key, so first paint
// fetches; a later graphEpoch change refetches and fixes the old mount/load race.
const pageInventory = createRoot(() => {
  const [physicalPages] = createResource(
    () => ({ epoch: graphEpoch(), inventory: pageInventoryRev() }),
    async ({ epoch, inventory }) => {
      const pages = await backend().listPages().catch(() => [] as PageEntry[]);
      return epoch === graphEpoch() && inventory === pageInventoryRev() ? pages : [];
    }
  );
  const [referencedNames] = createResource(
    () => ({ epoch: graphEpoch(), revision: dataRev(), inventory: pageInventoryRev() }),
    async ({ epoch, revision, inventory }) => {
      // `referenced_page_names` deliberately returns empty before the Rust warm
      // cache exists. Wait for its completion event instead, so that early empty
      // result cannot get memoized for this frontend revision.
      if (!(await waitForWarmCache(epoch))) return [];
      if (epoch !== graphEpoch() || revision !== dataRev() || inventory !== pageInventoryRev()) return [];
      const names = await backend().referencedPageNames().catch(() => [] as string[]);
      return epoch === graphEpoch() && revision === dataRev() && inventory === pageInventoryRev()
        ? names
        : [];
    }
  );
  // Physical display spellings win case-insensitively; the reference-only names
  // then fill gaps. This is the sole complete page-name inventory for namespace
  // consumers, while `allPages` intentionally remains physical-only.
  const names = createMemo(() => {
    const seen = new Set<string>();
    const inventory: string[] = [];
    const add = (name: string) => {
      const key = name.toLowerCase();
      if (!seen.has(key)) {
        seen.add(key);
        inventory.push(name);
      }
    };
    for (const page of physicalPages() ?? []) add(page.name);
    for (const name of referencedNames() ?? []) add(name);
    return inventory;
  });
  return { physicalPages, names };
});

/** All pages in the current graph (`undefined` until the first load). Reactive:
 *  reading it in a tracking scope re-runs when the list (re)loads. */
export function allPages(): PageEntry[] | undefined {
  return pageInventory.physicalPages();
}

/** Complete physical + reference-only page-name inventory (`[]` until loaded).
 *  Reactive; physical names keep their display spelling on case-folded collisions. */
export function allPageNames(): string[] {
  return pageInventory.names();
}

function parentPathLabel(p: PageEntry): string {
  const root = p.kind === "journal" ? "journals/" : "pages/";
  const rel = p.path.startsWith(root) ? p.path.slice(root.length) : p.path;
  const slash = rel.lastIndexOf("/");
  return slash >= 0 ? `${rel.slice(0, slash)}/` : root;
}

export function pageListLabel(p: PageEntry, pages: PageEntry[]): string {
  const same = pages.filter((x) => x.kind === p.kind && x.name === p.name);
  if (same.length < 2) return p.name;
  const label = parentPathLabel(p);
  const unique = same.filter((x) => parentPathLabel(x) === label).length === 1;
  return `${p.name} — ${unique ? label : p.path}`;
}
