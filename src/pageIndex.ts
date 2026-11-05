import { reportUiFailure, reportUnreadableFiles } from "./uiFailure";
import { createEffect, createMemo, createRoot, createSignal, on } from "solid-js";
import { backend } from "./backend";
import { graphOwner, latestOwner, readOwned } from "./owned";
import { graphEpoch, pageInventoryRev } from "./graphSession";
import { pageIdentityKey } from "./ui";
import { linkTargetIn, ownerPath, type LinkDestination } from "./inventoryRows";
import { hidden_from_all_pages } from "./render/wasm/lsdoc_wasm";
import type { PageEntry, PageInventory, PageInventoryDelta, PageInventoryEntry, PageKind, ResolvedPage } from "./types";

// The frontend's ONE name answerer: the latest `page_inventory` result, keyed by
// the backend's `refs::page_key`. It answers a name by looking up
// `pageIdentityKey(name)` and returning that entry's `target`. It decides no
// precedence, alias or collision itself: the store already did (an existing file
// beats a colliding alias, several alias owners are in path order, and names fold
// by `page_key`). All Pages and the namespace name list are views of the same
// data. No other frontend module may cache `page_inventory` or build a name map
// (`pageIndex.guard.test.ts`).
//
// Refresh economy: graph binding or native inventory-change signals cost one
// IPC per tick, and a burst while one is in flight costs one more. Text-only
// saves leave this answer alone (I-25). A change that only moved alias or
// reference-only names (typing inside `[[…]]`, an `alias::` edit) carries
// those entries and is patched in place with no IPC (`patchPageIndex`).

interface Held {
  generation: number;
  rev: bigint;
  entries: PageInventoryEntry[];
  /** The `existing` entries: the file list, which a patch never changes, so
   *  views of the files (All pages) keep their answer across patches. */
  files: PageInventoryEntry[];
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
let inFlight: string | null = null;
let again = false;
const refreshScope = {};
// False in a WebView the backend grants no graph-wide read (Quick Capture).
let inventoryGranted = true;

/** The Quick Capture WebView has no graph slot: the backend grants it only
 *  narrow capture-scoped commands (src-tauri `state.rs`), so `page_inventory`
 *  is refused there by design. Its entry point calls this before rendering:
 *  the index then never reads, and every name lookup answers as before the
 *  first load (`undefined`, the name unchanged, no names), which callers
 *  already handle. Capture completion uses `capture_quick_switch` instead.
 *  Without it, typing `[[` in Quick Capture raised "Couldn't refresh the page
 *  list" (UI-CAPTURE-NO-PAGE-INVENTORY). */
export function withoutPageInventory(): void {
  inventoryGranted = false;
}

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
  const files = inventory.entries.filter((entry) => entry.target.kind === "existing");
  return { generation, rev, entries: inventory.entries, files, pages, journals, nameById };
}

/** Rust's `str` order (UTF-8 bytes, i.e. code points), which the backend sorts
 *  the inventory by; JS `<` compares UTF-16 units and differs above U+FFFF. */
function compareCodePoints(left: string, right: string): number {
  if (left === right) return 0;
  const a = left[Symbol.iterator](), b = right[Symbol.iterator]();
  for (;;) {
    const x = a.next(), y = b.next();
    if (x.done || y.done) return x.done ? (y.done ? 0 : -1) : 1;
    if (x.value !== y.value) return x.value.codePointAt(0)! - y.value.codePointAt(0)!;
  }
}

/** The backend's inventory order: by key, then by name. */
function entryOrder(left: PageInventoryEntry, right: PageInventoryEntry): number {
  return compareCodePoints(left.key, right.key) || compareCodePoints(left.name, right.name);
}

/** Apply one publication's inventory delta to the held answer, as the next
 *  in-order change: the answer then equals a fresh `page_inventory` read at
 *  `rev`. Returns `null` when it cannot (nothing held yet, a rebind, or a read
 *  in flight or queued whose answer may predate or postdate this change): the
 *  caller refetches, as before. `aliases`: an alias entry was dropped or added.
 *  Cost O(entries) to copy the entry list, O(changed log entries) otherwise;
 *  no IPC. */
