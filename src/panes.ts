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
  type PdfRoute,
  type Route,
  type PageTarget,
} from "./router";
import { publishPdfNavigationIntent } from "./pdfNavigation";
import { registerPaneFocusSetter } from "./ui";
import { setCellSel } from "./sheet/selection";
import {
  clearSelection,
  doc,
  pageByName,
  registerPaneRouteProvider,
  installHistoryRouteContextAdapter,
} from "./store";
import { journalTitle } from "./journal";
import { isMobilePlatform } from "./nativeChrome";
import {
  nearestPane,
  nearestPaneInDirection,
  takeBlockSelectionForPaneReturn,
  type PaneDirection,
} from "./paneSelect";

export type LayoutNode =
  | {
      kind: "split";
      dir: "row" | "col";
      ratio: number;
      children: [LayoutNode, LayoutNode];
    }
  | { kind: "pane"; paneId: string };

export const [layoutRoot, setLayoutRoot] = createSignal<LayoutNode>({
  kind: "pane",
  paneId: "main",
});

// Transient maximize state (GH #285): the maximized pane borrows the whole
// pane area while layoutRoot() keeps the complete split tree and its ratios,
// so toggling back restores the exact arrangement and session/workspace
// persistence never sees a maximized-looking layout.
const [maximizedPaneId, setMaximizedPaneId] = createSignal<string | null>(null);
export { maximizedPaneId };

/**
 * The subtree to render in the pane area: only the maximized leaf, falling
 * back to the real tree when nothing (valid) is maximized.
 */
export function visibleLayoutNode(): LayoutNode {
  const id = maximizedPaneId();
  return id && layoutPaneIds().includes(id) ? { kind: "pane", paneId: id } : layoutRoot();
}

export function togglePaneMaximize(paneId = focusedPaneId()): boolean {
  if (maximizedPaneId() === paneId) {
    setMaximizedPaneId(null);
    return true;
  }
  if (!layoutHasMultiplePanes() || !layoutPaneIds().includes(paneId)) return false;
  setMaximizedPaneId(paneId);
  return true;
}

// Single mutation choke point: a layout change that drops the maximized pane
// clears the transient state instead of maximizing a stale id.
function commitLayout(node: LayoutNode) {
  const id = maximizedPaneId();
  if (id && !layoutPaneIds(node).includes(id)) setMaximizedPaneId(null);
  setLayoutRoot(node);
}

const [focusedPaneIdAccessor, writeFocusedPaneId] = createSignal("main");
export const focusedPaneId = focusedPaneIdAccessor;

