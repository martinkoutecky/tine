import { createSignal } from "solid-js";
import { PaneContext, type PaneContextValue } from "./paneContext";
import {
  createPaneRouter,
  installPaneRouterRegistry,
  installLastTabCloseHandler,
  installNavigationInterceptor,
  mainPaneRouter,
  tabRoute,
  type PaneSnapshot,
  type PaneRouter,
  type Route,
} from "./router";
import { registerPaneFocusSetter } from "./ui";
import { doc, pageByName, registerPaneRouteProvider } from "./store";
import { journalTitle } from "./journal";
import { isMobilePlatform } from "./nativeChrome";

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
export const [focusedPaneId, setFocusedPaneId] = createSignal("main");

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
  return replacePaneInLayout(node, paneId, {
    kind: "split",
    dir,
    ratio: 0.5,
    children: [
      { kind: "pane", paneId },
      { kind: "pane", paneId: newPaneId },
    ],
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

function routeForJournalsDuplicate(source: PaneSnapshot): Route {
  const active = source.tabs[Math.min(Math.max(0, source.activeIndex | 0), source.tabs.length - 1)];
  for (let i = active.pos - 1; i >= 0; i--) {
    const r = active.history[i];
    if (r?.kind === "page") return r;
  }
  for (const r of active.history) {
    if (r?.kind === "page") return r;
  }
  const name = doc.feed[0] ?? journalTitle(new Date());
  return { kind: "page", name, pageKind: pageByName(name)?.kind ?? "journal" };
}

function splitSnapshotForNewPane(source: PaneRouter): PaneSnapshot {
  const snap = source.duplicateActiveSnapshot();
  const active = snap.tabs[0];
  if (active && tabRoute({ id: "snapshot", history: active.history, pos: active.pos, pinned: active.pinned }).kind === "journals") {
    active.history = [routeForJournalsDuplicate(snap)];
    active.pos = 0;
  }
  return snap;
}

export function splitPane(
  paneId = focusedPaneId(),
  dir: "row" | "col" = "row",
  opts: { focusNew?: boolean } = {}
): string | null {
  if (isMobilePlatform) return null;
  if (!layoutPaneIds().includes(paneId)) return null;
  const newPaneId = freshPaneId();
  const source = paneRouter(paneId);
  const router = paneRouter(newPaneId);
  router.restoreSnapshot(splitSnapshotForNewPane(source));
  setLayoutRoot(splitLayoutNode(layoutRoot(), paneId, dir, newPaneId));
  if (opts.focusNew !== false) setFocusedPaneId(newPaneId);
  focusedRouter().scheduleSessionSave();
  return newPaneId;
}

export function closePane(paneId = focusedPaneId()): boolean {
  if (layoutPaneIds().length <= 1) return false;
  const res = closeLayoutPane(layoutRoot(), paneId);
  if (!res.closed) return false;
  setLayoutRoot(res.node);
  if (paneId !== "main") routers.delete(paneId);
  setFocusedPaneId(res.focusedPaneId);
  focusedRouter().scheduleSessionSave();
  return true;
}

export function focusPane(paneId: string) {
  if (layoutPaneIds().includes(paneId)) setFocusedPaneId(paneId);
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
  setLayoutRoot(update(layoutRoot(), 0));
  focusedRouter().scheduleSessionSave();
}

export function openRouteInOtherPane(route: Route, sourcePaneId = focusedPaneId()): string | null {
  const ids = layoutPaneIds();
  let target = ids.find((id) => id !== sourcePaneId) ?? null;
  if (!target) target = splitPane(sourcePaneId, "row", { focusNew: false });
  if (!target) return null;
  const router = paneRouter(target);
  router.openInNewTab(route, true);
  setFocusedPaneId(sourcePaneId);
  return target;
}

export function resetPaneLayoutToSingle(snapshot?: PaneSnapshot) {
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
  setLayoutRoot(root);
  for (const id of [...routers.keys()]) {
    if (id !== "main" && !ids.includes(id)) routers.delete(id);
  }
  setFocusedPaneId(ids.includes(focused) ? focused : ids[0] ?? "main");
}

installPaneRouterRegistry({ focusedRouter, mainRouter });
installLastTabCloseHandler((paneId) => closePane(paneId));
installNavigationInterceptor((paneId, r) => {
  if (r.kind !== "journals") return false;
  const existing = feedPaneId();
  if (existing && existing !== paneId) {
    setFocusedPaneId(existing);
    return true;
  }
  return false;
});
registerPaneRouteProvider(activePaneRoutes);
registerPaneFocusSetter(setFocusedPaneId);
