// Favorites: membership, identity and the arrangement page (family 22).
//
// ONE state: the arrangement tree. `favorites()` is its flat pre-order
// membership, so the sidebar, the star buttons and config.edn can never
// disagree. Every change goes through `commit`, a graph-scoped preference write
// that rolls the whole tree back (and toasts) when persisting fails.
//
// Persisting writes the arrangement page first (only once the tree holds a
// label or a nested row — a graph that never groups never grows a page), then
// membership and the page's name in ONE config write. The page is user data:
// Tine writes it only over the version it last saw; a page that changed on disk
// is adopted instead, and an edit to it (in Tine or outside) is a membership
// statement, projected to config.edn without writing the page back.
import { createEffect, createRoot, createSignal, on } from "solid-js";
import { backend } from "./backend";
import type { createPage, favoritesArrangementBlocks, favoritesArrangementPage, reloadHlsIfLoaded } from "./document";
import { dataRev } from "./graphSession";
import { seedGraphSignal, writeGraphSignal } from "./graphPreferences";
import { graphOwner, readOwned, writeOwned } from "./owned";
import { navigationName } from "./pageIndex";
import { pageIdentityKey } from "./pageIdentity";
import { pushToast } from "./toasts";
import type { BlockDto, PageKind } from "./types";
import type { PageTarget } from "./routeTypes";
import {
  DEFAULT_FAVORITES_PAGE, FAVORITES_PAGE_PROPERTY, type FavItem, type FavLayout, type FavNode,
  carriesArrangement, favoriteNode, itemKind, labelNode, layoutFromBlocks, layoutMembers,
  layoutToMarkdown, moveNode, nodeAt, reconcileLayout, uniqueGroupName, updateAt,
} from "./favoritesLayout";

export type { FavItem } from "./favoritesLayout";

const [layout, setLayout] = createSignal<FavLayout>([]);
export const favoritesLayout = layout;
export const favorites = (): FavItem[] => layoutMembers(layout());

/** The page named by `:tine/favorites-page`, once this graph has one. */
let arrangementPage: string | null = null;
/** The arrangement page's content as Tine last saw it on disk (Markdown of its
 *  tree), or null when unknown. A write proceeds only over this version. */
let pageBase: string | null = null;
let generation = 0;
/** Counts Tine's own page writes, so a read that began before one is not
 *  mistaken for an outside edit. */
let ownWrites = 0;
const MARKER = `${FAVORITES_PAGE_PROPERTY}:: true`;
/** The document door's page write, installed by graph.ts: document reaches
 *  this module through ui.ts, so a static import would close a cycle (I-11). */
export interface FavoritesPageDoor {
  createPage: typeof createPage; favoritesArrangementPage: typeof favoritesArrangementPage;
  favoritesArrangementBlocks: typeof favoritesArrangementBlocks; reloadHlsIfLoaded: typeof reloadHlsIfLoaded;
}
let door: FavoritesPageDoor | null = null;
export function installFavoritesPageDoor(installed: FavoritesPageDoor): void { door = installed; }
function pageDoor(): FavoritesPageDoor {
  if (!door) throw new Error("the Favorites page door is not installed");
  return door;
}
const diskLayout = (blocks: BlockDto[]) => layoutFromBlocks(pageDoor().favoritesArrangementBlocks(blocks));
const IS_ARRANGEMENT_PAGE = /^tine\/favorites::\s*true\s*$/m;

/** THE favorites identity: kind, then the alias-resolved name folded like
 *  core `refs::page_key`. Membership, arrangement and deletion all use it. */
export function favoriteKey(name: string, kind: PageKind): string {
  return `${kind}\0${pageIdentityKey(kind === "page" ? navigationName(name) : name)}`;
}
const memberKey = (name: string) => favoriteKey(name, itemKind(name));
const nodeKey = (node: FavNode) => favoriteKey(node.target!, node.kind ?? itemKind(node.target!));

