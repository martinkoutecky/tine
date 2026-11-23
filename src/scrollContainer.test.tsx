import { afterEach, describe, expect, it, vi } from "vitest";
import { batchViewportChanges, cachedScrollContainer, preserveAboveViewport, scrollContainer, trackViewportAnchor } from "./scrollContainer";
import { observeNear, resetNearObserverForTests } from "./lazyObserve";
import { createSignal, createRoot } from "solid-js";
import { render } from "solid-js/web";
import { reservedGroupHeight } from "./reservedGroupHeight";

afterEach(() => { resetNearObserverForTests(); vi.unstubAllGlobals(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("anchors a surviving reference through filtering and resize without undoing a scrollbar jump", () => {
  let mutate!: () => void, resize!: () => void;
  vi.stubGlobal("MutationObserver", class { constructor(callback: () => void) { mutate = callback; } observe() {} disconnect() {} });
  vi.stubGlobal("ResizeObserver", class { constructor(callback: () => void) { resize = callback; } observe() {} disconnect() {} });
  const root = document.createElement("div"), group = document.createElement("div");
  root.style.overflowY = "auto"; group.className = "reference-group"; root.append(group); document.body.append(root);
  let position = 100;
  vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0, left: 0, width: 900 } as DOMRect);
  vi.spyOn(group, "getBoundingClientRect").mockImplementation(() => ({ top: position - root.scrollTop }) as DOMRect);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => group });
  const stop = trackViewportAnchor(group), stopOther = trackViewportAnchor(group);
  try {
    position += 80; mutate(); expect(root.scrollTop).toBe(80);
    root.scrollTop = 500; position += 20; mutate(); expect(root.scrollTop).toBe(500);
    position += 50; resize(); expect(root.scrollTop).toBe(550);
  } finally { stop(); stopOther(); Reflect.deleteProperty(document, "elementFromPoint"); }
});

it("admits 50 below-view IO entries with zero rect reads, including the expanded root margin", () => {
  let deliver!: IntersectionObserverCallback;
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: IntersectionObserverCallback) { deliver = callback; }
    observe() {} unobserve() {} disconnect() {}
  });
  const root = document.createElement("div");
  root.style.overflowY = "auto";
  document.body.append(root);
  const reads = vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    return (this === root ? { top: 100 } : { bottom: 200 }) as unknown as DOMRect;
  });
  const changes = vi.fn();
  const entries = Array.from({ length: 50 }, () => {
    const host = document.createElement("div");
    root.append(host);
    observeNear(host, entry => preserveAboveViewport(host, changes, undefined, false, entry, 1200));
    return { target: host, isIntersecting: true, rootBounds: { top: -1100 }, boundingClientRect: { bottom: 200 } } as unknown as IntersectionObserverEntry;
  });
  deliver(entries, {} as IntersectionObserver);
  expect(changes).toHaveBeenCalledTimes(50);
  expect(reads).not.toHaveBeenCalled();
});

it("reads before the DOM batch, restores once for 50 above-view entries, and includes their total growth", () => {
  const root = document.createElement("div"), reader = document.createElement("div");
  reader.className = "block-main";
  root.append(reader); document.body.append(root);
  vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 100, left: 0, width: 400 } as DOMRect);
  const top = vi.spyOn(reader, "getBoundingClientRect").mockImplementation(() => ({ top: 120 + reader.children.length * 10 - root.scrollTop }) as DOMRect);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => reader });
  let writes = 0, scroll = 0;
  Object.defineProperty(root, "scrollTop", { configurable: true, get: () => scroll, set: value => { writes++; scroll = value; } });
  const disposers: (() => void)[] = [];
  const updates = Array.from({ length: 50 }, () => {
    const host = document.createElement("div"); root.prepend(host);
    vi.spyOn(host, "getBoundingClientRect").mockReturnValue({ bottom: 50 } as DOMRect);
    const [shown, setShown] = createSignal(false);
    disposers.push(render(() => <>{shown() && <span />}</>, reader));
    return () => preserveAboveViewport(host, () => setShown(true), root, false,
      { rootBounds: { top: -1100 }, boundingClientRect: { bottom: 50 } } as IntersectionObserverEntry, 1200);
  });
  try {
    batchViewportChanges(() => updates.forEach(update => update()));
    expect(root.scrollTop).toBe(500);
    expect(writes).toBe(1);
    expect(top).toHaveBeenCalledTimes(2);
  } finally { disposers.forEach(dispose => dispose()); Reflect.deleteProperty(document, "elementFromPoint"); }
});

