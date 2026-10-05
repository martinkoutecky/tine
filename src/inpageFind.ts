import { revealOutlineBlock } from "./outlineViewport";
import { batch, createMemo, createRoot, createSignal } from "solid-js";
import { mainPages, pageByName, revealNode, resolveBlockRef, node as docNode } from "./document";
import { renderedBlockText, type RenderedTextOptions } from "./render/renderedText";
import { renderedBlocks } from "./lazyObserve";
import type { Format } from "./types";
import { focusedPaneId, layoutPaneIds, visibleLayoutNode, paneRouter } from "./panes";
import { sameRoute } from "./router";
import { captureBinding, stillBound } from "./binding";
import { searchSubstringSpans } from "./editor/searchQuery";
import { rightSidebar, rightSidebarOpen, sidebarItemKey, searchRemoveAccents } from "./ui";

export interface InPageFindMatch {
  blockId: string;
  /** Owning pane or sidebar item; distinguishes repeated content across views. */
  scopeId?: string;
  /** Present for visible non-outline surfaces such as query rows and references. */
  surfaceId?: string;
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
  const [surfaceRevision, setSurfaceRevision] = createSignal(0);
  const matches = createMemo(() => currentMatchesFor(query()));
  return {
    open, setOpen,
    query, setQuery,
    activeIndex, setActiveIndex,
    focusRequest, setFocusRequest,
    preserveEditorBlur, setPreserveEditorBlur,
    paneId, setPaneId,
    surfaceRevision, setSurfaceRevision,
    matches,
  };
});

let restoreFocusEl: HTMLElement | null = null;
let revealToken = 0;
let highlightToken = 0;
let overlayRoot: HTMLDivElement | null = null;
let surfaceObserver: MutationObserver | null = null;
let surfaceObserverFrame = 0;

export const inPageFindOpen = state.open;
export const inPageFindQuery = state.query;
export const inPageFindActiveIndex = state.activeIndex;
export const inPageFindFocusRequest = state.focusRequest;
export const inPageFindPaneId = state.paneId;

export function inPageFindPreservesEditorBlur(): boolean {
  return state.preserveEditorBlur();
}

/** Non-overlapping original UTF-16 ranges using the caller's graph fold policy.
 * Empty query has no matches. The mapped substring scan costs O(text × query)
 * plus deduplication across candidate spans. */
export function findTextOccurrences(text: string, query: string, removeAccents: boolean): { start: number; end: number }[] {
  if (!query) return [];
  const out: { start: number; end: number }[] = [];
  for (const span of searchSubstringSpans(text, query, Number.POSITIVE_INFINITY, removeAccents)) {
    if (!out.length || span.start >= out[out.length - 1].end) out.push(span);
  }
  return out;
}

/** One find walk for DTO nodes and live ids; lookup is borrowed and missing live
 * ids are skipped. O(searched nodes + rendered text), sharing the existing text cache. */
function appendOutlineMatches<T>(
  blocks: readonly T[], lookup: (block: T) => { id: string; raw: string; children: readonly T[] } | undefined,
  query: string, format: Format, removeAccents: boolean, out: InPageFindMatch[],
): void {
  for (const block of blocks) {
    const node = lookup(block);
    if (!node) continue;
    const text = cachedRenderedBlockText(node.id, node.raw, format);
    findTextOccurrences(text, query, removeAccents).forEach((m, ordinalInBlock) => {
      out.push({ blockId: node.id, ordinalInBlock, start: m.start, end: m.end });
    });
    appendOutlineMatches(node.children, lookup, query, format, removeAccents, out);
  }
}
const dtoFindNode = (node: InPageFindBlock) => node;

export function collectInPageFindMatches(
  blocks: readonly InPageFindBlock[],
  query: string,
  format: Format = "md",
  removeAccents = true,
): InPageFindMatch[] {
  const q = query.trim();
  if (!q) return [];
  const out: InPageFindMatch[] = [];
  appendOutlineMatches(blocks, dtoFindNode, q, format, removeAccents, out);
  return out;
}

