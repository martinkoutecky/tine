import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { AstBody } from "../render/body";
import { renderedBlockText } from "../render/renderedText";
import { resetStore } from "../document";
import { doc, setDoc, type Node as StoreNode } from "../document/model";
import { installKeybindings } from "../keybindings";
import { setGraphMeta } from "../graphSession";
import { initHoverPreviewSettings } from "../hoverPreviewSettings";
import type { GraphMeta } from "../types";
import { Block } from "./Block";

// GH #1052: a nested page reference is ONE link, shown exactly as before; only
// the page a click opens depends on the part under the pointer
// (Page.nestedReference.test.tsx covers the destinations).

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  window.getSelection()?.removeAllRanges();
  resetStore();
  setGraphMeta(null);
  document.body.replaceChildren();
});

function mountBody(raw: string) {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <div class="block-content"><AstBody raw={raw} /></div>, host);
  return { root: host, dispose };
}

/** `[[a0 [[a1 … [[a{depth-1}]] …]]]]`. */
const nested = (depth: number): string => {
  let out = `a${depth - 1}`;
  for (let i = depth - 2; i >= 0; i--) out = `a${i} [[${out}]] t`;
  return `[[${out}]]`;
};

describe("a nested reference renders as one link (GH #1052)", () => {
  it("ten levels deep: one link element, and element construction linear in depth", () => {
    const built = (raw: string) => {
      const clone = vi.spyOn(Node.prototype, "cloneNode");
      const create = vi.spyOn(document, "createElement");
      const { root, dispose } = mountBody(raw);
      const count = clone.mock.calls.length + create.mock.calls.length;
      const links = root.querySelectorAll("a.page-ref, a.tag").length;
      const text = root.textContent;
      dispose();
      clone.mockRestore();
      create.mockRestore();
      return { count, links, text };
    };
    const shallow = built(nested(5));
    const deep = built(nested(10));
    expect(deep.links).toBe(1);
    expect(deep.text).toBe(`[[${nested(10)}]]`); // show brackets is on by default
    // Five more levels add five more parts: a small constant each, not a doubling.
    expect(deep.count - shallow.count).toBeLessThanOrEqual(5 * 8);
  });

  it("hovering onto the inner part arms exactly one preview read", async () => {
    vi.useFakeTimers();
    vi.spyOn(backend(), "getAppString").mockImplementation(async (_key, fallback) => fallback);
    initHoverPreviewSettings();
    await vi.advanceTimersByTimeAsync(0);
    const read = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const { root, dispose } = mountBody("[[Lorem [[Impsum]]]]");
    try {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      let inner: Element | null = null;
      for (let n = walker.nextNode(); n && !inner; n = walker.nextNode()) if (n.textContent!.includes("Impsum")) inner = n.parentElement;
      expect(inner!.closest("a.page-ref")).not.toBeNull();
      // A pointer entering the inner text enters every ancestor it was not in.
      const path: Element[] = [];
      for (let el: Element | null = inner!; el && el !== root; el = el.parentElement) path.unshift(el);
      for (const el of path) el.dispatchEvent(new PointerEvent("pointerenter", { bubbles: false, pointerType: "mouse", isPrimary: true }));
      await vi.advanceTimersByTimeAsync(2000);
      expect(read).toHaveBeenCalledTimes(1);
    } finally { dispose(); }
  });

  it.each([true, false])("with show brackets %s the link shows what Find and Rendered Copy read", (brackets) => {
    setGraphMeta({ show_brackets: brackets } as GraphMeta);
    const raw = "[[Lorem [[Impsum]]]]";
    const { root, dispose } = mountBody(raw);
    try {
      const shown = root.querySelector("a.page-ref")!.textContent;
      // As before #1052: the token's text, wrapped in the bracket pair when shown.
      expect(shown).toBe(brackets ? "[[[[Lorem [[Impsum]]]]]]" : "[[Lorem [[Impsum]]]]");
      const projected = renderedBlockText(raw, "md", { typographicGlyphs: false, stripLinks: !brackets, removeTags: false, removeProperties: false });
      expect(shown).toBe(projected);
    } finally { dispose(); }
  });

  it("Delete over a selection ending inside the inner part of an iconned page keeps the unselected text", async () => {
    vi.spyOn(backend(), "pageIcons").mockImplementation(async (names: string[]) =>
      Object.fromEntries(names.filter((n) => n === "Target").map((n) => [n, "🎯"])));
    const raw = "prefix [[abc [[Target]] tail]]";
    const node: StoreNode = { id: "a", raw, collapsed: false, parent: null, page: "P", children: [] };
    setDoc({
      byId: { a: node },
      pages: [{ name: "P", kind: "page", title: "P", preBlock: null, roots: ["a"], format: "md", readOnly: false, guide: false }],
      feed: ["P"],
      loaded: true,
    });
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <Block id="a" />, host);
    const disposeKeys = installKeybindings();
    try {
      for (let i = 0; i < 30; i++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
      const wrapper = host.querySelector(".block-content-wrapper")!;
      const texts: Text[] = [];
      const walker = document.createTreeWalker(wrapper, NodeFilter.SHOW_TEXT);
      for (let n = walker.nextNode(); n; n = walker.nextNode()) texts.push(n as Text);
      const from = texts.find((t) => t.data.startsWith("prefix"))!;
      const to = texts.find((t) => t.data.includes("Target"))!;
      const range = document.createRange();
      range.setStart(from, 3);
      range.setEnd(to, to.data.indexOf("Tar") + 3);
      window.getSelection()!.removeAllRanges();
      window.getSelection()!.addRange(range);
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
      // The end inside the link maps to the link's nearer edge (its start), as
      // for any page reference: only `fix ` was selected outside it.
      expect(doc.byId["a"].raw).toBe("pre[[abc [[Target]] tail]]");
    } finally {
      disposeKeys();
      dispose();
    }
  });
});
