import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { clearTransientLayersForTest } from "../transientLayers";
import { bumpDataRev } from "../dataRevision";
import { ensurePageLoaded, node, resetStore } from "../document";

// GH #735: the hover popup shows the page the way the page itself renders:
// fenced code, admonitions, quotes and existing folds, not flattened text lines.
const OPEN_MS = 1000;

beforeAll(async () => { await initParser(); });
beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => {
  resetStore();
  clearTransientLayersForTest();
  vi.useRealTimers();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await Promise.resolve();
  await Promise.resolve();
}
async function waitFor(fn: () => boolean, timeout = 1000): Promise<boolean> {
  for (let elapsed = 0; elapsed < timeout; elapsed += 25) {
    if (fn()) return true;
    await advance(25);
  }
  return fn();
}
const popup = () => document.body.querySelector<HTMLElement>(".peek-popup");

async function openPeek(blocks: unknown[], source: object = {}): Promise<{ card: HTMLElement; dispose: () => void }> {
  vi.spyOn(backend(), "getPage").mockResolvedValue({
    id: "pages/Probe.md", name: "Probe", title: "Probe", kind: "page", format: "md", pre_block: "", blocks, ...source,
  } as never);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <div class="block-content"><AstBody raw="see [[Probe]]" /></div>, host);
  const link = host.querySelector("a.page-ref")!;
  link.dispatchEvent(new PointerEvent("pointerenter", { bubbles: false, pointerType: "mouse", isPrimary: true }));
  link.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
  await advance(OPEN_MS);
  expect(await waitFor(() => !!popup()?.querySelector(".ls-block"))).toBe(true);
  return { card: popup()!, dispose: () => { dispose(); host.remove(); } };
}
const blk = (id: string, raw: string, extra: object = {}) => ({ id, raw, collapsed: false, children: [], ...extra });

describe("GH #735: hover popup renders block content like the page", () => {
  it("fenced code keeps its code-block rendering and line structure", async () => {
    const { card, dispose } = await openPeek([blk("c1", "A snippet\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```")]);
    try {
      const pre = card.querySelector("pre.code-block");
      expect(pre).not.toBeNull();
      expect(pre!.textContent).toContain("println!");
      expect(card.textContent).not.toContain("```");
    } finally { dispose(); }
  });
  it("an Org-style admonition and a quote block render as such", async () => {
    const { card, dispose } = await openPeek([
      blk("n1", "#+BEGIN_NOTE\nremember this\n#+END_NOTE"),
      blk("q1", "#+BEGIN_QUOTE\nto be or not\n#+END_QUOTE"),
      blk("q2", "> a markdown quote"),
    ]);
    try {
      expect(card.querySelector(".callout.callout-note")).not.toBeNull();
      expect(card.querySelectorAll("blockquote.md-quote").length).toBeGreaterThanOrEqual(2);
      expect(card.textContent).not.toContain("BEGIN_NOTE");
      expect(card.textContent).not.toContain("BEGIN_QUOTE");
    } finally { dispose(); }
  });
  it("a collapsed block shows its source fold and can be opened", async () => {
    const { card, dispose } = await openPeek([
      blk("f1", "Folded parent\ncollapsed:: true", { collapsed: true, children: [blk("f2", "hidden child")] }),
      blk("o1", "Open parent", { children: [blk("o2", "visible child")] }),
    ]);
    try {
      expect(card.textContent).not.toContain("hidden child");
      expect(card.textContent).toContain("visible child");
      const folded = [...card.querySelectorAll<HTMLElement>(".ls-block.ref-block")].find((el) => el.dataset.blockId === "f1")!;
      expect(folded.querySelector(".bullet-container.bullet-closed")).not.toBeNull();
      folded.querySelector<HTMLElement>(".collapse-toggle")!.click();
      await advance(0);
      expect(card.textContent).toContain("hidden child");
    } finally { dispose(); }
  });
});