export function patchPageIndex(rev: bigint, delta: PageInventoryDelta): { aliases: boolean } | null {
  const current = held();
  if (!current || current.generation !== generation || inFlight !== null || queued) return null;
  // A read at or after `rev` already holds this change.
  if (rev <= current.rev) return { aliases: false };
  const keys = new Set(delta.keys);
  let aliases = delta.entries.some((entry) => entry.target.kind === "alias");
  const entries: PageInventoryEntry[] = [];
  for (const entry of current.entries) {
    if (keys.has(entry.key) && entry.target.kind !== "existing") {
      aliases ||= entry.target.kind === "alias";
      const map = entry.is_journal ? current.journals : current.pages;
      if (map.get(entry.key) === entry) map.delete(entry.key);
    } else {
      entries.push(entry);
    }
  }
  for (const entry of delta.entries) {
    let low = 0, high = entries.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (entryOrder(entries[middle], entry) <= 0) low = middle + 1;
      else high = middle;
    }
    entries.splice(low, 0, entry);
    // The full read keeps the last entry of a key in list order.
    const map = entry.is_journal ? current.journals : current.pages;
    const kept = map.get(entry.key);
    if (!kept || entryOrder(kept, entry) <= 0) map.set(entry.key, entry);
  }
  setHeld({ ...current, rev, entries });
  return { aliases };
}

/** Fetch the inventory now and keep it unless a newer one is already held or
 *  the graph was rebound meanwhile. */
export async function refreshPageIndex(): Promise<void> {
  if (!inventoryGranted) return;
  const requested = generation;
  const owner = latestOwner(refreshScope, "inventory", graphOwner(() => requested === generation));
  let inventory: PageInventory;
  try {
    const result = await readOwned(owner, backend().pageInventory());
    if (result.kind === "stale") return;
    inventory = result.value;
  } catch (error) {
    if (owner()) reportUiFailure("page-inventory", error);
    return;
  }
  if (inventory.unreadable?.length) reportUnreadableFiles(inventory.unreadable);
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
    void signalled();
  });
}

/** One signal-driven fetch in flight per graph binding, plus at most one
 *  trailing fetch for the signals that arrived meanwhile (PERF-FIXES D2): a
 *  burst of publications (a rename's referrers) costs two inventory reads,
 *  not one per publication. A rebind or graph switch never waits behind the
 *  previous binding's read. */
async function signalled(): Promise<void> {
  const binding = `${generation}:${graphEpoch()}`;
  if (inFlight === binding) { again = true; return; }
  inFlight = binding;
  try {
    do {
      again = false;
      await refreshPageIndex();
    } while (again && inFlight === binding);
  } finally {
    if (inFlight === binding) inFlight = null;
  }
}

/** Forget the held inventory (graph bind/switch, rename). */
export function resetPageIndex(): void {
  generation++;
  setHeld(null);
}

let installed: {
  allPages: () => PageEntry[] | undefined;
  allPageNames: () => string[];
  referenceOnlyPages: () => PageEntry[];
} | null = null;

/** Start the refresh triggers (idempotent). App calls it at mount; every view
 *  calls it too, so a view mounted without App still loads. */
export function installPageIndex(): void {
  ensureInstalled();
}

