import { createSignal } from "solid-js";
import { PaneContext, type PaneContextValue } from "./paneContext";
import {
  createPaneRouter,
  installPaneRouterRegistry,
  installLastTabCloseHandler,
  installNavigationInterceptor,
  historyRouteContextAdapter,
  mainPaneRouter,
  makePdfRoute,
  tabRoute,
  type AdoptedTab,
  type PaneSnapshot,
  type PaneRouter,
  type Route,
  type PageTarget,
  type PdfRoute,
} from "./router";
import { publishPdfNavigationIntent } from "./pdfNavigation";
import { registerPaneFocusSetter } from "./ui";
import { setCellSel } from "./sheet/selection";
import { clearSelection, pageByName, registerPaneRouteProvider, installHistoryRouteContextAdapter, node as docNode, feedNames } from "./document";
import { journalTitle, appNow } from "./journal";
import { isSinglePaneShell } from "./nativeChrome";
import {
  nearestPane,
  nearestPaneInDirection,
  takeBlockSelectionForPaneReturn,
  type PaneDirection,
} from "./paneSelect";
import { graphScopedSignal } from "./binding";
import { MAIN_WINDOW_ID, activeWindowId, setActiveWindowId, windowById, windowIdOf } from "./windowRealm";

export type LayoutNode =
  | {
      kind: "split";
      dir: "row" | "col";
      ratio: number;
      children: [LayoutNode, LayoutNode];
    }
  | { kind: "pane"; paneId: string };

// Workspace windows (OG-MULTIWINDOW P3): every Tine window owns its own pane
// tree, focused pane and maximized pane, keyed by its window id (src/windowRealm.ts).
// Pane ids are unique app-wide, so a pane belongs to exactly one window and
// `windowOfPane` recovers it. Calls without an explicit window act in the
// window the user is in (`activeWindowId`), which is what keyboard commands,
// the Quick Switcher and the palette mean by "here".
const MAIN_LAYOUT: LayoutNode = { kind: "pane", paneId: "main" };
const [layouts, setLayouts] = createSignal<Readonly<Record<string, LayoutNode>>>({ [MAIN_WINDOW_ID]: MAIN_LAYOUT });
const [focusedPanes, setFocusedPanes] = createSignal<Readonly<Record<string, string>>>({ [MAIN_WINDOW_ID]: "main" });
const maximizedPane = graphScopedSignal<Readonly<Record<string, string>>>();
const maximizedPanes = () => maximizedPane[0]() ?? {};
function setMaximizedPaneId(windowId: string, paneId: string | null) {
  const next = { ...maximizedPanes() };
  if (paneId === null) delete next[windowId];
  else next[windowId] = paneId;
  maximizedPane[1](Object.keys(next).length ? next : null);
}

/** The window the user is in, when it owns a pane tree (else main). */
export function currentLayoutWindowId(): string {
  const id = activeWindowId();
  return layouts()[id] ? id : MAIN_WINDOW_ID;
}

/** Ids of the windows that own a pane tree: main first, then workspace windows. */
export function layoutWindowIds(): string[] {
  return Object.keys(layouts());
}

/** The pane tree of `windowId` (the current window by default; main for an
 * unknown id). Reactive. */
export function layoutRoot(windowId = currentLayoutWindowId()): LayoutNode {
  const all = layouts();
  return all[windowId] ?? all[MAIN_WINDOW_ID] ?? MAIN_LAYOUT;
}

/** The window whose pane tree holds `paneId` (main when none does). O(panes). */
export function windowOfPane(paneId: string): string {
  const all = layouts();
  for (const id of Object.keys(all)) if (layoutPaneIds(all[id]).includes(paneId)) return id;
  return MAIN_WINDOW_ID;
}

/** Every pane id in every window, main's first. O(panes). */
export function allPaneIds(): string[] {
  return Object.values(layouts()).flatMap((node) => layoutPaneIds(node));
}

/** Return the pane tree shown on screen in `windowId`. Maximizing a pane leaves
 * the saved split tree and ratios intact; an unknown pane id falls back to that
 * tree. Cost: O(panes). No I/O or failure. */
export function visibleLayoutNode(windowId = currentLayoutWindowId()): LayoutNode {
  const id = maximizedPanes()[windowId];
  const root = layoutRoot(windowId);
  return id && layoutPaneIds(root).includes(id) ? { kind: "pane", paneId: id } : root;
}