it("reuses root style resolution across sibling registrations and resolves a moved host again", () => {
  const root = document.createElement("div"), pane = document.createElement("div"), other = document.createElement("div");
  root.style.overflowY = other.style.overflowY = "auto";
  root.append(pane); document.body.append(root, other);
  const style = vi.spyOn(window, "getComputedStyle");
  for (let i = 0; i < 50; i++) {
    const host = document.createElement("div"); pane.append(host);
    expect(cachedScrollContainer(host)).toBe(root);
  }
  expect(style).toHaveBeenCalledTimes(2);
  other.append(pane);
  expect(cachedScrollContainer(pane.firstElementChild!)).toBe(other);
});

it("remembers delivered group sizes without any cleanup layout reads", () => {
  let deliver!: ResizeObserverCallback;
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: ResizeObserverCallback) { deliver = callback; }
    observe() {} unobserve() {} disconnect() {}
  });
  const disposers: (() => void)[] = [], hosts: HTMLElement[] = [];
  for (let i = 0; i < 50; i++) {
    const host = document.createElement("div"); document.body.append(host); hosts.push(host);
    createRoot(dispose => { disposers.push(dispose); reservedGroupHeight(() => `cleanup-${i}`, () => host, () => true); });
  }
  deliver(hosts.map(target => ({ target, borderBoxSize: [{ blockSize: 288 }] })) as unknown as ResizeObserverEntry[], {} as ResizeObserver);
  const reads = vi.spyOn(Element.prototype, "getBoundingClientRect");
  disposers.forEach(dispose => dispose());
  expect(reads).not.toHaveBeenCalled();
});

it("uses the nearest vertical scroller, including nested and horizontal-only containers", () => {
  const outer = document.createElement("div"), horizontal = document.createElement("div"), inner = document.createElement("div");
  outer.style.overflowY = "auto";
  horizontal.style.overflowX = "auto";
  outer.append(horizontal); horizontal.append(inner); document.body.append(outer);
  expect(scrollContainer(inner)).toBe(outer);
  horizontal.style.overflowY = "scroll";
  expect(scrollContainer(inner)).toBe(horizontal);
});

it.each([false, true])("preserves above-view content without doubling native anchoring (%s)", (native) => {
  const root = document.createElement("div"), host = document.createElement("div");
  root.append(host); document.body.append(root);
  root.scrollTop = 500;
  let bottom = -100;
  vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0 } as DOMRect);
  vi.spyOn(host, "getBoundingClientRect").mockImplementation(() => ({ bottom: bottom - (root.scrollTop - 500) }) as DOMRect);
  preserveAboveViewport(host, () => {
    bottom += 96;
    if (native) root.scrollTop += 96;
  }, root);
  expect(root.scrollTop).toBe(596);
  expect(host.getBoundingClientRect().bottom).toBe(-100);
});

it.each([10, 200])("leaves a visible or below-view replacement to its scroll owner (%s)", bottom => {
  const root = document.createElement("div"), host = document.createElement("div");
  vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0 } as DOMRect);
  vi.spyOn(host, "getBoundingClientRect").mockReturnValue({ bottom } as DOMRect);
  const change = vi.fn();
  preserveAboveViewport(host, change, root);
  expect(change).toHaveBeenCalledOnce();
  expect(root.scrollTop).toBe(0);
});