function ensureInstalled() {
  if (installed) return installed;
  installed = createRoot(() => {
    if (inventoryGranted) createEffect(on([graphEpoch, pageInventoryRev], () => scheduleRefresh()));
    // Keyed on the file list, which a patch leaves alone: All pages and the
    // views built on it do not recompute while names are typed.
    const files = createMemo(() => held()?.files);
    const allPages = createMemo(() => {
      const current = files();
      if (!current) return undefined;
      const rows: PageEntry[] = [];
      for (const entry of current) {
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
      // Physical display spellings win per page identity (`pageIdentityKey`,
      // the store's `page_key`); alias and reference-only names then fill gaps.
      const seen = new Set<string>();
      const names: string[] = [];
      const keys: string[] = [];
      const add = (name: string) => {
        const key = pageIdentityKey(name);
        if (!seen.has(key)) {
          seen.add(key);
          names.push(name);
          keys.push(key);
        }
      };
      for (const page of allPages() ?? []) add(page.name);
      for (const entry of current.entries) {
        if (entry.target.kind !== "existing") add(entry.name);
      }
      keysOfNames.set(names, keys);
      return names;
    });
    // The reference-only pages All pages lists (D-20261102-CM4 CM-REFONLY):
    // the backend's absent entries that open themselves (no `link`: not a
    // date spelling of a day, not an alias), less what OG's `get-all-pages`
    // drops (built-in pages, UUID names, drawings) by the Rust answer
    // (`hidden_from_all_pages`; CM step 2 batches 5 and 8). A patch that
    // leaves this list equal leaves All pages alone. O(entries).
    const referenceOnlyPages = createMemo<PageEntry[]>(() => {
      const current = held();
      if (!current) return [];
      const rows: PageEntry[] = [];
      for (const entry of current.entries) {
        if (entry.target.kind !== "absent" || entry.link || entry.is_journal || hidden_from_all_pages(entry.name)) continue;
        rows.push({ name: entry.name, kind: "page", date_key: null, path: "" });
      }
      return rows;
    }, [], { equals: (a, b) => a.length === b.length && a.every((row, at) => row.name === b[at].name) });
    return { allPages, allPageNames, referenceOnlyPages };
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

// The pure row readers live in `inventoryRows.ts`, shared with the published
// site's answers; this module applies them to the one held inventory.
export { ownerPath, type LinkDestination } from "./inventoryRows";

/** What `[[name]]` opens: `linkTargetIn` over the held inventory (the store's
 *  `WholeGraph::link_target`, carried as the entry's `link`; a name the
 *  inventory does not list answers as an absent page of the Rust-decided
 *  kind). O(1), no IPC. */
export function linkTarget(name: string): LinkDestination {
  ensureInstalled();
  return linkTargetIn(held(), name);
}

/** A named ordinary-page route (not a link): an alias opens what the store
 *  says its link opens (the owner, of the owner's kind: a journal's alias
 *  opens the journal), and so does a content-free file the store marks
 *  `empty_alias` (EMPTY-ALIAS, as Logseq's `get-redirect-page-name`); any
 *  other name stays an ordinary page of that name. A route carrying the
 *  file's path never comes here (the router keeps an exact owner). */
export function navigationTarget(name: string): LinkDestination {
  ensureInstalled();
  const entry = held()?.pages.get(pageIdentityKey(name));
  if ((entry?.target.kind === "alias" || entry?.empty_alias) && entry.link) return entry.link;
  return { name: navigationName(name), kind: "page" };
}

/** The page name navigation opens for `name`: the file's own name for
 *  `existing`, the alias's owner (the store's `link`) for `alias`, else `name`
 *  unchanged. */
export function navigationName(name: string): string {
  ensureInstalled();
  const entry = held()?.pages.get(pageIdentityKey(name));
  const target = entry?.target;
  if (!target || target.kind === "absent") return name;
  if (target.kind === "alias" && entry?.link) return entry.link.name;
  const id = ownerPath(target);
  return (id === undefined ? undefined : held()?.nameById.get(id)) ?? name;
}

/** Physical owner's display name, using the inventory's existing identity map. */
export function pageNameForPath(path: string): string | undefined {
  ensureInstalled();
  return held()?.nameById.get(path);
}

/** Every physical page and journal file (`undefined` until the first load). */
export function allPages(): PageEntry[] | undefined {
  return ensureInstalled().allPages();
}

/** The reference-only pages All pages lists, path `""` (they have no file). */
export function referenceOnlyPages(): PageEntry[] {
  return ensureInstalled().referenceOnlyPages();
}

/** Physical, alias and reference-only page names (`[]` until loaded). */
export function allPageNames(): string[] {
  return ensureInstalled().allPageNames();
}

/** Whether `name` is a namespace name, as the store answers it (Logseq's
 *  `namespace-page?`, `refs::is_namespace_name`; model round 10, S15): it
 *  contains `/` and its inventory entry carries no `not_namespace`. A name the
 *  inventory does not list (none loaded yet) counts by its `/` alone. `key` is
 *  `pageIdentityKey(name)` when the caller already has it. O(1). */
export function isNamespaceName(name: string, key?: string): boolean {
  if (!name.includes("/")) return false;
  const current = held();
  if (!current) return true;
  const k = key ?? pageIdentityKey(name);
  return !(current.pages.get(k) ?? current.journals.get(k))?.not_namespace;
}

const keysOfNames = new WeakMap<readonly string[], readonly string[]>();

/** The page keys (`pageIdentityKey`) of an `allPageNames()` result, index for
 *  index, already computed by its dedup; undefined for any other array. Lets a
 *  view that nests names (the namespace tree) skip a second key call per name. */
export function allPageNameKeys(names: readonly string[]): readonly string[] | undefined {
  return keysOfNames.get(names);
}
