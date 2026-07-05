import { batch, createRoot, createSignal } from "solid-js";
import { doc, mainPages, setDoc } from "./store";
import { renderedBlockText, type RenderedTextOptions } from "./render/renderedText";
import { renderedBlocks } from "./lazyObserve";
import type { Format } from "./types";

export interface InPageFindMatch {
  blockId: string;
  ordinalInBlock: number;
  start: number;
  end: number;
}

export interface InPageFindBlock {
  id: string;
  raw: string;
  children: InPageFindBlock[];
}

const FIND_HIGHLIGHT = "tine-find";
const FIND_ACTIVE_HIGHLIGHT = "tine-find-active";

const renderedTextOptions: RenderedTextOptions = {
  typographicGlyphs: false,
  stripLinks: false,
  removeTags: false,
  removeProperties: false,
};

const state = createRoot(() => {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [activeIndex, setActiveIndex] = createSignal(-1);
  const [focusRequest, setFocusRequest] = createSignal(0);
  const [preserveEditorBlur, setPreserveEditorBlur] = createSignal(false);
  return {
    open, setOpen,
    query, setQuery,
    activeIndex, setActiveIndex,
    focusRequest, setFocusRequest,
    preserveEditorBlur, setPreserveEditorBlur,
  };
});

let restoreFocusEl: HTMLElement | null = null;
let revealToken = 0;
let overlayRoot: HTMLDivElement | null = null;

export const inPageFindOpen = state.open;
export const inPageFindQuery = state.query;
export const inPageFindActiveIndex = state.activeIndex;
export const inPageFindFocusRequest = state.focusRequest;

export function inPageFindPreservesEditorBlur(): boolean {
  return state.preserveEditorBlur();
}

export function findTextOccurrences(text: string, query: string): { start: number; end: number }[] {
  if (!query) return [];
  const haystack = text.toLocaleLowerCase();
  const needle = query.toLocaleLowerCase();
  const out: { start: number; end: number }[] = [];
  let from = 0;
  while (from <= haystack.length) {
    const idx = haystack.indexOf(needle, from);
    if (idx === -1) break;
    out.push({ start: idx, end: idx + query.length });
    from = idx + Math.max(needle.length, 1);
  }
  return out;
}

export function collectInPageFindMatches(
  blocks: readonly InPageFindBlock[],
  query: string,
  format: Format = "md",
): InPageFindMatch[] {
  const q = query.trim();
  if (!q) return [];
  const out: InPageFindMatch[] = [];
  const walk = (bs: readonly InPageFindBlock[]) => {
    for (const b of bs) {
      const text = renderedBlockText(b.raw, format, renderedTextOptions);
      findTextOccurrences(text, q).forEach((m, ordinalInBlock) => {
        out.push({ blockId: b.id, ordinalInBlock, start: m.start, end: m.end });
      });
      walk(b.children);
    }
  };
  walk(blocks);
  return out;
}

function currentMatchesFor(query: string): InPageFindMatch[] {
  const q = query.trim();
  if (!q || !doc.loaded) return [];
  const out: InPageFindMatch[] = [];
  const walk = (ids: readonly string[], format: Format) => {
    for (const id of ids) {
      const n = doc.byId[id];
      if (!n) continue;
      const text = renderedBlockText(n.raw, format, renderedTextOptions);
      findTextOccurrences(text, q).forEach((m, ordinalInBlock) => {
        out.push({ blockId: id, ordinalInBlock, start: m.start, end: m.end });
      });
      walk(n.children, format);
    }
  };
  for (const p of mainPages()) walk(p.roots, p.format);
  return out;
}

export function inPageFindMatches(): InPageFindMatch[] {
  return currentMatchesFor(state.query());
}

export function openInPageFind() {
  if (typeof document !== "undefined") restoreFocusEl = document.activeElement as HTMLElement | null;
  batch(() => {
    state.setPreserveEditorBlur(true);
    state.setOpen(true);
    state.setFocusRequest((n) => n + 1);
  });
  if (state.query().trim()) activateInPageFindIndex(Math.max(0, state.activeIndex()));
}

