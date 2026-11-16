import { afterEach, expect, it, vi } from "vitest";
import { batchViewportChanges, cachedScrollContainer, preserveAboveViewport, scrollContainer } from "./scrollContainer";
import { observeNear, resetNearObserverForTests } from "./lazyObserve";
import { createSignal, createRoot } from "solid-js";
import { render } from "solid-js/web";
import { reservedGroupHeight } from "./reservedGroupHeight";

afterEach(() => { resetNearObserverForTests(); vi.unstubAllGlobals(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

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
