import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For } from "solid-js";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { resetNearObserverForTests } from "../lazyObserve";

// Beta 0.9.0-beta.5 bench regression: opening a large page parsed twice the bodies
// it shows (bigLoad parse misses 12 -> 24, perf A/B over budget). Bodies near the
// view mount before they are seen only ABOVE it, where a late mount would move the
// rows being read (GH #810); below the view a mount moves nothing the reader sees.
//
// jsdom has no layout, so this stub IntersectionObserver applies the observer's own
// root and rootMargin to a fixed row geometry, as Chromium does: rows of ROW px,
// row 0 at the top of a VIEW px pane, rows above it at negative offsets.
const ROW = 75;
const VIEW = 900;
const FIRST = -20;
const LAST = 39;

function marginOf(spec: string | undefined): { top: number; bottom: number } {
  const parts = (spec ?? "0px").trim().split(/\s+/).map((p) => Number.parseFloat(p) || 0);
  const [top, , bottom = top] = parts.length === 1 ? [parts[0], 0, parts[0]] : parts;
  return { top, bottom };
}

function rectAt(top: number, height: number): DOMRect {
  return { top, bottom: top + height, left: 0, right: 800, width: 800, height, x: 0, y: top, toJSON() { return {}; } } as DOMRect;
}

const cleanups: (() => void)[] = [];
afterEach(() => {
  while (cleanups.length) cleanups.pop()!();
  resetNearObserverForTests();
});
beforeAll(async () => { await initParser(); });

describe("a cold open of a large page parses only the bodies in or above the view", () => {
  it("leaves bodies below the view unparsed and mounts those just above it (GH #810)", async () => {
    const pane = document.createElement("div");
    pane.className = "main-content";
    pane.style.overflowY = "auto";
    document.body.append(pane);
    cleanups.push(() => pane.remove());

    const original = Element.prototype.getBoundingClientRect;
    Element.prototype.getBoundingClientRect = function (this: Element) {
      if (this === pane) return rectAt(0, VIEW);
      const row = this.closest("[data-row]");
      return row ? rectAt(Number(row.getAttribute("data-row")) * ROW, ROW) : rectAt(0, 0);
    };
    cleanups.push(() => { Element.prototype.getBoundingClientRect = original; });

    vi.stubGlobal("IntersectionObserver", class {
      constructor(private cb: IntersectionObserverCallback, private options: IntersectionObserverInit = {}) {}
      observe(el: Element) {
        queueMicrotask(() => {
          const root = this.options.root as Element | null | undefined;
          const view = root ? root.getBoundingClientRect() : rectAt(0, VIEW);
          const m = marginOf(this.options.rootMargin);
          const bounds = rectAt(view.top - m.top, view.height + m.top + m.bottom);
          const box = el.getBoundingClientRect();
          const isIntersecting = box.bottom > bounds.top && box.top < bounds.bottom;
          this.cb([{ target: el, isIntersecting, boundingClientRect: box, rootBounds: bounds,
            intersectionRatio: isIntersecting ? 1 : 0, intersectionRect: box, time: 0 } as IntersectionObserverEntry], this as never);
        });
      }
      unobserve() {}
      disconnect() {}
    });
    cleanups.push(() => vi.unstubAllGlobals());

    const w = window as unknown as { __tineBench?: boolean; __tineParseStats?: { calls: number; hits: number; misses: number } };
    w.__tineBench = true;
    cleanups.push(() => { delete w.__tineBench; });

    const rows = Array.from({ length: LAST - FIRST + 1 }, (_, i) => FIRST + i);
    const dispose = render(() => <For each={rows}>{(row) =>
      <div data-row={row}><AstBody raw={`Row ${row} **body** near-parse ${Math.random()}`} blockId={`near-parse-${row}`} /></div>}</For>, pane);
    cleanups.push(dispose);
    w.__tineParseStats = { calls: 0, hits: 0, misses: 0 };
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));

    const mounted = (row: number) => !pane.querySelector(`[data-row="${row}"] .ast-deferred`);
    const inView = rows.filter((r) => r >= 0 && r * ROW < VIEW);
    const justAbove = rows.filter((r) => r < 0 && (r + 1) * ROW > -1200);
    const below = rows.filter((r) => r * ROW >= VIEW);
    expect(inView.every(mounted), "every body in view renders").toBe(true);
    expect(justAbove.every(mounted), "GH #810: bodies within the margin above the view render before they are seen").toBe(true);
    expect(below.filter(mounted), "bodies below the view stay unparsed placeholders on a cold open").toEqual([]);
    expect(w.__tineParseStats?.misses, "cold parses = bodies in or just above the view").toBe(inView.length + justAbove.length);
  });
});