/** Toggle a pane's transient full-area view. Returns false when no split or
 * target exists. Cost: O(panes); never changes the persisted layout. */
export function togglePaneMaximize(paneId = focusedPaneId()): boolean {
  const windowId = windowOfPane(paneId);
  if (maximizedPanes()[windowId] === paneId) {
    setMaximizedPaneId(windowId, null);
    return true;
  }
  const root = layoutRoot(windowId);
  if (!layoutHasMultiplePanes(root) || !layoutPaneIds(root).includes(paneId)) return false;
  setMaximizedPaneId(windowId, paneId);
  return true;
}

function commitLayout(node: LayoutNode, windowId: string) {
  const id = maximizedPanes()[windowId];
  if (id && !layoutPaneIds(node).includes(id)) setMaximizedPaneId(windowId, null);
  setLayouts({ ...layouts(), [windowId]: node });
}

/** The focused pane of `windowId` (the current window by default). */
export function focusedPaneId(windowId = currentLayoutWindowId()): string {
  const all = focusedPanes();
  return all[windowId] ?? firstPaneId(layoutRoot(windowId)) ?? "main";
}

/**
 * The one focus-state boundary: records `paneId` as the focused pane of the
 * window that owns it, clearing block/cell selection when focus moves, and
 * un-maximizes any other pane of that window so the focused pane is always
 * visible (history/session adapters call this directly). Does not validate that
 * the pane exists, switch windows, or activate its route; see focusPane.
 */
export function setFocusedPaneId(paneId: string) {
  const windowId = windowOfPane(paneId);
  const maximized = maximizedPanes()[windowId];
  if (maximized && maximized !== paneId) setMaximizedPaneId(windowId, null);
  if (focusedPaneId() !== paneId) {
    clearSelection();
    setCellSel(null);
  }
  setFocusedPanes({ ...focusedPanes(), [windowId]: paneId });
}

const routers = new Map<string, PaneRouter>([["main", mainPaneRouter]]);
let paneCounter = 1;

function freshPaneId(): string {
  let id = "";
  do {
    id = `pane-${paneCounter++}`;
  } while (routers.has(id));
  return id;
}

export function paneRouter(paneId: string): PaneRouter {
  const existing = routers.get(paneId);
  if (existing) return existing;
  const router = createPaneRouter(paneId);
  routers.set(paneId, router);
  return router;
}

export function mainRouter(): PaneRouter {
  return paneRouter("main");
}

export function focusedRouter(): PaneRouter {
  const paneId = focusedPaneId();
  if (paneId === "pdf") return mainRouter();
  return routers.has(paneId) ? paneRouter(paneId) : paneRouter(firstPaneId(layoutRoot()) ?? "main");
}

/** The focused pane's router of a given window. */
export function focusedRouterOf(windowId: string): PaneRouter {
  const paneId = focusedPaneId(windowId);
  return routers.has(paneId) ? paneRouter(paneId) : paneRouter(firstPaneId(layoutRoot(windowId)) ?? "main");
}

export { PaneContext, type PaneContextValue };

export function layoutPaneIds(node: LayoutNode = layoutRoot()): string[] {
  if (node.kind === "pane") return [node.paneId];
  return [...layoutPaneIds(node.children[0]), ...layoutPaneIds(node.children[1])];
}

export function layoutHasMultiplePanes(node: LayoutNode = layoutRoot()): boolean {
  return layoutPaneIds(node).length > 1;
}

export function firstPaneId(node: LayoutNode | null): string | null {
  if (!node) return null;
  return node.kind === "pane" ? node.paneId : firstPaneId(node.children[0]);
}

/** Routes shown in every window: a page open only in a workspace window stays
 * loaded exactly like one open in a main pane. */
export function activePaneRoutes(): Route[] {
  return allPaneIds().map((id) => paneRouter(id).route());
}

export function rewritePageTargetAcrossPanes(from: PageTarget, to: PageTarget) {
  for (const router of routers.values()) router.rewritePageTarget(from, to);
}

export function removePageTargetAcrossPanes(target: PageTarget) {
  for (const router of routers.values()) router.removePageTarget(target);
}

export function feedPaneId(windowId = currentLayoutWindowId()): string | null {
  return layoutPaneIds(layoutRoot(windowId)).find((id) => paneRouter(id).route().kind === "journals") ?? null;
}