it("an open page preview refreshes its DTO folds when source data changes", async () => {
  const parent = blk("refresh-parent", "Parent", { children: [blk("refresh-child", "Child")] });
  const { card, dispose } = await openPeek([parent]);
  try {
    expect(card.textContent).toContain("Child");
    vi.mocked(backend().getPage).mockResolvedValue({
      id: "pages/Probe.md", name: "Probe", title: "Probe", kind: "page", format: "md", pre_block: null,
      blocks: [{ ...parent, collapsed: true }],
    });
    bumpDataRev();
    expect(await waitFor(() => !card.textContent?.includes("Child"))).toBe(true);
  } finally { dispose(); }
});

it("a page-link preview folds the answered source kind, including an alias of a journal", async () => {
  const blocks = [blk("journal-parent", "Parent", { children: [blk("journal-child", "Child")] })];
  const source = { id: "journals/2026_10_23.md", name: "Oct 23rd, 2026", title: "Oct 23rd, 2026",
    kind: "journal" as const, format: "md" as const, pre_block: null, blocks };
  ensurePageLoaded(source);
  const { card, dispose } = await openPeek(blocks, source);
  try {
    card.querySelector<HTMLElement>(".collapse-toggle")!.click();
    expect(await waitFor(() => node("journal-parent")?.collapsed === true)).toBe(true);
    expect(node("journal-parent")?.raw).toContain("collapsed:: true");
  } finally { dispose(); }
});

describe("GH #735: block-reference popup and live content", () => {
  it("a block-reference popup renders code and folds like the page popup", async () => {
    vi.spyOn(backend(), "previewBlock").mockResolvedValue({
      truncated: 0,
      group: { page: "Probe", kind: "page", blocks: [
        blk("r1", "Root\n```js\nconst x = 1;\n```", { collapsed: true, children: [blk("r2", "tucked away")] }),
      ] },
    } as never);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <div class="block-content"><AstBody raw="see ((58900000-0000-4000-8000-0000000000b1))" /></div>, host);
    try {
      const ref = host.querySelector(".block-ref")!;
      ref.dispatchEvent(new PointerEvent("pointerenter", { bubbles: false, pointerType: "mouse", isPrimary: true }));
      ref.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
      await advance(OPEN_MS);
      expect(await waitFor(() => !!popup()?.querySelector(".ls-block"))).toBe(true);
      expect(popup()!.querySelector("pre.code-block")).not.toBeNull();
      expect(popup()!.textContent).not.toContain("tucked away");
      vi.mocked(backend().previewBlock).mockResolvedValue({
        truncated: 0,
        group: { page: "Probe", kind: "page", blocks: [
          blk("r1", "Root", { children: [blk("r2", "tucked away")] }),
        ] },
      });
      bumpDataRev();
      expect(await waitFor(() => !!popup()?.textContent?.includes("tucked away"))).toBe(true);
    } finally { dispose(); host.remove(); }
  });
  it("a #+BEGIN_QUERY block in a popup is not executed as a live query", async () => {
    const { card, dispose } = await openPeek([
      blk("q1", "#+BEGIN_QUERY\n{:title \"Live\" :query [:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]]}\n#+END_QUERY"),
    ]);
    try {
      expect(card.querySelector(".query-block, .query-builder, .query-results")).toBeNull();
    } finally { dispose(); }
  });
});

