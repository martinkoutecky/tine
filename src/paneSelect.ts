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

  // EVERY side of every pane is a target — splitting it splits just that pane
  // on that side, at that pane's extent. Internal sides matter too: in a
  // 3-column layout the middle-top pane's right side splits ONLY that pane
  // (its height), while the seam at the same coordinate inserts a full-height
  // column (Martin's Jul 8 follow-up #2). Where a segment coincides exactly
  // with a seam, the seam shadows it by rank — same split either way.
  const paneEdges: PaneEdgeRect[] = [];
  for (const p of panes) {
    const r = p.rect;
    paneEdges.push({ paneId: p.paneId, side: "left", rect: { x: r.x, y: r.y, w: 0, h: r.h } });
    paneEdges.push({ paneId: p.paneId, side: "right", rect: { x: r.x + r.w, y: r.y, w: 0, h: r.h } });
    paneEdges.push({ paneId: p.paneId, side: "top", rect: { x: r.x, y: r.y, w: r.w, h: 0 } });
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

// A step must land at-or-beyond the CURRENT TARGET'S boundary in the pressed
// direction, not merely beyond its center: a perpendicular window edge whose
// center is barely "ahead" of an off-center pane's center otherwise wins with
// a near-zero distance (Martin's Jul 8 report #3: ArrowRight from a pane left
// of window-center selected the global TOP edge).
function beyondLeadingBoundary(rect: Rect, c: { x: number; y: number }, dir: PaneDirection): boolean {
  switch (dir) {
    case "left": return c.x <= rect.x + EPS;
    case "right": return c.x >= rect.x + rect.w - EPS;
    case "up": return c.y <= rect.y + EPS;
    case "down": return c.y >= rect.y + rect.h - EPS;
  }
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

  // Pressing INTO a pane-edge segment again WIDENS the split scope at the
  // same coordinate (TreeSheets' "edge of the left half, then the full
  // edge"): pane side → a covering seam (splits across the whole boundary) →
  // the whole-window edge (root split). Same-position-but-wider targets are
  // unreachable by distance-stepping, so this ladder is the only way in.
  // A seam must be STRICTLY wider (an exactly-coinciding seam is the same
  // split); the window edge counts even at equal extent (nesting in the pane
  // vs splitting the root differ) — except on a true solo pane, where the
  // trees are identical either way.
  const sideForDir: Record<PaneDirection, PaneEdgeSide> = { left: "left", right: "right", up: "top", down: "bottom" };
  if (current.kind === "pane-edge" && current.side === sideForDir[dir]) {
    const horiz = dir === "left" || dir === "right";
    const coord = horiz ? currentRect.x : currentRect.y;
    const span = crossSpan(currentRect, dir);
    const atCoord = (r: Rect) => Math.abs((horiz ? r.x : r.y) - coord) < EPS;
    const coversStrictlyWider = (r: Rect) => {
      const s = crossSpan(r, dir);
      return s[0] <= span[0] + EPS && s[1] >= span[1] - EPS && s[1] - s[0] > span[1] - span[0] + EPS;
    };
    const seam = geom.seams.find((s) => atCoord(s.rect) && coversStrictlyWider(s.rect));
    if (seam) return { kind: "seam", path: seam.path };
    const edge = geom.edges.find((e) => e.side === current.side && atCoord(e.rect));
    if (edge && root.kind !== "pane") return { kind: "edge", side: current.side };
    return current;
  }

  // LATERAL movement along an edge (TreeSheets, Martin's Jul 8 follow-up #3):
  // a perpendicular arrow on an edge segment slides to the ADJACENT pane's
  // segment on the same side of the same line — "the top edge of the next
  // column over" — instead of diving to the current pane's own perpendicular
  // side. Falls through to generic stepping at the end of the line (which
  // naturally turns the corner).
  if (current.kind === "pane-edge") {
    const horizSide = current.side === "top" || current.side === "bottom";
    const isLateral = horizSide ? dir === "left" || dir === "right" : dir === "up" || dir === "down";
    if (isLateral) {
      const lineCoord = horizSide ? currentRect.y : currentRect.x;
      const c0 = center(currentRect);
      const along = geom.paneEdges
        .filter((e) => e.side === current.side && e.paneId !== current.paneId)
        .filter((e) => Math.abs((horizSide ? e.rect.y : e.rect.x) - lineCoord) < EPS)
        .map((e) => ({ e, c: center(e.rect) }))
        .filter((x) => isAhead(c0, x.c, dir))
        .sort((a, b) => primaryDistance(c0, a.c, dir) - primaryDistance(c0, b.c, dir));
      if (along[0]) return { kind: "pane-edge", paneId: along[0].e.paneId, side: current.side };
    }
  }

  const from = center(currentRect);
  const fromSpan = crossSpan(currentRect, dir);
  const candidates = allTargets(geom)
    .filter((candidate) => !samePaneTarget(candidate, current))
    .map((candidate) => ({ candidate, rect: targetRect(geom, candidate) }))
    .filter((x): x is { candidate: PaneTarget; rect: Rect } => !!x.rect)
    .map((x) => ({ ...x, c: center(x.rect) }))
    .filter((x) => isAhead(from, x.c, dir))
    .filter((x) => beyondLeadingBoundary(currentRect, x.c, dir))
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