export function replacePaneInLayout(
  node: LayoutNode,
  paneId: string,
  replacement: LayoutNode
): LayoutNode {
  if (node.kind === "pane") return node.paneId === paneId ? replacement : node;
  return {
    ...node,
    children: [
      replacePaneInLayout(node.children[0], paneId, replacement),
      replacePaneInLayout(node.children[1], paneId, replacement),
    ],
  };
}

export function splitLayoutNode(
  node: LayoutNode,
  paneId: string,
  dir: "row" | "col",
  newPaneId: string
): LayoutNode {
  return splitLayoutNodeAt(node, paneId, dir, newPaneId, "after");
}

export function splitLayoutNodeAt(
  node: LayoutNode,
  paneId: string,
  dir: "row" | "col",
  newPaneId: string,
  position: "before" | "after" = "after"
): LayoutNode {
  const oldLeaf: LayoutNode = { kind: "pane", paneId };
  const newLeaf: LayoutNode = { kind: "pane", paneId: newPaneId };
  return replacePaneInLayout(node, paneId, {
    kind: "split",
    dir,
    ratio: 0.5,
    children: position === "before" ? [newLeaf, oldLeaf] : [oldLeaf, newLeaf],
  });
}

function findSiblingFocus(node: LayoutNode): string {
  return firstPaneId(node) ?? "main";
}

export function closeLayoutPane(
  node: LayoutNode,
  paneId: string
): { node: LayoutNode; focusedPaneId: string; closed: boolean } {
  if (node.kind === "pane") {
    return { node, focusedPaneId: node.paneId, closed: false };
  }
  const [a, b] = node.children;
  if (a.kind === "pane" && a.paneId === paneId) {
    return { node: b, focusedPaneId: findSiblingFocus(b), closed: true };
  }
  if (b.kind === "pane" && b.paneId === paneId) {
    return { node: a, focusedPaneId: findSiblingFocus(a), closed: true };
  }
  const ca = closeLayoutPane(a, paneId);
  if (ca.closed) {
    return { node: { ...node, children: [ca.node, b] }, focusedPaneId: ca.focusedPaneId, closed: true };
  }
  const cb = closeLayoutPane(b, paneId);
  if (cb.closed) {
    return { node: { ...node, children: [a, cb.node] }, focusedPaneId: cb.focusedPaneId, closed: true };
  }
  return { node, focusedPaneId: firstPaneId(node) ?? "main", closed: false };
}

function routeForJournalsDuplicate(anchor: string | null): Route {
  const selectedDay = anchor ? docNode(anchor)?.page : undefined;
  const today = journalTitle(appNow());
  const name =
    (selectedDay && feedNames().includes(selectedDay) ? selectedDay : undefined) ??
    (feedNames().includes(today) ? today : feedNames()[0] ?? today);
  return { kind: "page", name, pageKind: pageByName(name)?.kind ?? "journal" };
}

function splitSnapshotForNewPane(source: PaneRouter): PaneSnapshot {
  const snap = source.duplicateActiveSnapshot();
  const active = snap.tabs[0];
  if (active && tabRoute({ id: "snapshot", history: active.history, pos: active.pos, pinned: active.pinned }).kind === "journals") {
    active.history = [routeForJournalsDuplicate(takeBlockSelectionForPaneReturn())];
    active.pos = 0;
    active.pinned = false;
  }
  return snap;
}

function snapshotFromAdoptedTab(tab: AdoptedTab): PaneSnapshot {
  return {
    tabs: [{ history: tab.history, pos: tab.pos, pinned: tab.pinned }],
    activeIndex: 0,
    scrolls: [tab.scroll],
  };
}

export function splitPane(
  paneId = focusedPaneId(),
  dir: "row" | "col" = "row",
  opts: { focusNew?: boolean; position?: "before" | "after"; snapshot?: PaneSnapshot } = {}
): string | null {
  if (isSinglePaneShell()) return null;
  const windowId = windowOfPane(paneId);
  if (!layoutPaneIds(layoutRoot(windowId)).includes(paneId)) return null;
  const newPaneId = freshPaneId();
  const source = paneRouter(paneId);
  const router = paneRouter(newPaneId);
  router.restoreSnapshot(opts.snapshot ?? splitSnapshotForNewPane(source));
  commitLayout(splitLayoutNodeAt(layoutRoot(windowId), paneId, dir, newPaneId, opts.position ?? "after"), windowId);
  if (opts.focusNew !== false) focusPane(newPaneId);
  focusedRouter().scheduleSessionSave();
  return newPaneId;
}