it("restores a resource update after its enclosing reactive batch flushes", async () => {
  const root = document.createElement("div"), host = document.createElement("div");
  root.append(host); document.body.append(root);
  root.scrollTop = 500;
  let bottom = -100;
  vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0 } as DOMRect);
  vi.spyOn(host, "getBoundingClientRect").mockImplementation(() => ({ bottom: bottom - (root.scrollTop - 500) }) as DOMRect);
  preserveAboveViewport(host, () => {}, root, true);
  // Solid flushes the DOM after the resource setter has returned.
  bottom += 256;
  await Promise.resolve();
  expect(root.scrollTop).toBe(756);
  expect(host.getBoundingClientRect().bottom).toBe(-100);
});

it("does not hit-test or force a second layout when source reads complete below the viewport", async () => {
  const root = document.createElement("div");
  const reader = document.createElement("div");
  root.append(reader);
  document.body.append(root);
  const viewport = vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 800, left: 0, width: 900 } as DOMRect);
  const anchor = vi.spyOn(reader, "getBoundingClientRect").mockReturnValue({ top: 100 } as DOMRect);
  const hit = vi.fn(() => reader);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: hit });
  const hosts = Array.from({ length: 28 }, () => {
    const host = document.createElement("div");
    root.append(host);
    return { host, rect: vi.spyOn(host, "getBoundingClientRect").mockReturnValue({ top: 900, bottom: 930 } as DOMRect) };
  });
  const publish = vi.fn();
  try {
    for (const { host } of hosts) {
      preserveAboveViewport(host, publish, root, true);
      await Promise.resolve();
    }
    expect(publish).toHaveBeenCalledTimes(hosts.length);
    expect(root.scrollTop).toBe(0);
    expect(viewport).toHaveBeenCalledTimes(hosts.length);
    for (const { rect } of hosts) expect(rect).toHaveBeenCalledOnce();
    expect(hit).not.toHaveBeenCalled();
    expect(anchor).not.toHaveBeenCalled();
  } finally { Reflect.deleteProperty(document, "elementFromPoint"); }
});

it("coalesces 50 deferred resource updates into one snapshot and one scroll write per root", async () => {
  const root = document.createElement("div"), reader = document.createElement("div");
  reader.className = "block-main";
  root.append(reader); document.body.append(root);
  let growth = 0, scroll = 0, writes = 0;
  Object.defineProperty(root, "scrollTop", { configurable: true, get: () => scroll, set: value => { writes++; scroll = value; } });
  const viewport = vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0, left: 0, width: 400 } as DOMRect);
  const anchor = vi.spyOn(reader, "getBoundingClientRect").mockImplementation(() => ({ top: 20 + growth - scroll }) as DOMRect);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => reader });
  const rects = [];
  for (let i = 0; i < 50; i++) {
    const host = document.createElement("div"); root.prepend(host);
    rects.push(vi.spyOn(host, "getBoundingClientRect").mockReturnValue({ bottom: -100 } as DOMRect));
    preserveAboveViewport(host, () => { growth += 10; }, root, true);
  }
  try {
    await Promise.resolve();
    expect(scroll).toBe(500);
    expect(writes).toBe(1);
    expect(viewport).toHaveBeenCalledOnce();
    expect(anchor).toHaveBeenCalledTimes(2);
    expect(rects.reduce((sum, rect) => sum + rect.mock.calls.length, 0)).toBe(1);
  } finally { Reflect.deleteProperty(document, "elementFromPoint"); }
});