export function isFavorite(name: string): boolean {
  return favorites().some((f) => favoriteKey(f.name, f.kind) === favoriteKey(name, f.kind));
}

/** Replace the arrangement with a flat membership list, without writing. */
export function setFavorites(items: FavItem[]): void {
  setLayout(items.map((item) => favoriteNode(item.name, item.kind)));
}

function commit(next: FavLayout): void {
  writeGraphSignal("favorites", layout, setLayout, next, persistArrangement, "favorites");
}
/** The tree without the favorite(s) matching `key`; their children move up. */
const without = (nodes: FavNode[], key: string): FavNode[] => nodes.flatMap((node) =>
  node.target !== null && nodeKey(node) === key ? without(node.children, key) : [{ ...node, children: without(node.children, key) }]);

export function toggleFavorite(name: string, kind: PageKind = "page"): void {
  const key = favoriteKey(name, kind);
  if (isFavoriteKey(key)) commit(without(layout(), key));
  else commit([...layout(), favoriteNode(name, kind)]);
}

/** A page or journal was deleted: drop exactly that kind's favorite. */
export function forgetDeletedFavorite(name: string, kind: PageKind): void {
  const key = favoriteKey(name, kind);
  if (isFavoriteKey(key)) commit(without(layout(), key));
}
const isFavoriteKey = (key: string) => favorites().some((f) => favoriteKey(f.name, f.kind) === key);

/** A page was renamed on disk. Its links in the arrangement page were
 *  rewritten with it, so only membership is written here. */
export function renameFavorite(from: PageTarget, to: PageTarget): void {
  const key = favoriteKey(from.name, from.pageKind);
  let changed = false;
  const retarget = (nodes: FavNode[]): FavNode[] => nodes.map((node) => {
    const hit = node.target !== null && nodeKey(node) === key;
    changed ||= hit;
    return { ...(hit ? favoriteNode(to.name, to.pageKind) : node), collapsed: node.collapsed, children: retarget(node.children) };
  });
  const renamed = retarget(layout());
  if (!changed) return;
  const next = reconcileLayout(renamed, layoutMembers(renamed).map((f) => f.name), memberKey);
  if (arrangementPage) pageBase = layoutToMarkdown(next);
  commit(next);
}

/** Adopt a freshly opened graph's favorites. `names` (config.edn) decides
 *  WHICH pages are favorites; the arrangement page, when named, decides where
 *  they sit. Nothing is written. */
export function seedFavorites(names: string[], page: string | null = null): void {
  generation += 1;
  arrangementPage = page?.trim() || null;
  pageBase = null;
  setFavorites(names.map((name) => ({ name, kind: itemKind(name) })));
  seedGraphSignal("favorites");
  if (arrangementPage) void readArrangement(names);
}

let watching = false;
/** Re-read the arrangement page whenever graph data moves (own saves and
 *  outside edits both bump `dataRev`); a changed page is adopted. */
function watchArrangement(): void {
  if (watching) return;
  watching = true;
  createRoot(() => createEffect(on(dataRev, () => { if (arrangementPage) void readArrangement(null); }, { defer: true })));
}

/** Read the arrangement page. With `membership` (graph open), fold it into
 *  config's membership and seed; without (a later change), adopt the page as
 *  the user's statement of membership when it differs from what Tine saw. */
async function readArrangement(membership: string[] | null): Promise<void> {
  watchArrangement();
  const page = arrangementPage!;
  const gen = generation;
  const writes = ownWrites;
  const owner = graphOwner(() => gen === generation && writes === ownWrites);
  const read = await readOwned(owner, backend().getPage(page, "page")).catch((error: unknown) => {
    pushToast(`Could not read the Favorites page "${page}": ${String(error)}`, "error");
    return null;
  });
  if (read?.kind !== "current" || !read.value) return;
  const disk = diskLayout(read.value.blocks);
  const text = layoutToMarkdown(disk);
  if (membership) {
    pageBase = text;
    setLayout(reconcileLayout(disk, membership, memberKey));
    seedGraphSignal("favorites");
  } else if (text !== pageBase) {
    pageBase = text;
    commit(disk);
  }
}

