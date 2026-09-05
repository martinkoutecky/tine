import { beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { isBuiltinHidden } from "../editor/properties";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { clickBeyondRenderedEnd, codeCardOffsetFromRange, editorOffsetFromRenderedRange } from "./spans";
import { PaneContext, focusPane, paneRouter, resetPaneLayoutToSingle, splitPane } from "../panes";

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

  it("maps clicks inside inline code to the corresponding source character (GH #114)", () => {
    const raw = "before `a literal block` after";
    const { root, dispose } = mountedBody(raw);
    try {
      const codeStart = raw.indexOf("a literal block");
      for (const offset of [0, 2, 8, "a literal block".length]) {
        expect(
          editorOffsetFromRenderedRange(
            root,
            textRange(root, "a literal block", offset),
            raw,
            isBuiltinHidden,
          ),
        ).toBe(codeStart + offset);
      }
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

  // GH #465. This is the case that forced the geometric rule: unlike the
  // trailing link above, the span map here answers, and answers plausibly, so
  // no "mapping failed" fallback can rescue it. The block ends `…italics.*` and
  // the caret lands on byte 22, one before the closing delimiter.
  it("maps the end of trailing italic text to a spot BEFORE the invisible `*`", () => {
    const raw = "*some text in italics.*";
    const { root, dispose } = mountedBody(raw);
    try {
      const em = root.querySelector("em");
      expect(em).toBeTruthy();
      const end = editorOffsetFromRenderedRange(
        root,
        textRange(root, "some text in italics.", "some text in italics.".length),
        raw,
        isBuiltinHidden,
      );
      expect(end).toBe(raw.length - 1);
      // ...which is why a click past the final glyph must be recognised by where
      // it landed rather than by what the span map says. Interior precision has
      // to survive that: a deliberate click inside the italic text still maps
      // exactly, and must not be swept to the end.
      expect(editorOffsetFromRenderedRange(root, textRange(root, "some", 2), raw, isBuiltinHidden))
        .toBe(raw.indexOf("some") + 2);
    } finally {
      dispose();
    }
  });

  // Regression: the GH #465 geometric probe called Range.getClientRects, which
  // jsdom does not implement, so it THREW out of the click handler and took the
  // whole rendered-block edit gesture with it. With no layout engine there are
  // no line boxes to ask about, so it must decline and let the span mapping
  // answer, exactly as before #465.
  it("declines instead of throwing where the environment has no layout", () => {
    const { root, dispose } = mountedBody("*some text in italics.*");
    try {
      expect(clickBeyondRenderedEnd(root, 10_000, 10)).toBe(false);
    } finally {
      dispose();
    }
  });

  // GH #465: a `{{img … right}}` wrapper is `float: right`, so it is drawn hard
  // against the block's right edge, and being taller than the line it rides it
  // owns the bottom band. A single range over the whole block picks that box up
  // and then answers "where does the text end?" with the full content width,
  // which killed the past-the-end caret in that block. Measured in the running
  // app: wrapper right 1080 / bottom 545.2 against text right 665.5 / bottom
  // 542.2. jsdom has no layout, so the geometry here is supplied in the same
  // shape; what the test pins is that out-of-flow children are excluded.
  it("ignores a floated decoration when deciding where the text ends", () => {
    const root = document.createElement("div");
    root.className = "block-content";
    const badge = document.createElement("span");
    badge.style.float = "right";
    badge.textContent = "img";
    root.append(badge, document.createTextNode("some text in italics."));
    document.body.appendChild(root);

    const rects = new Map<Node, DOMRect>([
      [badge, new DOMRect(585.8, 10, 22.2, 26)],
      [root.lastChild!, new DOMRect(8, 13, 156.6, 19)],
    ]);
    const original = Range.prototype.getClientRects;
    // Only `selectNode` is used, so the range's start container/offset names the
    // node being measured.
    Range.prototype.getClientRects = function (this: Range) {
      const node = this.startContainer.childNodes[this.startOffset];
      const rect = rects.get(node);
      return [rect ?? new DOMRect(0, 0, 0, 0)] as unknown as DOMRectList;
    };
    try {
      // A click in the run-out after "italics." on the block's only line.
      expect(clickBeyondRenderedEnd(root, 300, 22)).toBe(true);
      // ...but a click still over the glyphs is not past the end.
      expect(clickBeyondRenderedEnd(root, 100, 22)).toBe(false);
      // Without the filter the float owns the bottom band and the same click is
      // not past the end — the pre-fix behaviour, and the necessity control.
      badge.style.float = "";
      expect(clickBeyondRenderedEnd(root, 300, 22)).toBe(false);
    } finally {
      Range.prototype.getClientRects = original;
      root.remove();
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

// GH #42: shift+click a [[page]] / block-ref opens it in the sidebar; the anchor
// must suppress the browser's native shift-range-selection (preventDefault on the
// shift mousedown) so text in the main editor isn't selected as a side effect.
describe("shift+click ref suppresses native selection (GH #42)", () => {
  // Solid delegates `mousedown` on `document`, so the host must be attached to
  // the live document tree for the handler to fire when we dispatch.
  function mountAttached(raw: string): { root: HTMLElement; dispose: () => void } {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => (
      <div class="block-content"><AstBody raw={raw} /></div>
    ), host);
    return { root: host, dispose: () => { dispose(); host.remove(); } };
  }
  function shiftMouseDefaultPrevented(el: Element, shift: boolean): boolean {
    const ev = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, shiftKey: shift });
    el.dispatchEvent(ev);
    return ev.defaultPrevented;
  }

  it("preventDefaults a shift+mousedown on a page-ref, but not a plain one", () => {
    const { root, dispose } = mountAttached("see [[Some Page]] now");
    try {
      const link = root.querySelector("a.page-ref");
      expect(link).toBeTruthy();
      expect(shiftMouseDefaultPrevented(link!, true)).toBe(true);
      expect(shiftMouseDefaultPrevented(link!, false)).toBe(false);
    } finally {
      dispose();
    }
  });

  it("preventDefaults a shift+mousedown on a #tag", () => {
    const { root, dispose } = mountAttached("tagged #project here");
    try {
      const tag = root.querySelector("a.tag");
      expect(tag).toBeTruthy();
      expect(shiftMouseDefaultPrevented(tag!, true)).toBe(true);
      expect(shiftMouseDefaultPrevented(tag!, false)).toBe(false);
    } finally {
      dispose();
    }
  });
});

describe("split-pane link navigation", () => {
  it("opens a middle-clicked page ref in the already-active pane, not the source pane (GH #87)", () => {
    resetPaneLayoutToSingle();
    const sourcePaneId = splitPane("main", "row", { focusNew: false })!;
    focusPane("main");
    const host = document.createElement("div");
    document.body.appendChild(host);
    const source = paneRouter(sourcePaneId);
    const active = paneRouter("main");
    const sourceTabsBefore = source.tabs().length;
    const activeTabsBefore = active.tabs().length;
    const dispose = render(() => (
      <PaneContext.Provider value={{ paneId: sourcePaneId, router: source }}>
        <div class="block-content"><AstBody raw="see [[Target]]" /></div>
      </PaneContext.Provider>
    ), host);
    try {
      const link = host.querySelector("a.page-ref");
      expect(link).toBeTruthy();
      link!.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
      expect(source.tabs()).toHaveLength(sourceTabsBefore);
      expect(active.tabs()).toHaveLength(activeTabsBefore + 1);
      expect(active.tabs().some((tab) => tab.history.some((route) => route.kind === "page" && route.name === "Target"))).toBe(true);
    } finally {
      dispose();
      host.remove();
      resetPaneLayoutToSingle();
    }
  });
});

describe("code-card click-to-caret mapping (GH #489)", () => {
  // A whole-block code fence renders as highlight.js markup, which carries none
  // of the lsdoc span attributes the general mapper reads. Its own mapper has
  // to answer from rendered text position instead, and must decline anything
  // outside the card so ordinary blocks keep using the general path.
  const raw = ["```js", "const a = 1;", "const b = 2;", "```"].join("\n");

  it("maps a click inside the highlighted body to its offset in the code", () => {
    const { root, dispose } = mountedBody(raw);
    try {
      expect(root.querySelector("pre.code-block > code")).toBeTruthy();
      const body = "const a = 1;\nconst b = 2;";
      expect(codeCardOffsetFromRange(root, textRange(root, "const a", 3))).toBe(3);
      expect(codeCardOffsetFromRange(root, textRange(root, "const b", 5))).toBe(body.indexOf("const b") + 5);
    } finally {
      dispose();
    }
  });

  it("declines a range that is not inside a code card", () => {
    const { root, dispose } = mountedBody("plain **text** here");
    try {
      expect(codeCardOffsetFromRange(root, textRange(root, "text", 2))).toBeNull();
    } finally {
      dispose();
    }
  });
});