function nodeAtPath(node: LayoutNode, path: number[]): LayoutNode | null {
  let cur: LayoutNode | null = node;
  for (const idx of path) {
    if (!cur || cur.kind === "pane") return null;
    cur = cur.children[idx] ?? null;
  }
  return cur;
}

function nodeContainsPane(node: LayoutNode, paneId: string): boolean {
  if (node.kind === "pane") return node.paneId === paneId;
  return nodeContainsPane(node.children[0], paneId) || nodeContainsPane(node.children[1], paneId);
}

export function splitPaneAtSeam(
  path: number[],
  sourcePaneId: string | null,
  opts: { focusNew?: boolean; snapshot?: PaneSnapshot; windowId?: string } = {}
): string | null {
  const windowId = opts.windowId ?? (sourcePaneId ? windowOfPane(sourcePaneId) : currentLayoutWindowId());
  const root = layoutRoot(windowId);
  const split = nodeAtPath(root, path);
  if (!split || split.kind === "pane") return null;
  const source = sourcePaneId && layoutPaneIds(root).includes(sourcePaneId) ? sourcePaneId : null;
  const sourceSide =
    source && nodeContainsPane(split.children[0], source)
      ? 0
      : source && nodeContainsPane(split.children[1], source)
        ? 1
        : 0;
  const paneId = source && nodeContainsPane(split.children[sourceSide], source)
    ? source
    : firstPaneId(split.children[sourceSide]);
  if (!paneId) return null;
  return splitPane(paneId, split.dir, {
    position: sourceSide === 0 ? "after" : "before",
    focusNew: opts.focusNew,
    snapshot: opts.snapshot,
  });
}

export function splitRootAtEdge(
  side: "left" | "right" | "top" | "bottom",
  sourcePaneId = focusedPaneId(),
  opts: { focusNew?: boolean; snapshot?: PaneSnapshot } = {}
): string | null {
  if (isSinglePaneShell()) return null;
  const windowId = allPaneIds().includes(sourcePaneId) ? windowOfPane(sourcePaneId) : currentLayoutWindowId();
  const ids = layoutPaneIds(layoutRoot(windowId));
  const sourceId = ids.includes(sourcePaneId) ? sourcePaneId : ids[0];
  if (!sourceId) return null;
  const newPaneId = freshPaneId();
  paneRouter(newPaneId).restoreSnapshot(opts.snapshot ?? splitSnapshotForNewPane(paneRouter(sourceId)));
  const oldRoot = layoutRoot(windowId);
  const newLeaf: LayoutNode = { kind: "pane", paneId: newPaneId };
  const dir = side === "left" || side === "right" ? "row" : "col";
  const newFirst = side === "left" || side === "top";
  commitLayout({
    kind: "split",
    dir,
    ratio: 0.5,
    children: newFirst ? [newLeaf, oldRoot] : [oldRoot, newLeaf],
  }, windowId);
  if (opts.focusNew !== false) focusPane(newPaneId);
  focusedRouter().scheduleSessionSave();
  return newPaneId;
}

let closeWindowHandler: ((windowId: string) => boolean) | undefined;
/** src/workspaceWindows.ts: closing a workspace window's last pane closes it. */
export function installWorkspaceWindowCloser(handler: (windowId: string) => boolean): () => void {
  closeWindowHandler = handler;
  return () => { if (closeWindowHandler === handler) closeWindowHandler = undefined; };
}

export function closePane(paneId = focusedPaneId()): boolean {
  const windowId = windowOfPane(paneId);
  const root = layoutRoot(windowId);
  if (layoutPaneIds(root).length <= 1) {
    return windowId !== MAIN_WINDOW_ID && layoutPaneIds(root).includes(paneId) && !!closeWindowHandler?.(windowId);
  }
  const closingFocusedPane = focusedPaneId(windowId) === paneId;
  const res = closeLayoutPane(root, paneId);
  if (!res.closed) return false;
  commitLayout(res.node, windowId);
  if (paneId !== "main") routers.delete(paneId);
  // Closing a background pane must not manufacture a foreground visit. When
  // the focused pane closes, however, its sibling becomes the page the user is
  // actually looking at and must pass through the same activation boundary as
  // a pointer-driven pane focus change.
  if (closingFocusedPane) focusPane(res.focusedPaneId);
  focusedRouter().scheduleSessionSave();
  return true;
}