const toBlocks = (nodes: FavNode[]): BlockDto[] =>
  nodes.map((node) => ({ id: "", raw: node.raw, collapsed: node.collapsed ?? false, children: toBlocks(node.children) }));

/** Write the arrangement page over the version Tine last saw, creating it
 *  lazily. A new page takes "Favorites", else "Favorites 2", …, never a user's
 *  own page; an orphaned arrangement page (a write that never reached config)
 *  is reused. Returns the page's name. */
async function writeArrangementPage(next: FavLayout, text: string): Promise<string> {
  const owner = graphOwner();
  const { createPage, favoritesArrangementPage, reloadHlsIfLoaded } = pageDoor();
  for (let n = 1; ; n += 1) {
    const name = arrangementPage ?? (n === 1 ? DEFAULT_FAVORITES_PAGE : `${DEFAULT_FAVORITES_PAGE} ${n}`);
    const read = await readOwned(owner, backend().getPage(name, "page"));
    if (read.kind === "stale") throw new Error("graph changed before the Favorites page write");
    const disk = read.value;
    if (disk && !arrangementPage && !IS_ARRANGEMENT_PAGE.test(disk.pre_block ?? "")) continue;
    if (disk && arrangementPage && layoutToMarkdown(diskLayout(disk.blocks)) !== pageBase) {
      void readArrangement(null);
      throw new Error(`the Favorites page "${name}" changed on disk; reloaded it`);
    }
    ownWrites += 1;
    await createPage(name, favoritesArrangementPage(name, MARKER, toBlocks(next)), { baseRev: disk?.rev ?? null });
    if (!owner()) throw new Error("graph changed during the Favorites page write");
    arrangementPage = name;
    pageBase = text;
    void reloadHlsIfLoaded(name);
    return name;
  }
}

function persistArrangement(next: FavLayout): Promise<unknown> {
  const owner = graphOwner();
  const names = layoutMembers(next).map((f) => f.name);
  const text = layoutToMarkdown(next);
  if (!(arrangementPage || carriesArrangement(next)) || text === pageBase)
    return backend().setFavorites(names, arrangementPage);
  return writeArrangementPage(next, text).then((page) => {
    if (!owner()) throw new Error("graph changed before the favorites write");
    return writeOwned(owner, backend().setFavorites(names, page)).catch((error: unknown) => {
      // The page is now ahead of the rolled-back tree: forget what Tine saw so
      // the next data change adopts the page and retries the projection.
      pageBase = null;
      throw error;
    });
  });
}

// Arrangement edits, addressed by path (child indices from the root).
export function moveFavoriteRow(from: number[], parent: number[], index: number): void {
  commit(moveNode(layout(), from, parent, index));
}
export function addFavoriteGroup(desired = "New group"): void {
  commit([...layout(), labelNode(uniqueGroupName(layout(), desired))]);
}
export function renameFavoriteGroup(path: number[], name: string): void {
  const node = nodeAt(layout(), path);
  if (!name.trim() || !node || node.target !== null) return;
  // Unique against every OTHER label, so an unchanged name keeps its spelling.
  const others = updateAt(layout(), path, (target) => target.children);
  commit(updateAt(layout(), path, (target) => [{ ...target, raw: uniqueGroupName(others, name.trim()) }]));
}
/** Delete a label WITHOUT unfavoriting anything: its children take its place. */
export function deleteFavoriteGroup(path: number[]): void {
  if (nodeAt(layout(), path)?.target !== null) return;
  commit(updateAt(layout(), path, (target) => target.children));
}
export function setFavoriteRowCollapsed(path: number[], collapsed: boolean): void {
  commit(updateAt(layout(), path, (target) => [{ ...target, collapsed: collapsed || undefined }]));
}