function currentMatchesFor(query: string): InPageFindMatch[] {
  const q = query.trim();
  if (!q) return [];
  const removeAccents = searchRemoveAccents();
  state.surfaceRevision();
  const out: InPageFindMatch[] = [];
  const scannedSurfaces = new Set<string>();
  for (const scope of findScopes()) {
    const start = out.length;
    appendOutlineMatches(scope.roots, docNode, q, scope.format, removeAccents, out);
    for (let i = start; i < out.length; i++) out[i].scopeId = scope.id;
    const element = findScopeElement(scope.id);
    if (!element || scannedSurfaces.has(scope.id)) continue;
    scannedSurfaces.add(scope.id);
    for (const surface of element.querySelectorAll<HTMLElement>("[data-inpage-find-surface]")) {
      const surfaceId = surface.dataset.inpageFindSurface;
      if (!surfaceId) continue;
      findTextOccurrences(searchableTextForRoot(surface), q, removeAccents).forEach((match, ordinalInBlock) => {
        out.push({ blockId: `surface:${surfaceId}`, scopeId: scope.id, surfaceId, ordinalInBlock, ...match });
      });
    }
  }
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

/** Visible layout order, then mounted sidebar stack order;
 * outlines retain lazy/collapsed descendants so Find can reveal them. */
function findScopes(): { id: string; roots: readonly string[]; format: Format }[] {
  const scopes: { id: string; roots: readonly string[]; format: Format }[] = [];
  for (const id of layoutPaneIds(visibleLayoutNode())) {
    const r = paneRouter(id).route();
    const pages = r.kind === "journals" ? mainPages() : r.kind === "page" ? [pageByName(r.name)].filter((p) => !!p) : [];
    for (const page of pages) {
      const root = r.kind === "page" && r.block ? resolveBlockRef({ uuid: r.block, page: r.name, pageKind: r.pageKind, path: r.path }, { navigation: true }) : null;
      scopes.push({ id, roots: root ? [root] : page.roots, format: page.format });
    }
    // Non-outline panes (Search, etc.) opt rendered rows in via surface ids.
    if (!pages.length) scopes.push({ id, roots: [], format: "md" });
  }
  if (rightSidebarOpen()) for (const item of rightSidebar()) {
    if (item.collapsed) continue;
    const id = `sidebar:${sidebarItemKey(item)}`;
    if (!findScopeElement(id)) continue;
    const page = pageByName(item.kind === "page" ? item.name : item.page);
    if (!page || (item.path && page.id !== item.path)) continue;
    const root = item.kind === "block" ? resolveBlockRef(item, { navigation: true }) : null;
    scopes.push({ id, roots: item.kind === "block" ? root ? [root] : [] : page.roots, format: page.format });
  }
  return scopes;
}

export function openInPageFind() {
  if (typeof document !== "undefined") restoreFocusEl = document.activeElement as HTMLElement | null;
  batch(() => {
    state.setPaneId(notesPaneId(focusedPaneId()));
    state.setPreserveEditorBlur(true);
    state.setOpen(true);
    state.setFocusRequest((n) => n + 1);
  });
  queueMicrotask(observeCurrentFindSurfaces);
  if (state.query().trim()) activateInPageFindIndex(Math.max(0, state.activeIndex()));
}

export function closeInPageFind(opts: { restoreFocus?: boolean } = {}) {
  revealToken++;
  const restoreFocus = opts.restoreFocus !== false;
  const target = restoreFocusEl;
  restoreFocusEl = null;
  batch(() => {
    state.setOpen(false);
    state.setActiveIndex(-1);
    state.setPaneId(null);
  });
  clearInPageFindHighlights();
  disconnectFindSurfaceObserver();
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
    revealToken++;
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
    revealToken++;
    state.setActiveIndex(-1);
    clearInPageFindHighlights();
    return;
  }
  const next = ((index % matches.length) + matches.length) % matches.length;
  state.setActiveIndex(next);
  const token = ++revealToken;
  const binding = captureBinding();
  const paneId = currentFindPaneId();
  const tabId = paneRouter(paneId).activeId();
  const route = paneRouter(paneId).route();
  void revealInPageFindMatch(matches[next]).then(() => {
    if (token === revealToken && state.open() && stillBound(binding)
      && currentFindPaneId() === paneId && paneRouter(paneId).activeId() === tabId
      && sameRoute(paneRouter(paneId).route(), route))
      refreshInPageFindHighlights();
  });
}