export function focusPane(paneId: string) {
  const windowId = windowOfPane(paneId);
  if (!layoutPaneIds(layoutRoot(windowId)).includes(paneId)) return;
  const otherWindow = windowId !== currentLayoutWindowId();
  if (!otherWindow && focusedPaneId() === paneId) return;
  setFocusedPaneId(paneId);
  if (otherWindow) {
    // A pane in another Tine window: that window becomes the one the user is in.
    setActiveWindowId(windowId);
    try { windowById(windowId)?.focus(); } catch { /* closing */ }
  }
  paneRouter(paneId).activateCurrentRoute();
}

function finishMovedTab(sourcePaneId: string, targetPaneId: string, moved: { emptied: boolean }) {
  focusPane(targetPaneId);
  if (moved.emptied) {
    closePane(sourcePaneId);
    if (layoutPaneIds().includes(targetPaneId)) focusPane(targetPaneId);
  } else {
    focusedRouter().scheduleSessionSave();
  }
}

export function moveTabToPane(
  sourcePaneId: string,
  tabId: string,
  targetPaneId: string,
  index?: number
): boolean {
  // Tabs move within one window; pointer capture does not cross OS windows.
  const ids = layoutPaneIds(layoutRoot(windowOfPane(sourcePaneId)));
  if (!ids.includes(sourcePaneId) || !ids.includes(targetPaneId)) return false;
  const source = paneRouter(sourcePaneId);
  if (!source.tabs().some((t) => t.id === tabId)) return false;
  if (sourcePaneId === targetPaneId) {
    if (typeof index === "number") source.moveTabToIndex(tabId, index);
    else source.setActiveTab(tabId);
    focusPane(targetPaneId);
    return true;
  }
  const moved = source.extractTabForAdoption(tabId);
  if (!moved) return false;
  paneRouter(targetPaneId).adoptTab(moved.tab, true, index);
  finishMovedTab(sourcePaneId, targetPaneId, moved);
  return true;
}

export function moveActiveTabToPane(sourcePaneId: string, targetPaneId: string): boolean {
  if (sourcePaneId === targetPaneId) return false;
  return moveTabToPane(sourcePaneId, paneRouter(sourcePaneId).activeId(), targetPaneId);
}

/**
 * Directional "Move tab to pane" (GH #282). When a pane lies in `dir` from
 * `sourcePaneId`, moves the source's active tab into it. With no neighbor the
 * layout grows in that direction: a multi-tab source donates its active tab to
 * the new pane; a one-tab source cannot be emptied (there is no empty-pane
 * route), so the new pane opens as a mirror of the current tab and the original
 * stays. Returns the pane that received the tab, or null when nothing changed
 * (unknown source, a refused move, or a platform without split panes). O(panes).
 */
export function moveActiveTabInDirection(sourcePaneId: string, dir: PaneDirection): string | null {
  const root = layoutRoot(windowOfPane(sourcePaneId));
  if (!layoutPaneIds(root).includes(sourcePaneId)) return null;
  const target = nearestPaneInDirection(root, sourcePaneId, dir);
  if (target) return moveActiveTabToPane(sourcePaneId, target) ? target : null;
  const side: "left" | "right" | "top" | "bottom" =
    dir === "up" ? "top" : dir === "down" ? "bottom" : dir;
  const source = paneRouter(sourcePaneId);
  if (source.tabs().length > 1) {
    return moveTabToSplitPane(sourcePaneId, source.activeId(), sourcePaneId, side);
  }
  return splitPane(sourcePaneId, side === "left" || side === "right" ? "row" : "col", {
    position: side === "left" || side === "top" ? "before" : "after",
  });
}

export function moveTabToSplitPane(
  sourcePaneId: string,
  tabId: string,
  targetPaneId: string,
  side: "left" | "right" | "top" | "bottom"
): string | null {
  const ids = layoutPaneIds(layoutRoot(windowOfPane(sourcePaneId)));
  if (isSinglePaneShell() || !ids.includes(sourcePaneId) || !ids.includes(targetPaneId)) return null;
  const source = paneRouter(sourcePaneId);
  if (!source.tabs().some((t) => t.id === tabId)) return null;
  const moved = source.extractTabForAdoption(tabId);
  if (!moved) return null;
  const newPaneId = splitPane(targetPaneId, side === "left" || side === "right" ? "row" : "col", {
    position: side === "left" || side === "top" ? "before" : "after",
    snapshot: snapshotFromAdoptedTab(moved.tab),
  });
  if (!newPaneId) return null;
  finishMovedTab(sourcePaneId, newPaneId, moved);
  return newPaneId;
}

