import { createSignal } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import { latestOwner, type Owner } from "../owned";
import { isMobilePlatform } from "../nativeChrome";
import { reportUiFailure } from "../uiFailure";

export const PDF_FIND_TEXT_CACHE_BYTES = isMobilePlatform ? 4 * 1024 * 1024 : 8 * 1024 * 1024;
export const PDF_FIND_PAGE_TEXT_BYTES = 1024 * 1024;
export const PDF_FIND_MATCH_CAP = 10_000;

/** Own one reader's bounded text cache and find UI. Document scans run off the
 * UI thread in pdf.js and stop when the graph/view owner retires. The caller
 * supplies page rendering and scroll navigation; this never writes graph data. */
export function createPdfFind(ctx: {
  document: () => pdfjs.PDFDocumentProxy | null;
  owner: Owner;
  requests: object;
  scrollToPage: (page: number) => void;
  renderPage: (page: number) => Promise<void>;
  renderedScale: (page: number) => number | undefined;
  textScale: (page: number) => number | undefined;
  buildTextLayer: (page: number, scale: number) => Promise<void>;
  textLayer: (page: number) => HTMLDivElement | undefined;
  scrollElement: () => HTMLDivElement;
  scale: () => number;
}) {
  const [findOpen, setFindOpen] = createSignal(false);
  const [findQuery, setFindQuery] = createSignal("");
  const [findCount, setFindCount] = createSignal(0);
  const [findCur, setFindCur] = createSignal(0);
  const [findTruncated, setFindTruncated] = createSignal(false);
  let matches: { page: number }[] = [];
  const pageTextCache: Record<number, string> = {};
  const pageTextLru: number[] = [];
  let cacheBytes = 0;
  let debounce: number | undefined;
  let input: HTMLInputElement | undefined;

  function touch(n: number) {
    const index = pageTextLru.indexOf(n);
    if (index >= 0) pageTextLru.splice(index, 1);
    pageTextLru.push(n);
  }
  function admit(n: number, value: string) {
    const bytes = value.length * 2;
    if (bytes > PDF_FIND_PAGE_TEXT_BYTES || bytes > PDF_FIND_TEXT_CACHE_BYTES) return;
    while (cacheBytes + bytes > PDF_FIND_TEXT_CACHE_BYTES && pageTextLru.length) {
      const old = pageTextLru.shift()!;
      cacheBytes -= pageTextCache[old].length * 2;
      delete pageTextCache[old];
    }
    pageTextCache[n] = value;
    cacheBytes += bytes;
    touch(n);
  }
  async function pageText(n: number, current: Owner): Promise<string | null> {
    if (pageTextCache[n] !== undefined) { touch(n); return pageTextCache[n]; }
    const doc = ctx.document();
    if (!doc) return "";
    const page = await doc.getPage(n);
    if (!current()) return null;
    const content = await page.getTextContent();
    if (!current()) return null;
    let value = "";
    for (const item of content.items) {
      const part = "str" in item && typeof item.str === "string" ? item.str : "";
      if ((value.length + part.length) * 2 > PDF_FIND_PAGE_TEXT_BYTES) {
        setFindTruncated(true);
        break;
      }
      value += part;
    }
    admit(n, value);
    return value;
  }
  /** A newer scan, a closed Find or a teardown retires the scan AND any
   * navigation it started (I-20/I-21). */
  function retireNavigation() { latestOwner(ctx.requests, "find-goto"); }
  function clearResults() {
    matches = [];
    setFindCount(0);
    setFindCur(0);
    setFindTruncated(false);
    window.getSelection()?.removeAllRanges();
  }
  async function run(query: string) {
    const current = latestOwner(ctx.requests, "find", ctx.owner);
    retireNavigation();
    if (!current()) return;
    const q = query.trim().toLowerCase();
    const doc = ctx.document();
    if (!q || !doc) {
      clearResults();
      return;
    }
    const acc: { page: number }[] = [];
    setFindTruncated(false);
    for (let n = 1; n <= doc.numPages; n++) {
      let loaded: string | null;
      try { loaded = await pageText(n, current); }
      catch (error) {
        // A page pdf.js cannot read: say so and drop the previous query's
        // results instead of leaving them as if they answered this query (I-9).
        if (current()) { clearResults(); reportUiFailure("pdf-find", error); }
        return;
      }
      if (loaded === null) return;
      const text = loaded.toLowerCase();
      if (!current()) return;
      let index = text.indexOf(q);
      while (index >= 0) {
        acc.push({ page: n });
        if (acc.length >= PDF_FIND_MATCH_CAP) { setFindTruncated(true); break; }
        index = text.indexOf(q, index + q.length);
      }
      if (acc.length >= PDF_FIND_MATCH_CAP) break;
    }
    if (!current()) return;
    matches = acc;
    setFindCount(acc.length);
    if (acc.length) navigate(0);
    else { setFindCur(0); window.getSelection()?.removeAllRanges(); }
  }
  function runReported(query: string) {
    run(query).catch((error) => reportUiFailure("pdf-find", error));
  }
  function scheduleFind(query: string) {
    setFindQuery(query);
    clearTimeout(debounce);
    debounce = window.setTimeout(() => runReported(query), 180);
  }
  function nextMatch(delta: number) {
    if (matches.length) navigate(findCur() - 1 + delta);
  }
  function navigate(index: number) {
    gotoMatch(index).catch((error) => reportUiFailure("pdf-find", error));
  }
  async function gotoMatch(index: number) {
    const length = matches.length;
    if (!length) return;
    // The newest navigation owns the selection; a closed Find, a new query or a
    // teardown retires it, and it never rereads the mutable `matches` after an
    // await (I-20/I-21).
    const current = latestOwner(ctx.requests, "find-goto", ctx.owner, findOpen);
    const i = ((index % length) + length) % length;
    setFindCur(i + 1);
    const match = matches[i];
    let occurrence = -1;
    for (let j = 0; j <= i; j++) if (matches[j].page === match.page) occurrence++;
    ctx.scrollToPage(match.page);
    const s = ctx.scale();
    if (ctx.renderedScale(match.page) !== s) await ctx.renderPage(match.page);
    if (!current()) return;
    const rendered = ctx.renderedScale(match.page);
    if (rendered !== undefined && ctx.textScale(match.page) !== rendered) {
      await ctx.buildTextLayer(match.page, rendered);
      if (!current()) return;
    }
    selectOccurrence(match.page, occurrence);
  }
  function selectOccurrence(page: number, occurrence: number) {
    const layer = ctx.textLayer(page);
    if (!layer || occurrence < 0) return;
    const q = findQuery().trim().toLowerCase();
    if (!q) return;
    const walker = document.createTreeWalker(layer, NodeFilter.SHOW_TEXT);
    const nodes: Text[] = [], starts: number[] = [];
    let combined = "";
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      starts.push(combined.length);
      nodes.push(node as Text);
      combined += node.textContent ?? "";
    }
    const hay = combined.toLowerCase();
    let from = hay.indexOf(q), count = 0;
    while (from >= 0 && count < occurrence) { from = hay.indexOf(q, from + q.length); count++; }
    if (from < 0) return;
    const to = from + q.length;
    const nodeAt = (offset: number) => {
      for (let i = 0; i < nodes.length; i++) {
        if (offset < starts[i] + (nodes[i].textContent?.length ?? 0)) return { node: nodes[i], start: starts[i] };
      }
      return null;
    };
    const a = nodeAt(from), b = nodeAt(to - 1);
    if (!a || !b) return;
    const range = document.createRange();
    try { range.setStart(a.node, from - a.start); range.setEnd(b.node, to - b.start); }
    catch { return; }
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    const rect = range.getBoundingClientRect();
    const scroller = ctx.scrollElement();
    const bounds = scroller.getBoundingClientRect();
    if (rect.top < bounds.top + 48 || rect.bottom > bounds.bottom - 24) {
      scroller.scrollTop += rect.top - bounds.top - scroller.clientHeight * 0.3;
    }
  }
  function openFind() {
    setFindOpen(true);
    queueMicrotask(() => { input?.focus(); input?.select(); });
    if (findQuery().trim()) runReported(findQuery());
  }
  function closeFind() {
    setFindOpen(false);
    // Closing retires the pending debounce, any running scan and navigation.
    cancel();
    window.getSelection()?.removeAllRanges();
  }
  function cancel() { clearTimeout(debounce); latestOwner(ctx.requests, "find"); retireNavigation(); }
  return { findOpen, findQuery, findCount, findCur, findTruncated,
    scheduleFind, nextMatch, openFind, closeFind, cancel,
    setInput: (el: HTMLInputElement) => { input = el; } };
}