function expandAncestorsForFind(blockId: string) {
  let parent = docNode(blockId)?.parent ?? null;
  while (parent !== null) {
    const n = docNode(parent);
    if (!n) return;
    if (n.collapsed) revealNode(parent);
    parent = n.parent;
  }
}

function blockSelector(id: string): string {
  const esc = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id.replace(/"/g, '\\"');
  return `.ls-block[data-block-id="${esc}"]`;
}

function paneSelector(id: string): string {
  const esc = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id.replace(/"/g, '\\"');
  return id.startsWith("sidebar:") ? `[data-sidebar-surface="${esc}"]` : `[data-pane-id="${esc}"]`;
}

function surfaceSelector(id: string): string {
  const esc = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id.replace(/"/g, '\\"');
  return `[data-inpage-find-surface="${esc}"]`;
}

/** Resolve a rendered block in its pane or sidebar item. O(DOM in that view),
 * returning null while a lazy block is unmounted; callers reveal/wait first. */
export function inPageFindBlockElement(id: string, scopeId = currentFindPaneId()): HTMLElement | null {
  return (document.querySelector(paneSelector(scopeId)) as HTMLElement | null)?.querySelector(blockSelector(id)) as HTMLElement | null;
}

function inPageFindSurfaceElement(id: string, paneId = currentFindPaneId()): HTMLElement | null {
  return (document.querySelector(paneSelector(paneId)) as HTMLElement | null)?.querySelector(surfaceSelector(id)) as HTMLElement | null;
}

function disconnectFindSurfaceObserver() {
  surfaceObserver?.disconnect();
  surfaceObserver = null;
  if (surfaceObserverFrame) cancelAnimationFrame(surfaceObserverFrame);
  surfaceObserverFrame = 0;
}

function observeCurrentFindSurfaces() {
  disconnectFindSurfaceObserver();
  if (!state.open() || typeof MutationObserver === "undefined") return;
  const pane = document.body;
  surfaceObserver = new MutationObserver((records) => {
    if (records.every((record) => {
      const target = record.target instanceof Element ? record.target : record.target.parentElement;
      if (target?.closest(".inpage-find-overlays,.inpage-find-bar")) return true;
      return record.type === "childList" && [...record.addedNodes, ...record.removedNodes].every((node) =>
        node instanceof Element && node.matches(".inpage-find-overlays,.inpage-find-bar"));
    })) return;
    if (surfaceObserverFrame) return;
    surfaceObserverFrame = requestAnimationFrame(() => {
      surfaceObserverFrame = 0;
      state.setSurfaceRevision((revision) => revision + 1);
      const matches = inPageFindMatches();
      if (!matches.length) state.setActiveIndex(-1);
      else if (state.activeIndex() < 0 || state.activeIndex() >= matches.length) state.setActiveIndex(0);
      refreshInPageFindHighlights();
    });
  });
  surfaceObserver.observe(pane, { childList: true, subtree: true, characterData: true });
  state.setSurfaceRevision((revision) => revision + 1);
}

function animationFrame(): Promise<void> {
  return new Promise((resolve) => {
    if (typeof requestAnimationFrame !== "undefined") requestAnimationFrame(() => resolve());
    else setTimeout(resolve, 0);
  });
}

export async function revealInPageFindMatch(match: InPageFindMatch): Promise<boolean> {
  const binding = captureBinding();
  const token = revealToken;
  const scopeId = match.scopeId ?? currentFindPaneId();
  const paneId = scopeId.startsWith("sidebar:") ? currentFindPaneId() : scopeId;
  const tabId = paneRouter(paneId).activeId();
  const route = paneRouter(paneId).route();
  const current = () => state.open() && token === revealToken && stillBound(binding)
    && !!findScopeElement(scopeId) && paneRouter(paneId).activeId() === tabId
    && sameRoute(paneRouter(paneId).route(), route);
  if (match.surfaceId) {
    for (let i = 0; i < 20; i++) {
      await animationFrame();
      if (!current()) return false;
      const element = inPageFindSurfaceElement(match.surfaceId, scopeId);
      if (element) {
        element.scrollIntoView({ block: "center", behavior: "smooth" });
        return true;
      }
    }
    return false;
  }
  const node = docNode(match.blockId);
  if (!node) return false;
  renderedBlocks.add(match.blockId);
  expandAncestorsForFind(match.blockId);
  for (let i = 0; i < 20; i++) {
    await animationFrame();
    if (!current()) return false;
    revealOutlineBlock(match.blockId, findScopeElement(scopeId));
    const el = inPageFindBlockElement(match.blockId, scopeId);
    if (el) {
      if (!centerInPageFindOccurrence(el, match.ordinalInBlock)) {
        el.scrollIntoView({ block: "center", behavior: "smooth" });
      }
      return true;
    }
  }
  return false;
}

/** Nearest ancestor that scrolls vertically (the pane content area), or null. */
function nearestVerticalScroller(el: HTMLElement): HTMLElement | null {
  for (let cur = el.parentElement; cur; cur = cur.parentElement) {
    const overflowY = window.getComputedStyle(cur).overflowY;
    // `overflow-y: auto` on an element that does not actually overflow scrolls
    // nowhere. Accepting one would make the adjustment below a no-op while
    // still reporting success, so the caller would skip its block-level
    // fallback and the occurrence would stay off-screen — worse than before.
    const scrolls = cur.scrollHeight > cur.clientHeight;
    if ((overflowY === "auto" || overflowY === "scroll") && scrolls) return cur;
  }
  return null;
}

/** GH #253: center the ACTIVE OCCURRENCE, not just its block. A block-level
 * scrollIntoView leaves occurrences off-screen when the block is taller than
 * the viewport. Measures the occurrence's Range rect (the same ordinal the
 * highlight layer uses) and adjusts the pane scroller. Returns false when no
 * precise rect can be resolved (e.g. the block is in edit mode; jsdom) so the
 * caller falls back to the coarse block scroll. */
function centerInPageFindOccurrence(blockEl: HTMLElement, ordinalInBlock: number): boolean {
  const root = blockEl.querySelector(".block-content") as HTMLElement | null;
  const query = state.query().trim();
  if (!root || !query) return false;
  const range = textRanges(root, query)[ordinalInBlock];
  if (!range || typeof range.getClientRects !== "function") return false;
  const rect = Array.from(range.getClientRects()).find((r) => r.width > 0 || r.height > 0);
  if (!rect) return false;
  const occurrenceCenter = rect.top + rect.height / 2;
  const scroller = nearestVerticalScroller(blockEl);
  if (scroller) {
    const scRect = scroller.getBoundingClientRect();
    scroller.scrollTop += occurrenceCenter - (scRect.top + scroller.clientHeight / 2);
  } else {
    window.scrollBy({ top: occurrenceCenter - window.innerHeight / 2 });
  }
  return true;
}

interface TextPart {
  node: Text;
  start: number;
  end: number;
}

function textPartsForRoot(root: HTMLElement): { parts: TextPart[]; text: string } {
  const parts: TextPart[] = [];
  let text = "";
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const value = node.textContent ?? "";
      if (!value) return NodeFilter.FILTER_REJECT;
      const parent = node.parentElement;
      const control = parent?.closest("button,input,textarea,select");
      if (!parent) return NodeFilter.FILTER_REJECT;
      // aria-hidden text is layout, not content: a code block's line gutter
      // repeats every line invisibly to mirror soft wraps (LineGutter).
      const hidden = parent.closest('[aria-hidden="true"]');
      if (hidden && root.contains(hidden)) return NodeFilter.FILTER_REJECT;
      // A control is chrome ("Show full block", a jump circle), so find skips its
      // label. A control that CARRIES content opts in with data-inpage-find-text:
      // an unlinked-reference mention is a button, and those are the very words
      // the reader typed into find (master 54c00e2d6).
      if (control && control !== root && !control.hasAttribute("data-inpage-find-text")) {
        return NodeFilter.FILTER_REJECT;
      }
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
  return { parts, text };
}

function searchableTextForRoot(root: HTMLElement): string {
  return textPartsForRoot(root).text;
}

function textRanges(root: HTMLElement, query: string): Range[] {
  const q = query.trim();
  if (!q) return [];
  const { parts, text } = textPartsForRoot(root);
  const pointForStart = (offset: number): [Text, number] | null => {
    for (const p of parts) if (offset >= p.start && offset < p.end) return [p.node, offset - p.start];
    return null;
  };
  const pointForEnd = (offset: number): [Text, number] | null => {
    for (const p of parts) if (offset > p.start && offset <= p.end) return [p.node, offset - p.start];
    return null;
  };
  const ranges: Range[] = [];
  for (const m of findTextOccurrences(text, q, searchRemoveAccents())) {
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

function findScopeElement(id: string): HTMLElement | null {
  return typeof document === "undefined" ? null : document.querySelector(paneSelector(id));
}

function elementMatchKey(el: HTMLElement, id: string): string {
  const scope = el.closest<HTMLElement>("[data-sidebar-surface],[data-pane-id]");
  return `${scope?.dataset.sidebarSurface ?? scope?.dataset.paneId ?? currentFindPaneId()}\0${id}`;
}
function matchKey(match: InPageFindMatch): string {
  return `${match.scopeId ?? currentFindPaneId()}\0${match.surfaceId ?? match.blockId}`;
}

function isViewportVisible(el: HTMLElement, clipRect?: DOMRect): boolean {
  const rect = el.getBoundingClientRect();
  const height = window.innerHeight || document.documentElement.clientHeight;
  const width = window.innerWidth || document.documentElement.clientWidth;
  if (rect.bottom < 0 || rect.right < 0 || rect.top > height || rect.left > width) return false;
  if (!clipRect) return true;
  return rect.bottom >= clipRect.top && rect.top <= clipRect.bottom && rect.right >= clipRect.left && rect.left <= clipRect.right;
}

function visibleFindElements(selector: string): HTMLElement[] {
  return [...new Set(findScopes().map((scope) => scope.id))].flatMap((id) => {
    const scope = findScopeElement(id);
    if (!scope) return [];
    const rect = scope.getBoundingClientRect();
    return Array.from(scope.querySelectorAll<HTMLElement>(selector)).filter((el) => isViewportVisible(el, rect));
  });
}
function visibleFindBlockElements(): HTMLElement[] {
  return visibleFindElements(".ls-block[data-block-id]").filter((el) => !!el.querySelector(".block-content"));
}
function visibleFindSurfaceElements(): HTMLElement[] {
  return visibleFindElements("[data-inpage-find-surface]");
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
  const surfaceCandidates = visibleFindSurfaceElements();
  if (activeMatch?.surfaceId) {
    const activeSurface = inPageFindSurfaceElement(activeMatch.surfaceId, activeMatch.scopeId);
    if (activeSurface && !surfaceCandidates.includes(activeSurface)) surfaceCandidates.push(activeSurface);
  } else if (activeMatch) {
    const activeBlock = inPageFindBlockElement(activeMatch.blockId, activeMatch.scopeId);
    if (activeBlock && !candidates.includes(activeBlock)) candidates.push(activeBlock);
  }
  const candidateIds = new Set(candidates.map((el) => elementMatchKey(el, blockIdForElement(el) ?? "")).filter((id): id is string => !!id));
  const candidateSurfaceIds = new Set(surfaceCandidates.map((element) => elementMatchKey(element, element.dataset.inpageFindSurface ?? "")).filter((id): id is string => !!id));
  if (!candidateIds.size && !candidateSurfaceIds.size) {
    resetCssHighlights();
    clearOverlayHighlights();
    return;
  }
  const matchesByBlock = new Map<string, { ordinalInBlock: number; matchIndex: number }[]>();
  const matchesBySurface = new Map<string, { ordinalInBlock: number; matchIndex: number }[]>();
  for (let i = 0; i < matches.length; i++) {
    const m = matches[i];
    if (m.surfaceId) {
      if (!candidateSurfaceIds.has(matchKey(m))) continue;
      const bucket = matchesBySurface.get(matchKey(m));
      if (bucket) bucket.push({ ordinalInBlock: m.ordinalInBlock, matchIndex: i });
      else matchesBySurface.set(matchKey(m), [{ ordinalInBlock: m.ordinalInBlock, matchIndex: i }]);
      continue;
    }
    if (!candidateIds.has(matchKey(m))) continue;
    const bucket = matchesByBlock.get(matchKey(m));
    if (bucket) bucket.push({ ordinalInBlock: m.ordinalInBlock, matchIndex: i });
    else matchesByBlock.set(matchKey(m), [{ ordinalInBlock: m.ordinalInBlock, matchIndex: i }]);
  }
  void refreshInPageFindHighlightsChunked(token, candidates, surfaceCandidates, matchesByBlock, matchesBySurface, query, activeIdx);
}

async function refreshInPageFindHighlightsChunked(
  token: number,
  candidates: HTMLElement[],
  surfaceCandidates: HTMLElement[],
  matchesByBlock: Map<string, { ordinalInBlock: number; matchIndex: number }[]>,
  matchesBySurface: Map<string, { ordinalInBlock: number; matchIndex: number }[]>,
  query: string,
  activeIdx: number,
) {
  const binding = captureBinding();
  const paneId = currentFindPaneId();
  const tabId = paneRouter(paneId).activeId();
  const route = paneRouter(paneId).route();
  const current = () => token === highlightToken && state.open() && stillBound(binding)
    && currentFindPaneId() === paneId && paneRouter(paneId).activeId() === tabId
    && sameRoute(paneRouter(paneId).route(), route);
  const ranges: Range[] = [];
  let activeRange: Range | null = null;
  for (let i = 0; i < candidates.length; i++) {
    if (i > 0 && i % HIGHLIGHT_BLOCK_CHUNK_SIZE === 0) await animationFrame();
    if (!current()) return;
    const block = candidates[i];
    const blockId = blockIdForElement(block);
    if (!blockId) continue;
    const blockMatches = matchesByBlock.get(elementMatchKey(block, blockId));
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
  for (let i = 0; i < surfaceCandidates.length; i++) {
    if ((candidates.length + i) > 0 && (candidates.length + i) % HIGHLIGHT_BLOCK_CHUNK_SIZE === 0) await animationFrame();
    if (!current()) return;
    const surface = surfaceCandidates[i];
    const surfaceId = surface.dataset.inpageFindSurface;
    if (!surfaceId) continue;
    const surfaceMatches = matchesBySurface.get(elementMatchKey(surface, surfaceId));
    if (!surfaceMatches?.length) continue;
    const surfaceRanges = textRanges(surface, query);
    for (const match of surfaceMatches) {
      const range = surfaceRanges[match.ordinalInBlock];
      if (!range) continue;
      if (match.matchIndex === activeIdx) {
        activeRange = range;
        surface.classList.add("inpage-find-active-block");
      } else {
        ranges.push(range);
      }
    }
  }
  if (!current()) return;
  if (!applyCssHighlights(ranges, activeRange)) applyOverlayHighlights(ranges, activeRange);
}