export function moveTabToSeamSplit(sourcePaneId: string, tabId: string, path: number[]): string | null {
  const root = layoutRoot(windowOfPane(sourcePaneId));
  const ids = layoutPaneIds(root);
  const split = nodeAtPath(root, path);
  if (isSinglePaneShell() || !ids.includes(sourcePaneId) || !split || split.kind === "pane") return null;
  const source = paneRouter(sourcePaneId);
  if (!source.tabs().some((t) => t.id === tabId)) return null;
  const moved = source.extractTabForAdoption(tabId);
  if (!moved) return null;
  const newPaneId = splitPaneAtSeam(path, sourcePaneId, {
    snapshot: snapshotFromAdoptedTab(moved.tab),
  });
  if (!newPaneId) return null;
  finishMovedTab(sourcePaneId, newPaneId, moved);
  return newPaneId;
}

export function moveTabToRootEdge(
  sourcePaneId: string,
  tabId: string,
  side: "left" | "right" | "top" | "bottom"
): string | null {
  const ids = layoutPaneIds(layoutRoot(windowOfPane(sourcePaneId)));
  if (isSinglePaneShell() || !ids.includes(sourcePaneId)) return null;
  const source = paneRouter(sourcePaneId);
  if (!source.tabs().some((t) => t.id === tabId)) return null;
  const moved = source.extractTabForAdoption(tabId);
  if (!moved) return null;
  const newPaneId = splitRootAtEdge(side, sourcePaneId, {
    snapshot: snapshotFromAdoptedTab(moved.tab),
  });
  if (!newPaneId) return null;
  finishMovedTab(sourcePaneId, newPaneId, moved);
  return newPaneId;
}

export function setSplitRatio(path: number[], ratio: number, windowId = currentLayoutWindowId()) {
  const clamp = Math.min(0.85, Math.max(0.15, ratio));
  const update = (node: LayoutNode, depth: number): LayoutNode => {
    if (node.kind === "pane") return node;
    if (depth === path.length) return { ...node, ratio: clamp };
    const idx = path[depth];
    return {
      ...node,
      children: idx === 0
        ? [update(node.children[0], depth + 1), node.children[1]]
        : [node.children[0], update(node.children[1], depth + 1)],
    };
  };
  commitLayout(update(layoutRoot(windowId), 0), windowId);
  focusedRouter().scheduleSessionSave();
}

function panePath(node: LayoutNode, paneId: string, prefix: number[] = []): number[] | null {
  if (node.kind === "pane") return node.paneId === paneId ? prefix : null;
  return panePath(node.children[0], paneId, [...prefix, 0])
    ?? panePath(node.children[1], paneId, [...prefix, 1]);
}

/** Resize a pane by five percentage points at its nearest split on `axis`.
 * Returns false if no matching ancestor exists. Ratios clamp to 15–85% and
 * the normal session save persists the change. Cost: O(panes). */
export function adjustPaneSize(paneId: string, axis: "width" | "height", grow: boolean): boolean {
  const windowId = windowOfPane(paneId);
  const root = layoutRoot(windowId);
  const path = panePath(root, paneId);
  if (!path) return false;
  const dir = axis === "width" ? "row" : "col";
  for (let depth = path.length - 1; depth >= 0; depth--) {
    const ancestor = nodeAtPath(root, path.slice(0, depth));
    if (!ancestor || ancestor.kind !== "split" || ancestor.dir !== dir) continue;
    const delta = (grow ? 0.05 : -0.05) * (path[depth] === 0 ? 1 : -1);
    setSplitRatio(path.slice(0, depth), ancestor.ratio + delta, windowId);
    return true;
  }
  return false;
}

