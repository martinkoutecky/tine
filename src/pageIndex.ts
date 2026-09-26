import { createEffect, createMemo, createRoot, createSignal, on } from "solid-js";
import { backend } from "./backend";
import { dataRev, graphEpoch, pageIdentityKey, pageInventoryRev } from "./ui";
import type { PageEntry, PageInventory, PageInventoryEntry, PageKind, ResolvedPage } from "./types";

// The frontend's ONE name answerer: the latest `page_inventory` result, keyed by
// the backend's `refs::page_key`. It answers a name by looking up
// `pageIdentityKey(name)` and returning that entry's `target`. It decides no
// precedence, alias or collision itself: the store already did (an existing file
// beats a colliding alias, several alias owners are in path order, and names fold
// by `page_key`). All Pages and the namespace name list are views of the same
// data. No other frontend module may cache `page_inventory` or build a name map
// (`pageIndex.guard.test.ts`).
//
// Refresh economy: one `page_inventory` IPC per trigger tick (graph bind or
// switch, a content save's `dataRev`, a create/delete/rename's
// `pageInventoryRev`). v0.6.5 paid one alias IPC per content save; this is the
// same count. The command waits for the initial load, so no warm-cache gate.

interface Held {
  generation: number;
  rev: bigint;
  entries: PageInventoryEntry[];
  /** Page-kind and journal-kind entries by `key`. */
  pages: Map<string, PageInventoryEntry>;
  journals: Map<string, PageInventoryEntry>;
  /** Display name of every physical file id. */
  nameById: Map<string, string>;
}

const [held, setHeld] = createSignal<Held | null>(null);
// Bumped by `resetPageIndex` (graph bind/switch, rename): every response to a
// request issued before it is dropped.
let generation = 0;
let queued = false;

function build(inventory: PageInventory, rev: bigint): Held {
  const pages = new Map<string, PageInventoryEntry>();
  const journals = new Map<string, PageInventoryEntry>();
  const nameById = new Map<string, string>();
  for (const entry of inventory.entries) {
    // Several entries share a key only for physical spelling twins (`Foo.md`
    // and `foo.md`); each is an `existing` target that the backend resolves to
    // the same page, so whichever is kept navigates identically.
    (entry.is_journal ? journals : pages).set(entry.key, entry);
    if (entry.target.kind === "existing") {
      for (const id of [entry.target.id, ...entry.target.others]) {
        if (!nameById.has(id)) nameById.set(id, entry.name);
      }
    }
  }
  return { generation, rev, entries: inventory.entries, pages, journals, nameById };
}

/** Fetch the inventory now and keep it unless a newer one is already held or
 *  the graph was rebound meanwhile. */
export async function refreshPageIndex(): Promise<void> {
  const requested = generation;
  const epoch = graphEpoch();
  let inventory: PageInventory;
  try {
    inventory = await backend().pageInventory();
  } catch {
    return;
  }
  if (requested !== generation || epoch !== graphEpoch()) return;
  const rev = BigInt(inventory.rev);
  const current = held();
  if (current && current.generation === generation && rev < current.rev) return;
  setHeld(build(inventory, rev));
}

function scheduleRefresh(): void {
  if (queued) return;
  queued = true;
  queueMicrotask(() => {
    queued = false;
    void refreshPageIndex();
  });
}

/** Forget the held inventory (graph bind/switch, rename). */
export function resetPageIndex(): void {
  generation++;
  setHeld(null);
}

let installed: { allPages: () => PageEntry[] | undefined; allPageNames: () => string[] } | null = null;

/** Start the refresh triggers (idempotent). App calls it at mount; every view
 *  calls it too, so a view mounted without App still loads. */
export function installPageIndex(): void {
  ensureInstalled();
}

function ensureInstalled() {
  if (installed) return installed;
  installed = createRoot(() => {
    createEffect(on([graphEpoch, dataRev, pageInventoryRev], () => scheduleRefresh()));
    const allPages = createMemo(() => {
      const current = held();
      if (!current) return undefined;
      const rows: PageEntry[] = [];
      for (const entry of current.entries) {
        if (entry.target.kind !== "existing") continue;
        // A duplicate-day journal lists only its canonical file (v0.6.5).
        const ids = entry.is_journal ? [entry.target.id] : [entry.target.id, ...entry.target.others];
        for (const path of ids) {
          rows.push({
            name: entry.name,
            kind: entry.is_journal ? "journal" : "page",
            date_key: entry.day,
            path,
          });
        }
      }
      return rows;
    });
    const allPageNames = createMemo(() => {
      const current = held();
      if (!current) return [];
      // Physical display spellings win case-insensitively; alias and
      // reference-only names then fill gaps.
      const seen = new Set<string>();
      const names: string[] = [];
      const add = (name: string) => {
        const key = name.toLowerCase();
        if (!seen.has(key)) {
          seen.add(key);
          names.push(name);
        }
      };
      for (const page of allPages() ?? []) add(page.name);
      for (const entry of current.entries) {
        if (entry.target.kind !== "existing") add(entry.name);
      }
      return names;
    });
    return { allPages, allPageNames };
  });
  return installed;
}

/** The backend's answer for `name`, or `undefined` before the first load and
 *  for a name the inventory does not list (neither is a file or an alias). */
export function resolvedTarget(name: string, kind: PageKind = "page"): ResolvedPage | undefined {
  ensureInstalled();
  const current = held();
  return (kind === "journal" ? current?.journals : current?.pages)?.get(pageIdentityKey(name))?.target;
}

/** The page name navigation opens for `name`: the file's own name for
 *  `existing`, the first owner's name for `alias`, else `name` unchanged. */
export function navigationName(name: string): string {
  const target = resolvedTarget(name);
  if (!target || target.kind === "absent") return name;
  const id = target.kind === "existing" ? target.id : target.owners[0];
  return held()?.nameById.get(id) ?? name;
}

/** Every physical page and journal file (`undefined` until the first load). */
export function allPages(): PageEntry[] | undefined {
  return ensureInstalled().allPages();
}

/** Physical, alias and reference-only page names (`[]` until loaded). */
export function allPageNames(): string[] {
  return ensureInstalled().allPageNames();
}
