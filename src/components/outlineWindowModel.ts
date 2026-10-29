import { createMemo, createSignal, onCleanup, untrack } from "solid-js";
import { node, onBlockRawChanged } from "../document";
import { clearOnBindingInvalidated } from "../binding";

/** Makes one pass's fold answer: `(id, page, stored) => folded`. */
type Folds = () => (id: string, page: string, stored: boolean) => boolean;

/** Membership observes structure alone. Hidden descendants still participate in
 * reveal registration, but never contribute to the spacer's estimated height. */
export function createOutlineWindowModel(ids: () => readonly string[], folds: Folds) {
  return createMemo(() => {
    const collapsed = folds();
    const all: string[] = [];
    const rows: { id: string; depth: number; visible: boolean }[] = [];
    const structure: string[] = [];
    const visit = (id: string, depth: number, visible: boolean) => {
      const n = node(id);
      if (!n) return;
      const folded = collapsed(id, n.page, n.collapsed);
      all.push(id);
      rows.push({ id, depth, visible });
      structure.push(id, String(depth), String(folded));
      n.children.forEach(child => visit(child, depth + 1, visible && !folded));
    };
    ids().forEach(id => visit(id, 0, true));
    return { all, members: new Set(all), rows, structure: structure.join("\0") };
  });
}

// Node-scoped monotonic content versions let remounted windows validate cached
// measurements without retaining document nodes, hashing text, or joining the
// whole group's text on an edit. Overlapping windows share the same version.
const versions = new WeakMap<object, { raw: string; revision: number }>();
let revision = 0;
function contentVersion(n: object, raw: string): number {
  const previous = versions.get(n);
  if (previous?.raw === raw) return previous.revision;
  const next = ++revision;
  versions.set(n, { raw, revision: next });
  return next;
}

/** One pass per structure/font/width change, then one raw-change listener per
 * window (BIGLOAD-090b): an edit adjusts one height contribution and one
 * version, so work scales with the edited text, never the number of rows.
 * The listener goes with this memo's owner, so stale geometry stops updating.
 * No document writes. */
export function createOutlineWindowGeometry(
  model: ReturnType<typeof createOutlineWindowModel>, width: () => number, font: () => string,
) {
  return createMemo(() => {
    const current = model(), w = width(), f = font();
    const rowOf = new Map<string, number>();
    const heights: number[] = [];
    const height = (i: number, raw: string) => current.rows[i].visible ? estimateRowHeight(raw, w - current.rows[i].depth * 20, f) : 0;
    let total = 0, top = 0;
    untrack(() => current.rows.forEach((row, i) => {
      rowOf.set(row.id, i);
      const n = node(row.id);
      heights[i] = n ? height(i, n.raw) : 0;
      total += heights[i];
      if (n) top = Math.max(top, contentVersion(n, n.raw));
    }));
    const [estimate, setEstimate] = createSignal(total);
    const [version, setVersion] = createSignal(top);
    onCleanup(onBlockRawChanged((id) => {
      const i = rowOf.get(id), n = node(id);
      if (i === undefined || !n) return;
      const next = height(i, n.raw);
      if (next !== heights[i]) setEstimate(value => value + next - heights[i]);
      heights[i] = next;
      const nextVersion = contentVersion(n, n.raw);
      setVersion(value => Math.max(value, nextVersion));
    }));
    return { estimate, version };
  });
}

// Text layout estimate only; never infer Logseq structure. Samples are bounded
// and shared; measured window heights replace estimates on first visit.
const textWidths = new Map<string, number>();
clearOnBindingInvalidated(() => textWidths.clear());
let measuringFont = "";
let canvas: CanvasRenderingContext2D | null | undefined;
function estimateRowHeight(raw: string, width: number, font: string): number {
  if (canvas === undefined) canvas = typeof CanvasRenderingContext2D === "undefined" ? null : document.createElement("canvas").getContext("2d");
  if (font !== measuringFont) {
    measuringFont = font; textWidths.clear();
    if (canvas) canvas.font = font;
  }
  const available = Math.max(80, width - 42);
  let lines = 0;
  for (const line of raw.split("\n")) {
    const sample = line.slice(0, 160);
    let size = textWidths.get(sample);
    if (size === undefined) {
      size = canvas?.measureText(sample).width ?? sample.length * 8;
      if (textWidths.size >= 4096) textWidths.clear();
      textWidths.set(sample, size);
    }
    const length = sample.length ? size * line.length / sample.length : 0;
    lines += Math.max(1, Math.ceil(length / available));
  }
  return Math.max(1, lines) * 26 + 2;
}