export function closeInPageFind(opts: { restoreFocus?: boolean } = {}) {
  const restoreFocus = opts.restoreFocus !== false;
  const target = restoreFocusEl;
  restoreFocusEl = null;
  batch(() => {
    state.setOpen(false);
    state.setActiveIndex(-1);
  });
  clearInPageFindHighlights();
  if (!restoreFocus) {
    state.setPreserveEditorBlur(false);
    return;
  }
  queueMicrotask(() => {
    const fallback = document.querySelector(".block-editor") as HTMLElement | null;
    const el = target?.isConnected ? target : fallback;
    el?.focus?.({ preventScroll: true });
    state.setPreserveEditorBlur(false);
  });
}

export function setInPageFindQuery(query: string) {
  state.setQuery(query);
  const matches = currentMatchesFor(query);
  if (!matches.length) {
    state.setActiveIndex(-1);
    clearInPageFindHighlights();
    return;
  }
  activateInPageFindIndex(0, matches);
}

export function stepInPageFind(delta: 1 | -1) {
  const matches = inPageFindMatches();
  if (!matches.length) return;
  const cur = state.activeIndex();
  const base = cur >= 0 ? cur : delta > 0 ? -1 : 0;
  activateInPageFindIndex(base + delta, matches);
}

export function activateInPageFindIndex(index: number, matches = inPageFindMatches()) {
  if (!matches.length) {
    state.setActiveIndex(-1);
    clearInPageFindHighlights();
    return;
  }
  const next = ((index % matches.length) + matches.length) % matches.length;
  state.setActiveIndex(next);
  const token = ++revealToken;
  void revealInPageFindMatch(matches[next]).then(() => {
    if (token === revealToken) refreshInPageFindHighlights();
  });
}

function expandAncestorsForFind(blockId: string) {
  let parent = doc.byId[blockId]?.parent ?? null;
  while (parent !== null) {
    const n = doc.byId[parent];
    if (!n) return;
    if (n.collapsed) setDoc("byId", parent, "collapsed", false);
    parent = n.parent;
  }
}