describe("GH #735: hover popup size and delay match Logseq", () => {
  async function arm(raw: string) {
    vi.spyOn(backend(), "getPage").mockResolvedValue({
      id: "pages/Probe.md", name: "Probe", title: "Probe", kind: "page", format: "md", pre_block: "",
      blocks: Array.from({ length: 40 }, (_, i) => blk(`b${i}`, `line ${i}`)),
    } as never);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <div class="block-content"><AstBody raw={raw} /></div>, host);
    const link = host.querySelector("a.page-ref")!;
    return { link, dispose: () => { dispose(); host.remove(); } };
  }
  const enter = (el: Element) => {
    el.dispatchEvent(new PointerEvent("pointerenter", { bubbles: false, pointerType: "mouse", isPrimary: true }));
    el.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
  };
  const leave = (el: Element) => {
    el.dispatchEvent(new PointerEvent("pointerleave", { bubbles: false, pointerType: "mouse", isPrimary: true }));
    el.dispatchEvent(new MouseEvent("mouseleave", { bubbles: false }));
  };

  it("opens after Logseq's 1000 ms hover delay, not before", async () => {
    const { link, dispose } = await arm("see [[Probe]]");
    try {
      enter(link);
      await advance(900);
      await Promise.resolve();
      expect(popup()).toBeNull();
      await advance(100);
      expect(await waitFor(() => !!popup()?.querySelector(".ls-block"))).toBe(true);
    } finally { dispose(); }
  });
  it("closes 100 ms after the pointer leaves, like Logseq", async () => {
    const { link, dispose } = await arm("see [[Probe]]");
    try {
      enter(link);
      await advance(OPEN_MS);
      expect(await waitFor(() => !!popup())).toBe(true);
      leave(link);
      await advance(90);
      expect(popup()).not.toBeNull();
      await advance(20);
      expect(popup()).toBeNull();
    } finally { dispose(); }
  });
  it.each([[2160, 1080], [2000, 1000], [1400, 700], [1100, 600], [900, 600], [500, 484]])("caps at max(600px, half the window) in a %ipx-tall window (D-20261021-FA3)", async (innerHeight, cap) => {
    const old = window.innerHeight;
    Object.defineProperty(window, "innerHeight", { configurable: true, value: innerHeight });
    const { link, dispose } = await arm("see [[Probe]]");
    try {
      enter(link);
      await advance(OPEN_MS);
      expect(await waitFor(() => !!popup())).toBe(true);
      expect(parseFloat(popup()!.style.maxHeight)).toBe(cap);
    } finally { dispose(); Object.defineProperty(window, "innerHeight", { configurable: true, value: old }); }
  });
  it("re-places itself when its content settles to a different height", async () => {
    const old = window.innerHeight;
    const callbacks: Array<() => void> = [];
    class FakeResizeObserver { constructor(cb: () => void) { callbacks.push(cb); } observe() {} disconnect() {} }
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    (window as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 1100 });
    const { link, dispose } = await arm("see [[Probe]]");
    link.getBoundingClientRect = () => ({ top: 1000, bottom: 1020, left: 100, right: 160, width: 60, height: 20, x: 100, y: 1000, toJSON() {} }) as DOMRect;
    let height = 0;
    const heightSpy = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(() => height);
    try {
      enter(link);
      await advance(OPEN_MS);
      expect(await waitFor(() => !!popup())).toBe(true);
      const before = parseFloat(popup()!.style.top);
      height = 500;
      callbacks.forEach((cb) => cb());
      await advance(0);
      const after = parseFloat(popup()!.style.top);
      expect(before).toBe(1000 - 6 - 320); // fallback height until measured
      expect(after).toBe(1000 - 6 - 500); // opens above and grows upward, staying inside the window
    } finally {
      heightSpy.mockRestore();
      dispose();
      delete (window as unknown as { ResizeObserver?: unknown }).ResizeObserver;
      vi.unstubAllGlobals();
      Object.defineProperty(window, "innerHeight", { configurable: true, value: old });
    }
  });
  it("never exceeds a window shorter than the cap", async () => {
    const old = window.innerHeight;
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 400 });
    const { link, dispose } = await arm("see [[Probe]]");
    try {
      enter(link);
      await advance(OPEN_MS);
      expect(await waitFor(() => !!popup())).toBe(true);
      expect(parseFloat(popup()!.style.maxHeight)).toBe(384);
    } finally { dispose(); Object.defineProperty(window, "innerHeight", { configurable: true, value: old }); }
  });
});