export function setFocusedPaneId(paneId: string) {
  // Every focus route, including history/session adapters, must reveal the pane
  // it focuses. Keeping this at the state boundary prevents a hidden pane from
  // becoming the logical target while another pane remains maximized.
  if (maximizedPaneId() && maximizedPaneId() !== paneId) setMaximizedPaneId(null);
  if (focusedPaneId() !== paneId) {
    clearSelection();
    setCellSel(null);
  }
  writeFocusedPaneId(paneId);
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

export function activePaneRoutes(): Route[] {
  return layoutPaneIds().map((id) => paneRouter(id).route());
}

export function rewritePageTargetAcrossPanes(from: PageTarget, to: PageTarget) {
  for (const router of routers.values()) router.rewritePageTarget(from, to);
}

export function removePageTargetAcrossPanes(target: PageTarget) {
  for (const router of routers.values()) router.removePageTarget(target);
}

export function feedPaneId(): string | null {
  return layoutPaneIds().find((id) => paneRouter(id).route().kind === "journals") ?? null;
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
  const selectedDay = anchor ? doc.byId[anchor]?.page : undefined;
  const today = journalTitle(new Date());
  const name =
    (selectedDay && doc.feed.includes(selectedDay) ? selectedDay : undefined) ??
    (doc.feed.includes(today) ? today : doc.feed[0] ?? today);
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
  if (isMobilePlatform) return null;
  if (!layoutPaneIds().includes(paneId)) return null;
  const newPaneId = freshPaneId();
  const source = paneRouter(paneId);
  const router = paneRouter(newPaneId);
  router.restoreSnapshot(opts.snapshot ?? splitSnapshotForNewPane(source));
  commitLayout(splitLayoutNodeAt(layoutRoot(), paneId, dir, newPaneId, opts.position ?? "after"));
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
  opts: { focusNew?: boolean; snapshot?: PaneSnapshot } = {}
): string | null {
  const split = nodeAtPath(layoutRoot(), path);
  if (!split || split.kind === "pane") return null;
  const source = sourcePaneId && layoutPaneIds().includes(sourcePaneId) ? sourcePaneId : null;
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
  if (isMobilePlatform) return null;
  const ids = layoutPaneIds();
  const sourceId = ids.includes(sourcePaneId) ? sourcePaneId : ids[0];
  if (!sourceId) return null;
  const newPaneId = freshPaneId();
  paneRouter(newPaneId).restoreSnapshot(opts.snapshot ?? splitSnapshotForNewPane(paneRouter(sourceId)));
  const oldRoot = layoutRoot();
  const newLeaf: LayoutNode = { kind: "pane", paneId: newPaneId };
  const dir = side === "left" || side === "right" ? "row" : "col";
  const newFirst = side === "left" || side === "top";
  commitLayout({
    kind: "split",
    dir,
    ratio: 0.5,
    children: newFirst ? [newLeaf, oldRoot] : [oldRoot, newLeaf],
  });
  if (opts.focusNew !== false) focusPane(newPaneId);
  focusedRouter().scheduleSessionSave();
  return newPaneId;
}

export function closePane(paneId = focusedPaneId()): boolean {
  if (layoutPaneIds().length <= 1) return false;
  const closingFocusedPane = focusedPaneId() === paneId;
  const res = closeLayoutPane(layoutRoot(), paneId);
  if (!res.closed) return false;
  commitLayout(res.node);
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
  if (!layoutPaneIds().includes(paneId) || focusedPaneId() === paneId) return;
  setFocusedPaneId(paneId);
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
  const ids = layoutPaneIds();
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

// Directional "Move tab to pane" (GH #282): a real move when a pane lies that
// way, but with no neighbor the command grows the layout in the requested
// direction — the VS Code gesture a one-pane workflow uses to spawn its second
// pane. A multi-tab source donates its active tab into the new pane; a one-tab
// source cannot be emptied (Tine has no empty-pane route), so the new pane
// opens as a mirror of the current tab/history instead and the original stays.
export function moveActiveTabInDirection(sourcePaneId: string, dir: PaneDirection): string | null {
  if (!layoutPaneIds().includes(sourcePaneId)) return null;
  const target = nearestPaneInDirection(layoutRoot(), sourcePaneId, dir);
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
  const ids = layoutPaneIds();
  if (isMobilePlatform || !ids.includes(sourcePaneId) || !ids.includes(targetPaneId)) return null;
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
  const ids = layoutPaneIds();
  const split = nodeAtPath(layoutRoot(), path);
  if (isMobilePlatform || !ids.includes(sourcePaneId) || !split || split.kind === "pane") return null;
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
  const ids = layoutPaneIds();
  if (isMobilePlatform || !ids.includes(sourcePaneId)) return null;
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

export function setSplitRatio(path: number[], ratio: number) {
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
  commitLayout(update(layoutRoot(), 0));
  focusedRouter().scheduleSessionSave();
}

function panePath(node: LayoutNode, paneId: string, prefix: number[] = []): number[] | null {
  if (node.kind === "pane") return node.paneId === paneId ? prefix : null;
  return (
    panePath(node.children[0], paneId, [...prefix, 0]) ??
    panePath(node.children[1], paneId, [...prefix, 1])
  );
}

// Keyboard pane sizing (GH #286): nudge the NEAREST ancestor split of the
// matching axis by five percentage points so the pane's subtree enlarges or
// shrinks through it. setSplitRatio applies the 15–85% clamps; a pane whose
// ancestor chain has no split of that axis is a deliberate no-op.
export function adjustPaneSize(paneId: string, axis: "width" | "height", grow: boolean): boolean {
  const path = panePath(layoutRoot(), paneId);
  if (!path || path.length === 0) return false;
  const dir = axis === "width" ? "row" : "col";
  for (let depth = path.length - 1; depth >= 0; depth--) {
    const ancestor = nodeAtPath(layoutRoot(), path.slice(0, depth));
    if (!ancestor || ancestor.kind !== "split" || ancestor.dir !== dir) continue;
    const activeIsFirst = path[depth] === 0;
    const delta = (grow ? 0.05 : -0.05) * (activeIsFirst ? 1 : -1);
    setSplitRatio(path.slice(0, depth), ancestor.ratio + delta);
    return true;
  }
  return false;
}

export function openRouteInOtherPane(route: Route, sourcePaneId = focusedPaneId()): string | null {
  const ids = layoutPaneIds();
  let target = nearestPane(layoutRoot(), sourcePaneId) ?? ids.find((id) => id !== sourcePaneId) ?? null;
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
    else if (route.kind === "query") router.replaceActiveRoute(route);
    else if (route.kind === "pdf" || route.kind === "invalid") router.replaceActiveRoute(route);
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
  for (const paneId of layoutPaneIds()) {
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

/** Open a graph PDF as an ordinary route. Desktop preserves the source in a
 * reusable companion pane; mobile uses the current route history so hardware
 * Back returns to the link. Deliberate duplicates are a separate operation and
 * remain disabled until shared annotation mutation ownership lands. */
export function openPdf(
  filename: string,
  label: string,
  page?: number,
  highlightId?: string,
  options: OpenPdfOptions = {},
): PdfRoute | null {
  const sourcePaneId = options.sourcePaneId && layoutPaneIds().includes(options.sourcePaneId)
    ? options.sourcePaneId
    : focusedPaneId();
  const existing = options.anotherView ? null : pdfTab(filename);
  if (existing) {
    const router = paneRouter(existing.paneId);
    router.setActiveTab(existing.tabId);
    if (page !== undefined) router.updateActivePdfViewState({ page });
    publishPdfNavigationIntent(existing.route.viewId, { page, highlightId });
    if (!options.background) focusPane(existing.paneId);
    return existing.route;
  }

  // Do not expose editable duplicate views until the shared annotation session
  // and committed-merge backend reply exist.
  if (options.anotherView) return null;

  const route = makePdfRoute(filename, label, { page });
  publishPdfNavigationIntent(route.viewId, { page, highlightId });
  if (isMobilePlatform || options.inPlace) {
    paneRouter(sourcePaneId).openPdf(route);
    return route;
  }

  const companion = nearestPane(layoutRoot(), sourcePaneId)
    ?? layoutPaneIds().find((paneId) => paneId !== sourcePaneId)
    ?? null;
  if (companion) {
    paneRouter(companion).openInNewTab(route, !options.background);
    if (!options.background) focusPane(companion);
    else setFocusedPaneId(sourcePaneId);
    return route;
  }

  const created = splitPane(sourcePaneId, "row", {
    focusNew: !options.background,
    snapshot: {
      tabs: [{ history: [route], pos: 0, pinned: false }],
      activeIndex: 0,
      scrolls: [null],
    },
  });
  if (!created) return null;
  if (options.background) setFocusedPaneId(sourcePaneId);
  return route;
}

export function resetPaneLayoutToSingle(snapshot?: PaneSnapshot) {
  setMaximizedPaneId(null);
  setLayoutRoot({ kind: "pane", paneId: "main" });
  if (snapshot) mainRouter().restoreSnapshot(snapshot);
  for (const id of [...routers.keys()]) {
    if (id !== "main") routers.delete(id);
  }
  setFocusedPaneId("main");
}

export function restorePaneLayout(
  root: LayoutNode,
  snapshots: Map<string, PaneSnapshot>,
  focused = "main"
) {
  const ids = layoutPaneIds(root);
  for (const id of ids) {
    const snap = snapshots.get(id);
    if (snap) paneRouter(id).restoreSnapshot(snap);
    else paneRouter(id);
  }
  commitLayout(root);
  for (const id of [...routers.keys()]) {
    if (id !== "main" && !ids.includes(id)) routers.delete(id);
  }
  setFocusedPaneId(ids.includes(focused) ? focused : ids[0] ?? "main");
}

installPaneRouterRegistry({
  focusedRouter,
  mainRouter,
  routerForPane: (paneId) => routers.get(paneId),
  activatePane: (paneId) => {
    if (!layoutPaneIds().includes(paneId) || !routers.has(paneId)) return false;
    setFocusedPaneId(paneId);
    return true;
  },
});
installHistoryRouteContextAdapter(historyRouteContextAdapter);
installLastTabCloseHandler((paneId) => closePane(paneId));
installNavigationInterceptor((paneId, r) => {
  if (r.kind !== "journals") return false;
  const existing = feedPaneId();
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
registerPaneFocusSetter(focusPane);