it("does not scroll when a containing spacer absorbs the nested group's growth", () => {
  const root = document.createElement("div"), host = document.createElement("div"), reader = document.createElement("div");
  reader.className = "block-main";
  root.append(host, reader); document.body.append(root);
  root.scrollTop = 500;
  let bottom = -100;
  vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0, left: 0, width: 400 } as DOMRect);
  vi.spyOn(host, "getBoundingClientRect").mockImplementation(() => ({ bottom }) as DOMRect);
  vi.spyOn(reader, "getBoundingClientRect").mockReturnValue({ top: 20 } as DOMRect);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => reader });
  try {
    preserveAboveViewport(host, () => { bottom += 96; }, root);
    expect(root.scrollTop).toBe(500);
  } finally { Reflect.deleteProperty(document, "elementFromPoint"); }
});

it("anchors on flowing content, not on a pinned ancestor header at the top of the view (GH #944)", async () => {
  const root = document.createElement("div"), host = document.createElement("div"), pinned = document.createElement("button"), reader = document.createElement("div");
  const sticky = document.createElement("div"); sticky.className = "sticky-ancestor"; sticky.append(pinned);
  reader.className = "block-main";
  root.append(host, sticky, reader); document.body.append(root);
  let scroll = 500, shrink = 0;
  Object.defineProperty(root, "scrollTop", { configurable: true, get: () => scroll, set: value => { scroll = value; } });
  vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0, left: 0, width: 400 } as DOMRect);
  vi.spyOn(host, "getBoundingClientRect").mockReturnValue({ bottom: -100 } as DOMRect);
  vi.spyOn(pinned, "getBoundingClientRect").mockReturnValue({ top: 48 } as DOMRect);
  vi.spyOn(reader, "getBoundingClientRect").mockImplementation(() => ({ top: 200 - shrink - (scroll - 500) }) as DOMRect);
  // WebKit hands back the pinned header at the top, the reader a little lower.
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: (_x: number, y: number) => y < 10 ? pinned : reader });
  try {
    preserveAboveViewport(host, () => {}, root, true);
    shrink = 84; // the answer above shrank; the reader moved up by 84
    await Promise.resolve();
    expect(scroll).toBe(416);
  } finally { Reflect.deleteProperty(document, "elementFromPoint"); }
});