export function openRouteInOtherPane(route: Route, sourcePaneId = focusedPaneId()): string | null {
  const root = layoutRoot(windowOfPane(sourcePaneId));
  const ids = layoutPaneIds(root);
  let target = nearestPane(root, sourcePaneId) ?? ids.find((id) => id !== sourcePaneId) ?? null;
  const created = !target;
  if (!target) target = splitPane(sourcePaneId, "row", { focusNew: false });
  if (!target) return null;
  const router = paneRouter(target);
  if (created) {
    // The split seeded this pane with ONE duplicate tab; navigate it in
    // place so the pane ends up with a single tab whose back-history is the
    // source context (matching the embryo-switcher flow) — openInNewTab here
    // would leave a stray duplicate tab beside the target.
    if (route.kind === "journals") router.openJournals();
    else if (route.kind === "query" || route.kind === "pdf" || route.kind === "invalid" || route.kind === "conflicts") router.replaceActiveRoute(route);
    else if (route.block) router.openPageAtBlock(route.name, route.pageKind, route.block, route.path);
    else if (route.path) router.openFile(route.path, route.name, route.pageKind);
    else router.openPage(route.name, route.pageKind);
  } else {
    router.openInNewTab(route, true);
  }
  setFocusedPaneId(sourcePaneId);
  return target;
}

function pdfTab(filename: string): { paneId: string; tabId: string; route: PdfRoute } | null {
  // One editable view per file app-wide (annotation mutation ownership), so the
  // search covers every window.
  for (const paneId of allPaneIds()) {
    for (const tab of paneRouter(paneId).tabs()) {
      const route = tabRoute(tab);
      if (route.kind === "pdf" && route.filename === filename) return { paneId, tabId: tab.id, route };
    }
  }
  return null;
}

export interface OpenPdfOptions {
  sourcePaneId?: string;
  inPlace?: boolean;
  background?: boolean;
  anotherView?: boolean;
}

/** Open a graph PDF as an ordinary route. Reuses one view per file, keeping its
 * page when no target is requested. Desktop uses a companion pane; mobile uses
 * the current tab's history. Cost O(open tabs), no graph I/O. */
export function openPdf(filename: string, label: string, page?: number,
  highlightId?: string, options: OpenPdfOptions = {}): PdfRoute | null {
  const sourcePaneId = options.sourcePaneId && allPaneIds().includes(options.sourcePaneId)
    ? options.sourcePaneId : focusedPaneId();
  const sourceRoot = layoutRoot(windowOfPane(sourcePaneId));
  const ids = layoutPaneIds(sourceRoot);
  const existing = options.anotherView ? null : pdfTab(filename);
  if (existing) {
    const router = paneRouter(existing.paneId);
    router.setActiveTab(existing.tabId);
    if (page !== undefined) router.updateActivePdfViewState({ page });
    if (page !== undefined || highlightId !== undefined) {
      publishPdfNavigationIntent(existing.route.viewId, { page, highlightId });
    }
    if (!options.background) focusPane(existing.paneId);
    return existing.route;
  }
  // A second editable view needs shared annotation mutation ownership.
  if (options.anotherView) return null;
  const route = makePdfRoute(filename, label, { page });
  publishPdfNavigationIntent(route.viewId, { page, highlightId });
  if (isSinglePaneShell() || options.inPlace) {
    paneRouter(sourcePaneId).openPdf(route);
    return route;
  }
  const companion = nearestPane(sourceRoot, sourcePaneId) ?? ids.find((id) => id !== sourcePaneId) ?? null;
  if (companion) {
    paneRouter(companion).openInNewTab(route, !options.background);
    if (!options.background) focusPane(companion);
    return route;
  }
  const created = splitPane(sourcePaneId, "row", { focusNew: !options.background,
    snapshot: { tabs: [{ history: [route], pos: 0, pinned: false }], activeIndex: 0, scrolls: [null] } });
  return created ? route : null;
}

/** Open a PDF's notes in the companion pane, reusing its existing page tab.
 * On mobile the notes enter the current route history. Cost O(open tabs). */
export function openPdfNotes(sourcePaneId: string, notesPage: string, block?: string): string | null {
  if (isSinglePaneShell()) {
    const router = paneRouter(sourcePaneId);
    if (block) router.openPageAtBlock(notesPage, "page", block);
    else router.openPage(notesPage, "page");
    return sourcePaneId;
  }
  const targetPaneId = nearestPane(layoutRoot(windowOfPane(sourcePaneId)), sourcePaneId);
  if (targetPaneId) {
    const router = paneRouter(targetPaneId);
    const existing = router.tabs().find((tab) => {
      const route = tabRoute(tab);
      return route.kind === "page" && route.name === notesPage && route.pageKind === "page";
    });
    if (existing) {
      router.setActiveTab(existing.id);
      if (block) router.openPageAtBlock(notesPage, "page", block);
      focusPane(sourcePaneId);
      return targetPaneId;
    }
  }
  return openRouteInOtherPane({ kind: "page", name: notesPage, pageKind: "page",
    ...(block ? { block } : {}) }, sourcePaneId);
}

