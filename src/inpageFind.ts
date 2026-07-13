import { batch, createMemo, createRoot, createSignal } from "solid-js";
import { doc, mainPages, pageByName, setDoc, type FeedPage } from "./store";
import { renderedBlockText, type RenderedTextOptions } from "./render/renderedText";
import { renderedBlocks } from "./lazyObserve";
import type { Format } from "./types";
import { focusedPaneId, layoutPaneIds, paneRouter } from "./panes";

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
const RENDERED_TEXT_CACHE_LIMIT = 4096;
const HIGHLIGHT_BLOCK_CHUNK_SIZE = 40;

interface RenderedTextCacheEntry {
  raw: string;
  format: Format;
  text: string;
}

const renderedTextOptions: RenderedTextOptions = {
  typographicGlyphs: false,
  stripLinks: false,
  removeTags: false,
  removeProperties: false,
};

const renderedTextCache = new Map<string, RenderedTextCacheEntry>();

function cachedRenderedBlockText(blockId: string, raw: string, format: Format): string {
  const cached = renderedTextCache.get(blockId);
  if (cached && cached.raw === raw && cached.format === format) {
    renderedTextCache.delete(blockId);
    renderedTextCache.set(blockId, cached);
    return cached.text;
  }
  const text = renderedBlockText(raw, format, renderedTextOptions);
  renderedTextCache.set(blockId, { raw, format, text });
  while (renderedTextCache.size > RENDERED_TEXT_CACHE_LIMIT) {
    const oldest = renderedTextCache.keys().next().value;
    if (oldest === undefined) break;
    renderedTextCache.delete(oldest);
  }
  return text;
}

export function clearInPageFindRenderedTextCacheForTests() {
  renderedTextCache.clear();
}

const state = createRoot(() => {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [activeIndex, setActiveIndex] = createSignal(-1);
  const [focusRequest, setFocusRequest] = createSignal(0);
  const [preserveEditorBlur, setPreserveEditorBlur] = createSignal(false);
  const [paneId, setPaneId] = createSignal<string | null>(null);
  const matches = createMemo(() => currentMatchesFor(query()));
  return {
    open, setOpen,
    query, setQuery,
    activeIndex, setActiveIndex,
    focusRequest, setFocusRequest,
    preserveEditorBlur, setPreserveEditorBlur,
    paneId, setPaneId,
    matches,
  };
});

let restoreFocusEl: HTMLElement | null = null;
let revealToken = 0;
let highlightToken = 0;
let overlayRoot: HTMLDivElement | null = null;

export const inPageFindOpen = state.open;
export const inPageFindQuery = state.query;
export const inPageFindActiveIndex = state.activeIndex;
export const inPageFindFocusRequest = state.focusRequest;
export const inPageFindPaneId = state.paneId;

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
      const text = cachedRenderedBlockText(b.id, b.raw, format);
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
      const text = cachedRenderedBlockText(id, n.raw, format);
      findTextOccurrences(text, q).forEach((m, ordinalInBlock) => {
        out.push({ blockId: id, ordinalInBlock, start: m.start, end: m.end });
      });
      walk(n.children, format);
    }
  };
  for (const p of pagesForInPageFind()) walk(p.roots, p.format);
  return out;
}

export function inPageFindMatches(): InPageFindMatch[] {
  return state.matches();
}

export function scopedInPageFindMatchesForQuery(query: string): InPageFindMatch[] {
  return currentMatchesFor(query);
}

function notesPaneId(id: string | null): string {
  const ids = layoutPaneIds();
  return id && ids.includes(id) ? id : ids[0] ?? "main";
}

function currentFindPaneId(): string {
  return notesPaneId(state.paneId() ?? focusedPaneId());
}

function pagesForInPageFind(): FeedPage[] {
  const router = paneRouter(currentFindPaneId());
  const r = router.route();
  if (r.kind === "journals") return mainPages();
  if (r.kind === "query") return [];
  const page = pageByName(r.name);
  return page ? [page] : [];
}