// GH #944 residual (review of 4ed1c2f7b, traced on WebKitGTK): fixed probes at 2, 32 and 64 px found no anchor when the
// rerunning query straddled the view top (every probe hit its own rows) or when three pinned ancestors (30 px each)
// covered all three probes, and an anchor inside a second query that reruns in the same turn moved with that query.
// Round 2: six or more pins plus the queries used up an 8-probe budget that counted skips, and a hit in a second query's
// table (no row of its own inside the query) promoted to the row hosting that query, which misses that query's change.
// Geometry is stubbed (jsdom has no layout): view 0..600, pins stacked at the top, the queries, then the reader.
describe("GH #944: the reader's text holds when a query above it reruns", () => {
  const cases = [0, 1, 2, 3, 4, 6, 8].flatMap(pins => (["straddle", "above"] as const).flatMap(place => [
    { pins, place, queries: 1, result: "list" }, { pins, place, queries: 2, result: "list" }, { pins, place, queries: 2, result: "table" }]));
  it.each(cases)("$pins pins, the query $place the top, $queries queries changing, the second a $result", async ({ pins, place, queries, result }) => {
    const root = document.createElement("div"), reader = document.createElement("div");
    reader.className = "block-main";
    const query = (): [HTMLElement, HTMLElement] => {
      const host = document.createElement("div"), row = document.createElement("div");
      host.className = "query-block"; row.className = "block-main"; host.append(row);
      return [host, row];
    };
    const [a, aRow] = query(), [b, bRow] = query();
    // A table result: the hit is a cell inside the query, and the nearest row is the one hosting the query.
    const hostRow = document.createElement("div"), cell = document.createElement("td");
    hostRow.className = "block-main";
    if (result === "table") { bRow.replaceWith(cell); hostRow.append(b); }
    const headers = Array.from({ length: pins }, () => {
      const sticky = document.createElement("div"), button = document.createElement("button");
      sticky.className = "sticky-ancestor"; sticky.append(button);
      return sticky;
    });
    root.append(...headers, a, ...(queries === 2 ? [result === "table" ? hostRow : b] : []), reader); document.body.append(root);
    let scroll = 500, shrinkA = 0, shrinkB = 0;
    Object.defineProperty(root, "scrollTop", { configurable: true, get: () => scroll, set: value => { scroll = value; } });
    const pinned = pins * 30;
    const geometry = () => {
      const shift = scroll - 500;
      const aTop = (place === "above" ? -400 : -200) - shift;
      const aBottom = (place === "above" ? -100 : pinned + 100) - shrinkA - shift;
      const bTop = (place === "above" ? pinned + 5 : pinned + 110) - shrinkA - shift, bBottom = bTop + 100 - shrinkB;
      const reader = (place === "above" ? pinned + 10 : pinned + 110) + (queries === 2 ? 105 : 0) - shrinkA - shrinkB - shift;
      return { aTop, aBottom, bTop, bBottom, reader };
    };
    vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 600, left: 0, width: 400 } as DOMRect);
    headers.forEach((sticky, i) => vi.spyOn(sticky, "getBoundingClientRect").mockReturnValue({ top: i * 30, bottom: i * 30 + 30 } as DOMRect));
    vi.spyOn(a, "getBoundingClientRect").mockImplementation(() => ({ top: geometry().aTop, bottom: geometry().aBottom }) as DOMRect);
    vi.spyOn(b, "getBoundingClientRect").mockImplementation(() => ({ top: geometry().bTop, bottom: geometry().bBottom }) as DOMRect);
    vi.spyOn(reader, "getBoundingClientRect").mockImplementation(() => ({ top: geometry().reader }) as DOMRect);
    vi.spyOn(hostRow, "getBoundingClientRect").mockImplementation(() => ({ top: geometry().bTop }) as DOMRect);
    for (const row of [aRow, bRow]) vi.spyOn(row, "getBoundingClientRect").mockImplementation(() => ({ top: (row === aRow ? geometry().aTop : geometry().bTop) }) as DOMRect);
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: (_x: number, y: number) => {
      const g = geometry();
      if (y < pinned) return headers[Math.floor(y / 30)].firstElementChild;
      if (y >= g.aTop && y < g.aBottom) return aRow;
      if (queries === 2 && y >= g.bTop && y < g.bBottom) return result === "table" ? cell : bRow;
      return y >= g.reader ? reader : root;
    } });
    try {
      const before = geometry().reader;
      expect(before).toBeLessThan(600);
      preserveAboveViewport(a, () => { shrinkA = 84; }, root, true);
      if (queries === 2) preserveAboveViewport(b, () => { shrinkB = 40; }, root, true);
      await Promise.resolve();
      expect(geometry().reader).toBe(before);
    } finally { Reflect.deleteProperty(document, "elementFromPoint"); }
  });
});

