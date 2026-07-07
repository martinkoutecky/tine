import { beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { isBuiltinHidden } from "../editor/properties";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { editorOffsetFromRenderedRange } from "./spans";

beforeAll(async () => {
  await initParser();
});

function mountedBody(raw: string): { root: HTMLElement; dispose: () => void } {
  const host = document.createElement("div");
  const dispose = render(() => (
    <div class="block-content">
      <AstBody raw={raw} />
    </div>
  ), host);
  return { root: host.firstElementChild as HTMLElement, dispose };
}

function textRange(root: Node, needle: string, offset: number): Range {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let node: Text | null;
  while ((node = walker.nextNode() as Text | null)) {
    const idx = (node.textContent ?? "").indexOf(needle);
    if (idx !== -1) {
      const range = document.createRange();
      range.setStart(node, idx + offset);
      return range;
    }
  }
  throw new Error(`text node not found: ${needle}`);
}

function elementRange(el: Element, offset: number): Range {
  const range = document.createRange();
  range.setStart(el, offset);
  return range;
}

describe("click-to-caret span mapping", () => {
  it("maps marked-up block regions back to exact editor offsets", () => {
    const raw = "**bold** and [[link]] text";
    const { root, dispose } = mountedBody(raw);
    try {
      expect(editorOffsetFromRenderedRange(root, textRange(root, "bold", 2), raw, isBuiltinHidden))
        .toBe(raw.indexOf("bold") + 2);
      expect(editorOffsetFromRenderedRange(root, textRange(root, " and ", 3), raw, isBuiltinHidden))
        .toBe(raw.indexOf(" and ") + 3);

      const link = root.querySelector("a.page-ref");
      expect(link).toBeTruthy();
      expect(editorOffsetFromRenderedRange(root, elementRange(link!, 0), raw, isBuiltinHidden))
        .toBe(raw.indexOf("[[link]]"));

      expect(editorOffsetFromRenderedRange(root, textRange(root, " text", 2), raw, isBuiltinHidden))
        .toBe(raw.indexOf(" text") + 2);
    } finally {
      dispose();
    }
  });

  it("puts the caret AFTER a trailing link when clicked to its right (GH #34)", () => {
    // Clicking past the end of a block that ends in a link used to snap to the
    // span START (caret before the first `[`); it must land after `]]` instead.
    const raw = "asd [[xyz]]";
    const { root, dispose } = mountedBody(raw);
    try {
      const link = root.querySelector("a.page-ref");
      expect(link).toBeTruthy();
      // A click on the right edge resolves into the link's text at its end.
      expect(editorOffsetFromRenderedRange(root, elementRange(link!, link!.childNodes.length), raw, isBuiltinHidden))
        .toBe(raw.length);
      // ...while a click on the left edge still lands before it.
      expect(editorOffsetFromRenderedRange(root, elementRange(link!, 0), raw, isBuiltinHidden))
        .toBe(raw.indexOf("[[xyz]]"));
    } finally {
      dispose();
    }
  });

  it("accounts for hidden property lines between rendered regions", () => {
    const raw = "**bold**\nid:: abc\nplain";
    const { root, dispose } = mountedBody(raw);
    try {
      expect(editorOffsetFromRenderedRange(root, textRange(root, "plain", 2), raw, isBuiltinHidden))
        .toBe("**bold**\npl".length);
    } finally {
      dispose();
    }
  });
});