/** Reset the MAIN window to its single "main" pane. Workspace windows keep
 * their own trees (graph switch closes them first, src/workspaceWindows.ts). */
export function resetPaneLayoutToSingle(snapshot?: PaneSnapshot) {
  setMaximizedPaneId(MAIN_WINDOW_ID, null);
  commitLayout(MAIN_LAYOUT, MAIN_WINDOW_ID);
  if (snapshot) mainRouter().restoreSnapshot(snapshot);
  const kept = new Set(allPaneIds());
  for (const id of [...routers.keys()]) {
    if (id !== "main" && !kept.has(id)) routers.delete(id);
  }
  setFocusedPaneId("main");
}

/** Install a saved pane tree into `windowId` (main by default). Pane ids that
 * already belong to ANOTHER window are refused (returns false) so two windows
 * can never share a router. */
export function restorePaneLayout(
  root: LayoutNode,
  snapshots: Map<string, PaneSnapshot>,
  focused = "main",
  windowId: string = MAIN_WINDOW_ID,
): boolean {
  const ids = layoutPaneIds(root);
  const elsewhere = new Set(Object.entries(layouts()).filter(([id]) => id !== windowId)
    .flatMap(([, node]) => layoutPaneIds(node)));
  if (ids.some((id) => elsewhere.has(id)) || (windowId !== MAIN_WINDOW_ID && ids.includes("main"))) return false;
  for (const id of ids) {
    const snap = snapshots.get(id);
    if (snap) paneRouter(id).restoreSnapshot(snap);
    else paneRouter(id);
  }
  commitLayout(root, windowId);
  for (const id of [...routers.keys()]) {
    if (id !== "main" && !ids.includes(id) && !elsewhere.has(id)) routers.delete(id);
  }
  setFocusedPaneId(ids.includes(focused) ? focused : ids[0] ?? "main");
  return true;
}

/** Give a new workspace window a one-pane tree showing `snapshot`. Returns the
 * new pane id. */
export function createWindowLayout(windowId: string, snapshot: PaneSnapshot): string {
  const paneId = freshPaneId();
  paneRouter(paneId).restoreSnapshot(snapshot);
  commitLayout({ kind: "pane", paneId }, windowId);
  setFocusedPanes({ ...focusedPanes(), [windowId]: paneId });
  return paneId;
}

/** Forget a closed workspace window's pane tree and routers. Idempotent. */
export function dropWindowLayout(windowId: string) {
  if (windowId === MAIN_WINDOW_ID || !layouts()[windowId]) return;
  const ids = layoutPaneIds(layouts()[windowId]);
  const nextLayouts = { ...layouts() };
  delete nextLayouts[windowId];
  setLayouts(nextLayouts);
  const nextFocus = { ...focusedPanes() };
  delete nextFocus[windowId];
  setFocusedPanes(nextFocus);
  setMaximizedPaneId(windowId, null);
  for (const id of ids) if (id !== "main") routers.delete(id);
}

installPaneRouterRegistry({
  focusedRouter,
  mainRouter,
  routerForPane: (paneId) => routers.get(paneId),
  activatePane: (paneId) => {
    if (!allPaneIds().includes(paneId) || !routers.has(paneId)) return false;
    setFocusedPaneId(paneId);
    return true;
  },
});
installHistoryRouteContextAdapter(historyRouteContextAdapter);
installLastTabCloseHandler((paneId) => closePane(paneId));
installNavigationInterceptor((paneId, r) => {
  if (r.kind !== "journals") return false;
  const existing = feedPaneId(windowOfPane(paneId));
  if (existing && existing !== paneId) {
    focusPane(existing);
    return true;
  }
  return false;
});
registerPaneRouteProvider(activePaneRoutes);
// Pointer/focus-driven pane changes are genuine foreground activations. Raw
// setFocusedPaneId remains for restore/preload/layout construction, which must
// not rewrite RECENT merely because a saved session was reconstructed.
registerPaneFocusSetter((paneId, win) => {
  const windowId = windowIdOf(win) ?? MAIN_WINDOW_ID;
  // A click outside every pane keeps the window's own default: "main" in the
  // main window, the window's first pane in a workspace window.
  focusPane(paneId ?? (windowId === MAIN_WINDOW_ID ? "main" : firstPaneId(layoutRoot(windowId)) ?? "main"));
});