// GH #944 round 3 (review of a305636c1, finding 1): the query box's 2 px margin belongs to the authored row hosting it,
// so a probe 1 px below a skipped query lands on that host row. With two changing queries and the upper one publishing
// first, the chooser anchored on the second query's host row, whose top does not move with that query (40 px drift);
// with a short first host row near the bottom, the next 32 px step left the view while reader text was visible (84 px).
// Geometry (jsdom has no layout): view 0..600, pins stacked at the top, then host row X (2 px margin, query A, 2 px
// margin), host row Y (likewise, query B), then the reader. The hit in a query is the result shape named.
describe("GH #944: a changing query's boundary includes its host row's margin", () => {
  const shapes = {
    list: () => Object.assign(document.createElement("div"), { className: "block-main" }),
    table: () => { const table = document.createElement("table"), cell = document.createElement("td"); table.append(cell); return cell; },
    board: () => Object.assign(document.createElement("div"), { className: "query-board-card" }),
    page: () => Object.assign(document.createElement("a"), { className: "page-ref" }),
    search: () => Object.assign(document.createElement("span"), { className: "search-result" }),
    chrome: () => Object.assign(document.createElement("div"), { className: "query-header" }),
  };
  type Shape = keyof typeof shapes;
  const occurrence = (shape: Shape) => {
    const row = document.createElement("div"), content = document.createElement("div"), query = document.createElement("div");
    const result = shapes[shape]();
    row.className = "block-main"; content.className = "block-content-wrapper"; query.className = "query-block";
    query.append(result.closest("table") ?? result); content.append(query); row.append(content);
    return { row, content, query, result };
  };
  /** Builds the page and its stubbed layout; returns the reader's top and the counts of hit tests and rect reads. */
  const page = (opts: { pins: number; place: "straddle" | "above"; shape: Shape }) => {
    const root = document.createElement("div"), reader = document.createElement("div"), filler = document.createElement("div");
    reader.className = "block-main"; filler.className = "block-main";
    const A = occurrence("list"), B = occurrence(opts.shape);
    const headers = Array.from({ length: opts.pins }, () => {
      const sticky = document.createElement("div"), button = document.createElement("button");
      sticky.className = "sticky-ancestor"; sticky.append(button);
      return sticky;
    });
    root.append(...headers, A.row, filler, B.row, reader); document.body.append(root);
    const state = { scroll: 500, shrinkA: 0, shrinkB: 0, hits: 0, writes: 0 };
    Object.defineProperty(root, "scrollTop", { configurable: true, get: () => state.scroll, set: value => { state.writes++; state.scroll = value; } });
    const P = opts.pins * 30;
    const geometry = () => {
      const shift = state.scroll - 500;
      const aTop = (opts.place === "above" ? -400 : -200) - shift;
      const aBottom = (opts.place === "above" ? -2 : P + 100) - state.shrinkA - shift;
      const xBottom = aBottom + 2;
      // Above: a plain row under the pins (never hit), so the second host row starts at the pins' edge.
      const fillBottom = xBottom + (opts.place === "above" ? P : 0);
      const bTop = fillBottom + 2, bBottom = bTop + 100 - state.shrinkB, yBottom = bBottom + 2;
      return { aTop, aBottom, xBottom, fillBottom, bTop, bBottom, yBottom, reader: yBottom };
    };
    const rect = (el: Element, f: () => { top: number; bottom: number }) => vi.spyOn(el, "getBoundingClientRect").mockImplementation(() => f() as DOMRect);
    const rects = [
      vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 600, left: 0, width: 400 } as DOMRect),
      ...headers.map((sticky, i) => vi.spyOn(sticky, "getBoundingClientRect").mockReturnValue({ top: i * 30, bottom: i * 30 + 30 } as DOMRect)),
      rect(A.query, () => ({ top: geometry().aTop, bottom: geometry().aBottom })),
      rect(A.row, () => ({ top: geometry().aTop - 2, bottom: geometry().xBottom })),
      rect(A.result, () => ({ top: geometry().aTop, bottom: geometry().aBottom })),
      rect(filler, () => ({ top: geometry().xBottom, bottom: geometry().fillBottom })),
      rect(B.query, () => ({ top: geometry().bTop, bottom: geometry().bBottom })),
      rect(B.row, () => ({ top: geometry().bTop - 2, bottom: geometry().yBottom })),
      rect(B.result, () => ({ top: geometry().bTop, bottom: geometry().bBottom })),
      rect(reader, () => ({ top: geometry().reader, bottom: geometry().reader + 400 })),
    ];
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: (_x: number, y: number) => {
      state.hits++;
      const g = geometry();
      if (y < P) return headers[Math.floor(y / 30)].firstElementChild;
      if (y >= g.aTop && y < g.aBottom) return A.result;
      if (y >= g.aTop - 2 && y < g.xBottom) return A.content;
      if (y >= g.xBottom && y < g.fillBottom) return filler;
      if (y >= g.bTop && y < g.bBottom) return B.result;
      if (y >= g.bTop - 2 && y < g.yBottom) return B.content;
      return y >= g.reader ? reader : root;
    } });
    const reads = () => rects.reduce((sum, spy) => sum + spy.mock.calls.length, 0);
    return { root, A, B, state, geometry, reads, resetReads: () => rects.forEach(spy => spy.mockClear()) };
  };
  afterEach(() => { Reflect.deleteProperty(document, "elementFromPoint"); });

  const cases = ([0, 3] as const).flatMap(pins => (["straddle", "above"] as const).flatMap(place =>
    (Object.keys(shapes) as Shape[]).flatMap(shape => (["upper", "lower"] as const).map(first => ({ pins, place, shape, first })))));
  it.each(cases)("$pins pins, the first query $place the top, the second a $shape, the $first one publishing first",
    async ({ pins, place, shape, first }) => {
      const { root, A, B, state, geometry } = page({ pins, place, shape });
      const before = geometry().reader;
      expect(before).toBeLessThan(600);
      const publishA = () => preserveAboveViewport(A.query, () => { state.shrinkA = 84; }, root, true);
      const publishB = () => preserveAboveViewport(B.query, () => { state.shrinkB = 40; }, root, true);
      if (first === "upper") { publishA(); publishB(); } else { publishB(); publishA(); }
      await Promise.resolve();
      expect(geometry().reader).toBe(before);
    });

  it.each([0, 3])("a short host row near the bottom of the view still anchors the visible reader text below it (gap %s px)", async gap => {
    const root = document.createElement("div"), reader = document.createElement("div");
    reader.className = "block-main";
    const A = occurrence("list");
    root.append(A.row, reader); document.body.append(root);
    let scroll = 500, shrink = 0;
    Object.defineProperty(root, "scrollTop", { configurable: true, get: () => scroll, set: value => { scroll = value; } });
    const aBottom = () => 575 - shrink - (scroll - 500), readerTop = () => aBottom() + 2 + gap;
    vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 600, left: 0, width: 400 } as DOMRect);
    vi.spyOn(A.query, "getBoundingClientRect").mockImplementation(() => ({ top: -200, bottom: aBottom() }) as DOMRect);
    vi.spyOn(A.row, "getBoundingClientRect").mockImplementation(() => ({ top: -202, bottom: aBottom() + 2 }) as DOMRect);
    vi.spyOn(reader, "getBoundingClientRect").mockImplementation(() => ({ top: readerTop(), bottom: readerTop() + 400 }) as DOMRect);
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: (_x: number, y: number) =>
      y < aBottom() ? A.result : y < aBottom() + 2 ? A.content : y >= readerTop() ? reader : root });
    const before = readerTop();
    preserveAboveViewport(A.query, () => { shrink = 84; }, root, true);
    await Promise.resolve();
    expect(readerTop()).toBe(before);
  });

  // Review finding 2: cost scales with the obstructions crossed in the view, never with the page. Pinned for the
  // two-query page (the first query just above the view): hit tests and rect reads (view, query box, each pin, the host row crossed, the anchor) in one snapshot, one
  // scroll write.
  it.each([[4, 6, 8], [8, 10, 12], [11, 13, 15]])("%s pins: %s hit tests and %s rect reads per snapshot", async (pins, hits, reads) => {
    const p = page({ pins, place: "above", shape: "table" });
    p.resetReads();
    preserveAboveViewport(p.A.query, () => { p.state.shrinkA = 84; }, p.root, true);
    expect(p.state.hits).toBe(hits);
    expect(p.reads()).toBe(reads);
    p.resetReads();
    await Promise.resolve();
    expect(p.reads()).toBe(1); // the restore reads the anchor only
    expect(p.state.writes).toBe(1);
  });
});