export function openInPageFind() {
  if (typeof document !== "undefined") restoreFocusEl = document.activeElement as HTMLElement | null;
  batch(() => {
    state.setPaneId(notesPaneId(focusedPaneId()));
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
    state.setPaneId(null);
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
  const matches = inPageFindMatches();
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

function paneSelector(id: string): string {
  const esc = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id.replace(/"/g, '\\"');
  return `[data-pane-id="${esc}"]`;
}

export function inPageFindBlockElement(id: string, paneId = currentFindPaneId()): HTMLElement | null {
  return (document.querySelector(paneSelector(paneId)) as HTMLElement | null)?.querySelector(blockSelector(id)) as HTMLElement | null;
}

function animationFrame(): Promise<void> {
  return new Promise((resolve) => {
    if (typeof requestAnimationFrame !== "undefined") requestAnimationFrame(() => resolve());
    else setTimeout(resolve, 0);
  });
}

export async function revealInPageFindMatch(match: InPageFindMatch): Promise<boolean> {
  const node = doc.byId[match.blockId];
  if (!node) return false;
  renderedBlocks.add(match.blockId);
  expandAncestorsForFind(match.blockId);
  for (let i = 0; i < 20; i++) {
    await animationFrame();
    const el = inPageFindBlockElement(match.blockId);
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
  highlightToken++;
  if (typeof CSS !== "undefined") {
    ((CSS as unknown as { highlights?: Map<string, unknown> }).highlights as any)?.delete?.(FIND_HIGHLIGHT);
    ((CSS as unknown as { highlights?: Map<string, unknown> }).highlights as any)?.delete?.(FIND_ACTIVE_HIGHLIGHT);
  }
  clearOverlayHighlights();
  if (typeof document === "undefined") return;
  document.querySelectorAll(".inpage-find-active-block").forEach((el) => el.classList.remove("inpage-find-active-block"));
}

function blockIdForElement(el: Element): string | null {
  return el.getAttribute("data-block-id");
}

function currentFindPaneElement(): HTMLElement | null {
  return document.querySelector(paneSelector(currentFindPaneId())) as HTMLElement | null;
}

function isViewportVisible(el: HTMLElement, clipRect?: DOMRect): boolean {
  const rect = el.getBoundingClientRect();
  const height = window.innerHeight || document.documentElement.clientHeight;
  const width = window.innerWidth || document.documentElement.clientWidth;
  if (rect.bottom < 0 || rect.right < 0 || rect.top > height || rect.left > width) return false;
  if (!clipRect) return true;
  return rect.bottom >= clipRect.top && rect.top <= clipRect.bottom && rect.right >= clipRect.left && rect.left <= clipRect.right;
}

function visibleFindBlockElements(): HTMLElement[] {
  const pane = currentFindPaneElement();
  if (!pane) return [];
  const paneRect = pane.getBoundingClientRect();
  return Array.from(pane.querySelectorAll(".ls-block[data-block-id]")).filter((el): el is HTMLElement => {
    const htmlEl = el as HTMLElement;
    const content = htmlEl.querySelector(".block-content") as HTMLElement | null;
    return !!content && isViewportVisible(htmlEl, paneRect);
  });
}

function resetCssHighlights() {
  if (typeof CSS === "undefined") return;
  ((CSS as unknown as { highlights?: Map<string, unknown> }).highlights as any)?.delete?.(FIND_HIGHLIGHT);
  ((CSS as unknown as { highlights?: Map<string, unknown> }).highlights as any)?.delete?.(FIND_ACTIVE_HIGHLIGHT);
}

export function refreshInPageFindHighlights() {
  const token = ++highlightToken;
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
  const activeMatch = matches[activeIdx] ?? null;
  const candidates = visibleFindBlockElements();
  if (activeMatch) {
    const activeBlock = inPageFindBlockElement(activeMatch.blockId);
    if (activeBlock && !candidates.includes(activeBlock)) candidates.push(activeBlock);
  }
  const candidateIds = new Set(candidates.map(blockIdForElement).filter((id): id is string => !!id));
  if (!candidateIds.size) {
    resetCssHighlights();
    clearOverlayHighlights();
    return;
  }
  const matchesByBlock = new Map<string, { ordinalInBlock: number; matchIndex: number }[]>();
  for (let i = 0; i < matches.length; i++) {
    const m = matches[i];
    if (!candidateIds.has(m.blockId)) continue;
    const bucket = matchesByBlock.get(m.blockId);
    if (bucket) bucket.push({ ordinalInBlock: m.ordinalInBlock, matchIndex: i });
    else matchesByBlock.set(m.blockId, [{ ordinalInBlock: m.ordinalInBlock, matchIndex: i }]);
  }
  void refreshInPageFindHighlightsChunked(token, candidates, matchesByBlock, query, activeIdx);
}

async function refreshInPageFindHighlightsChunked(
  token: number,
  candidates: HTMLElement[],
  matchesByBlock: Map<string, { ordinalInBlock: number; matchIndex: number }[]>,
  query: string,
  activeIdx: number,
) {
  const ranges: Range[] = [];
  let activeRange: Range | null = null;
  for (let i = 0; i < candidates.length; i++) {
    if (i > 0 && i % HIGHLIGHT_BLOCK_CHUNK_SIZE === 0) await animationFrame();
    if (token !== highlightToken) return;
    const block = candidates[i];
    const blockId = blockIdForElement(block);
    if (!blockId) continue;
    const blockMatches = matchesByBlock.get(blockId);
    if (!blockMatches?.length) continue;
    const root = block?.querySelector(".block-content") as HTMLElement | null;
    if (!root) continue;
    const blockRanges = textRanges(root, query);
    for (const m of blockMatches) {
      const range = blockRanges[m.ordinalInBlock];
      if (!range) continue;
      if (m.matchIndex === activeIdx) {
        activeRange = range;
        block.classList.add("inpage-find-active-block");
      } else {
        ranges.push(range);
      }
    }
  }
  if (token !== highlightToken) return;
  if (!applyCssHighlights(ranges, activeRange)) applyOverlayHighlights(ranges, activeRange);
}