function blockSelector(id: string): string {
  const esc = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id.replace(/"/g, '\\"');
  return `.ls-block[data-block-id="${esc}"]`;
}

function blockElement(id: string): HTMLElement | null {
  return document.querySelector(blockSelector(id)) as HTMLElement | null;
}

export async function revealInPageFindMatch(match: InPageFindMatch): Promise<boolean> {
  const node = doc.byId[match.blockId];
  if (!node) return false;
  renderedBlocks.add(match.blockId);
  expandAncestorsForFind(match.blockId);
  for (let i = 0; i < 20; i++) {
    await new Promise((r) => requestAnimationFrame(r));
    const el = blockElement(match.blockId);
    if (el) {
      el.scrollIntoView({ block: "center", behavior: "smooth" });
      return true;
    }
  }
  return false;
}

interface TextPart {
  node: Text;
  start: number;
  end: number;
}

function textRanges(root: HTMLElement, query: string): Range[] {
  const q = query.trim();
  if (!q) return [];
  const parts: TextPart[] = [];
  let text = "";
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const value = node.textContent ?? "";
      if (!value) return NodeFilter.FILTER_REJECT;
      const parent = node.parentElement;
      if (!parent || parent.closest("button,input,textarea,select")) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  let node = walker.nextNode() as Text | null;
  while (node) {
    const value = node.textContent ?? "";
    parts.push({ node, start: text.length, end: text.length + value.length });
    text += value;
    node = walker.nextNode() as Text | null;
  }
  const pointForStart = (offset: number): [Text, number] | null => {
    for (const p of parts) if (offset >= p.start && offset < p.end) return [p.node, offset - p.start];
    return null;
  };
  const pointForEnd = (offset: number): [Text, number] | null => {
    for (const p of parts) if (offset > p.start && offset <= p.end) return [p.node, offset - p.start];
    return null;
  };
  const ranges: Range[] = [];
  for (const m of findTextOccurrences(text, q)) {
    const a = pointForStart(m.start);
    const b = pointForEnd(m.end);
    if (!a || !b) continue;
    const range = document.createRange();
    range.setStart(a[0], a[1]);
    range.setEnd(b[0], b[1]);
    ranges.push(range);
  }
  return ranges;
}

function clearOverlayHighlights() {
  overlayRoot?.remove();
  overlayRoot = null;
}

function applyOverlayHighlights(ranges: Range[], active: Range | null) {
  clearOverlayHighlights();
  overlayRoot = document.createElement("div");
  overlayRoot.className = "inpage-find-overlays";
  document.body.appendChild(overlayRoot);
  const add = (range: Range, activeRange: boolean) => {
    for (const rect of Array.from(range.getClientRects())) {
      if (rect.width <= 0 || rect.height <= 0) continue;
      const el = document.createElement("div");
      el.className = activeRange ? "inpage-find-overlay active" : "inpage-find-overlay";
      el.style.left = `${rect.left}px`;
      el.style.top = `${rect.top}px`;
      el.style.width = `${rect.width}px`;
      el.style.height = `${rect.height}px`;
      overlayRoot!.appendChild(el);
    }
  };
  ranges.forEach((r) => add(r, false));
  if (active) add(active, true);
}

function applyCssHighlights(ranges: Range[], active: Range | null): boolean {
  if (typeof CSS === "undefined") return false;
  const registry = (CSS as unknown as { highlights?: Map<string, unknown> }).highlights as any;
  const HighlightCtor = (window as unknown as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight;
  if (!registry || !HighlightCtor) return false;
  registry.delete(FIND_HIGHLIGHT);
  registry.delete(FIND_ACTIVE_HIGHLIGHT);
  if (ranges.length) registry.set(FIND_HIGHLIGHT, new HighlightCtor(...ranges));
  if (active) registry.set(FIND_ACTIVE_HIGHLIGHT, new HighlightCtor(active));
  clearOverlayHighlights();
  return true;
}

export function clearInPageFindHighlights() {
  if (typeof CSS !== "undefined") {
    ((CSS as unknown as { highlights?: Map<string, unknown> }).highlights as any)?.delete?.(FIND_HIGHLIGHT);
    ((CSS as unknown as { highlights?: Map<string, unknown> }).highlights as any)?.delete?.(FIND_ACTIVE_HIGHLIGHT);
  }
  clearOverlayHighlights();
  document.querySelectorAll(".inpage-find-active-block").forEach((el) => el.classList.remove("inpage-find-active-block"));
}

export function refreshInPageFindHighlights() {
  if (!state.open() || typeof document === "undefined") {
    clearInPageFindHighlights();
    return;
  }
  document.querySelectorAll(".inpage-find-active-block").forEach((el) => el.classList.remove("inpage-find-active-block"));
  const query = state.query().trim();
  if (!query) {
    clearInPageFindHighlights();
    return;
  }
  const matches = inPageFindMatches();
  const activeIdx = state.activeIndex();
  const ranges: Range[] = [];
  let activeRange: Range | null = null;
  const rangeCache = new Map<string, Range[]>();
  for (let i = 0; i < matches.length; i++) {
    const m = matches[i];
    const block = blockElement(m.blockId);
    const root = block?.querySelector(".block-content") as HTMLElement | null;
    if (!root) continue;
    let blockRanges = rangeCache.get(m.blockId);
    if (!blockRanges) {
      blockRanges = textRanges(root, query);
      rangeCache.set(m.blockId, blockRanges);
    }
    const range = blockRanges[m.ordinalInBlock];
    if (!range) continue;
    if (i === activeIdx) {
      activeRange = range;
      block?.classList.add("inpage-find-active-block");
    } else {
      ranges.push(range);
    }
  }
  if (!applyCssHighlights(ranges, activeRange)) applyOverlayHighlights(ranges, activeRange);
}
