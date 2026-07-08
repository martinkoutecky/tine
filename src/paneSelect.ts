import { createSignal } from "solid-js";
import type { LayoutNode } from "./panes";

export type PaneEdgeSide = "left" | "right" | "top" | "bottom";

export type PaneTarget =
  | { kind: "pane"; paneId: string }
  | { kind: "seam"; path: number[] }
  // A single pane's boundary segment on the window edge: splitting it splits
  // ONLY that pane ("split the left half horizontally" — Martin's Jul 8 nit).
  | { kind: "pane-edge"; paneId: string; side: PaneEdgeSide }
  // The whole-window edge: splitting it splits the root layout.
  | { kind: "edge"; side: PaneEdgeSide };

export type PaneDirection = "left" | "right" | "up" | "down";

export interface PaneRect {
  paneId: string;
  rect: Rect;
}

export interface SeamRect {
  path: number[];
  dir: "row" | "col";
  rect: Rect;
}

export interface EdgeRect {
  side: PaneEdgeSide;
  rect: Rect;
}

export interface PaneEdgeRect {
  paneId: string;
  side: PaneEdgeSide;
  rect: Rect;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PaneGeometry {
  panes: PaneRect[];
  seams: SeamRect[];
  edges: EdgeRect[];
  paneEdges: PaneEdgeRect[];
}

const ROOT_RECT: Rect = { x: 0, y: 0, w: 1, h: 1 };
const EPS = 1e-9;

export const [paneSel, setPaneSel] = createSignal<PaneTarget | null>(null);

let previousPaneTarget: string | null = null;

function samePath(a: readonly number[], b: readonly number[]): boolean {
  return a.length === b.length && a.every((n, i) => n === b[i]);
}

export function samePaneTarget(a: PaneTarget | null, b: PaneTarget | null): boolean {
  if (!a || !b || a.kind !== b.kind) return false;
  if (a.kind === "pane") return a.paneId === (b as Extract<PaneTarget, { kind: "pane" }>).paneId;
  if (a.kind === "edge") return a.side === (b as Extract<PaneTarget, { kind: "edge" }>).side;
  if (a.kind === "pane-edge") {
    const bb = b as Extract<PaneTarget, { kind: "pane-edge" }>;
    return a.paneId === bb.paneId && a.side === bb.side;
  }
  return samePath(a.path, (b as Extract<PaneTarget, { kind: "seam" }>).path);
}

export function computePaneGeometry(root: LayoutNode, rect: Rect = ROOT_RECT): PaneGeometry {
  const panes: PaneRect[] = [];
  const seams: SeamRect[] = [];

  const walk = (node: LayoutNode, box: Rect, path: number[]) => {
    if (node.kind === "pane") {
      panes.push({ paneId: node.paneId, rect: box });
      return;
    }

    if (node.dir === "row") {
      const leftW = box.w * node.ratio;
      const seamX = box.x + leftW;
      seams.push({ path, dir: node.dir, rect: { x: seamX, y: box.y, w: 0, h: box.h } });
      walk(node.children[0], { x: box.x, y: box.y, w: leftW, h: box.h }, [...path, 0]);
      walk(node.children[1], { x: seamX, y: box.y, w: box.w - leftW, h: box.h }, [...path, 1]);
      return;
    }

    const topH = box.h * node.ratio;
    const seamY = box.y + topH;
    seams.push({ path, dir: node.dir, rect: { x: box.x, y: seamY, w: box.w, h: 0 } });
    walk(node.children[0], { x: box.x, y: box.y, w: box.w, h: topH }, [...path, 0]);
    walk(node.children[1], { x: box.x, y: seamY, w: box.w, h: box.h - topH }, [...path, 1]);
  };

  walk(root, rect, []);

  // A pane's side lying ON the window boundary is its own target (splitting it
  // splits just that pane); sides on internal boundaries are covered by seams.
  const paneEdges: PaneEdgeRect[] = [];
  for (const p of panes) {
    const r = p.rect;
    if (Math.abs(r.x - rect.x) < EPS)
      paneEdges.push({ paneId: p.paneId, side: "left", rect: { x: r.x, y: r.y, w: 0, h: r.h } });
    if (Math.abs(r.x + r.w - (rect.x + rect.w)) < EPS)
      paneEdges.push({ paneId: p.paneId, side: "right", rect: { x: r.x + r.w, y: r.y, w: 0, h: r.h } });
    if (Math.abs(r.y - rect.y) < EPS)
      paneEdges.push({ paneId: p.paneId, side: "top", rect: { x: r.x, y: r.y, w: r.w, h: 0 } });
    if (Math.abs(r.y + r.h - (rect.y + rect.h)) < EPS)
      paneEdges.push({ paneId: p.paneId, side: "bottom", rect: { x: r.x, y: r.y + r.h, w: r.w, h: 0 } });
  }

  return {
    panes,
    seams,
    paneEdges,
    edges: [
      { side: "left", rect: { x: rect.x, y: rect.y, w: 0, h: rect.h } },
      { side: "right", rect: { x: rect.x + rect.w, y: rect.y, w: 0, h: rect.h } },
      { side: "top", rect: { x: rect.x, y: rect.y, w: rect.w, h: 0 } },
      { side: "bottom", rect: { x: rect.x, y: rect.y + rect.h, w: rect.w, h: 0 } },
    ],
  };
}

function center(rect: Rect): { x: number; y: number } {
  return { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 };
}

function targetRect(geom: PaneGeometry, target: PaneTarget): Rect | null {
  switch (target.kind) {
    case "pane":
      return geom.panes.find((p) => p.paneId === target.paneId)?.rect ?? null;
    case "seam":
      return geom.seams.find((s) => samePath(s.path, target.path))?.rect ?? null;
    case "pane-edge":
      return geom.paneEdges.find((e) => e.paneId === target.paneId && e.side === target.side)?.rect ?? null;
    case "edge":
      return geom.edges.find((e) => e.side === target.side)?.rect ?? null;
  }
}

function allTargets(geom: PaneGeometry): PaneTarget[] {
  return [
    ...geom.panes.map((p): PaneTarget => ({ kind: "pane", paneId: p.paneId })),
    ...geom.seams.map((s): PaneTarget => ({ kind: "seam", path: s.path })),
    ...geom.paneEdges.map((e): PaneTarget => ({ kind: "pane-edge", paneId: e.paneId, side: e.side })),
    ...geom.edges.map((e): PaneTarget => ({ kind: "edge", side: e.side })),
  ];
}

function isAhead(from: { x: number; y: number }, to: { x: number; y: number }, dir: PaneDirection): boolean {
  switch (dir) {
    case "left": return to.x < from.x - EPS;
    case "right": return to.x > from.x + EPS;
    case "up": return to.y < from.y - EPS;
    case "down": return to.y > from.y + EPS;
  }
}

function primaryDistance(from: { x: number; y: number }, to: { x: number; y: number }, dir: PaneDirection): number {
  return dir === "left" || dir === "right" ? Math.abs(to.x - from.x) : Math.abs(to.y - from.y);
}

function crossDistance(from: { x: number; y: number }, to: { x: number; y: number }, dir: PaneDirection): number {
  return dir === "left" || dir === "right" ? Math.abs(to.y - from.y) : Math.abs(to.x - from.x);
}

function targetRank(t: PaneTarget): number {
  if (t.kind === "seam") return 0;
  if (t.kind === "pane-edge") return 1;
  if (t.kind === "pane") return 2;
  return 3;
}

// Cross-axis interval overlap: stepping down from a pane must only consider
// targets that actually lie below IT (share horizontal extent), not whatever
// center happens to be nearest — without this, ArrowDown from a tall right
// pane selected the seam between the two LEFT panes (Martin's Jul 8 report).
// Degenerate (zero-length) intervals count via closed containment.
function crossSpan(rect: Rect, dir: PaneDirection): [number, number] {
  return dir === "left" || dir === "right" ? [rect.y, rect.y + rect.h] : [rect.x, rect.x + rect.w];
}

function spansOverlap(a: [number, number], b: [number, number]): boolean {
  const aDeg = a[1] - a[0] < EPS;
  const bDeg = b[1] - b[0] < EPS;
  if (aDeg && bDeg) return Math.abs(a[0] - b[0]) < EPS;
  if (aDeg) return b[0] - EPS <= a[0] && a[0] <= b[1] + EPS;
  if (bDeg) return a[0] - EPS <= b[0] && b[0] <= a[1] + EPS;
  return Math.min(a[1], b[1]) - Math.max(a[0], b[0]) > EPS;
}

function resolveTarget(geom: PaneGeometry, target: PaneTarget | null): PaneTarget {
  if (target && targetRect(geom, target)) return target;
  return geom.panes[0] ? { kind: "pane", paneId: geom.panes[0].paneId } : { kind: "edge", side: "left" };
}

export function stepPaneTarget(root: LayoutNode, target: PaneTarget | null, dir: PaneDirection): PaneTarget {
  const geom = computePaneGeometry(root);
  const current = resolveTarget(geom, target);
  const currentRect = targetRect(geom, current);
  if (!currentRect) return current;

  // Pressing INTO a pane-edge segment again widens it to the whole-window edge
  // (TreeSheets-style: the segment splits one pane, the full edge splits the
  // root). The global edge sits at the same coordinate, so distance-stepping
  // could never reach it.
  const sideForDir: Record<PaneDirection, PaneEdgeSide> = { left: "left", right: "right", up: "top", down: "bottom" };
  if (current.kind === "pane-edge" && current.side === sideForDir[dir]) {
    // ...unless the segment already spans the whole edge (solo pane / full-
    // height side pane): splitting either is identical, skip the ghost rung.
    const edgeRect = geom.edges.find((e) => e.side === current.side)?.rect;
    const seg = crossSpan(currentRect, dir);
    const full = edgeRect ? crossSpan(edgeRect, dir) : null;
    if (!full || Math.abs(seg[0] - full[0]) > EPS || Math.abs(seg[1] - full[1]) > EPS) {
      return { kind: "edge", side: current.side };
    }
    return current;
  }

  const from = center(currentRect);
  const fromSpan = crossSpan(currentRect, dir);
  const candidates = allTargets(geom)
    .filter((candidate) => !samePaneTarget(candidate, current))
    .map((candidate) => ({ candidate, rect: targetRect(geom, candidate) }))
    .filter((x): x is { candidate: PaneTarget; rect: Rect } => !!x.rect)
    .map((x) => ({ ...x, c: center(x.rect) }))
    .filter((x) => isAhead(from, x.c, dir))
    .filter((x) => spansOverlap(fromSpan, crossSpan(x.rect, dir)))
    .sort((a, b) => {
      const ap = primaryDistance(from, a.c, dir);
      const bp = primaryDistance(from, b.c, dir);
      if (Math.abs(ap - bp) > EPS) return ap - bp;
      const ac = crossDistance(from, a.c, dir);
      const bc = crossDistance(from, b.c, dir);
      if (Math.abs(ac - bc) > EPS) return ac - bc;
      return targetRank(a.candidate) - targetRank(b.candidate);
    });
  return candidates[0]?.candidate ?? current;
}

export function readingOrderPanes(root: LayoutNode): PaneRect[] {
  return [...computePaneGeometry(root).panes].sort((a, b) => {
    const ay = a.rect.y + a.rect.h / 2;
    const by = b.rect.y + b.rect.h / 2;
    if (Math.abs(ay - by) > EPS) return ay - by;
    const ax = a.rect.x + a.rect.w / 2;
    const bx = b.rect.x + b.rect.w / 2;
    if (Math.abs(ax - bx) > EPS) return ax - bx;
    return a.paneId.localeCompare(b.paneId);
  });
}

export function nearestPane(root: LayoutNode, sourcePaneId: string, exclude = sourcePaneId): string | null {
  const geom = computePaneGeometry(root);
  const source = geom.panes.find((p) => p.paneId === sourcePaneId) ?? geom.panes[0];
  if (!source) return null;
  const from = center(source.rect);
  const candidates = geom.panes
    .filter((p) => p.paneId !== exclude)
    .map((p) => {
      const c = center(p.rect);
      const dx = c.x - from.x;
      const dy = c.y - from.y;
      return { paneId: p.paneId, distance: dx * dx + dy * dy };
    })
    .sort((a, b) => a.distance - b.distance || a.paneId.localeCompare(b.paneId));
  return candidates[0]?.paneId ?? null;
}

export function nearestPaneInDirection(root: LayoutNode, sourcePaneId: string, dir: PaneDirection): string | null {
  const geom = computePaneGeometry(root);
  const source = geom.panes.find((p) => p.paneId === sourcePaneId);
  if (!source) return null;
  const from = center(source.rect);
  const candidates = geom.panes
    .filter((p) => p.paneId !== sourcePaneId)
    .map((p) => ({ paneId: p.paneId, c: center(p.rect) }))
    .filter((p) => isAhead(from, p.c, dir))
    .sort((a, b) => {
      const ap = primaryDistance(from, a.c, dir);
      const bp = primaryDistance(from, b.c, dir);
      if (Math.abs(ap - bp) > EPS) return ap - bp;
      const ac = crossDistance(from, a.c, dir);
      const bc = crossDistance(from, b.c, dir);
      if (Math.abs(ac - bc) > EPS) return ac - bc;
      return a.paneId.localeCompare(b.paneId);
    });
  return candidates[0]?.paneId ?? null;
}

export function enterPaneSelect(paneId: string): void {
  previousPaneTarget = paneId;
  setPaneSel({ kind: "pane", paneId });
}

export function exitPaneSelect(): void {
  setPaneSel(null);
}

export function previousPaneSelectionTarget(): string | null {
  return previousPaneTarget;
}

export function movePaneSelection(root: LayoutNode, dir: PaneDirection): PaneTarget {
  const current = paneSel();
  if (current?.kind === "pane") previousPaneTarget = current.paneId;
  const next = stepPaneTarget(root, current, dir);
  if (next.kind === "pane") previousPaneTarget = next.paneId;
  setPaneSel(next);
  return next;
}
