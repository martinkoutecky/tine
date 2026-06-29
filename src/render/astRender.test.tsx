import { describe, it, expect, beforeAll } from "vitest";
import { render } from "solid-js/web";
import { renderInlines, InlineText } from "./inline";
import { renderBlocks } from "./body";
import { initParser } from "./parse";
import type { JSX } from "solid-js";
import type { Block, Inline } from "./ast";

// A few render paths reach back into the wasm parser (e.g. a properties block
// renders each value via InlineText → parseBlock). Node supports WebAssembly +
// atob, so we load the real vendored parser once — this doubles as a wasm smoke
// test (it must instantiate cleanly outside WebKitGTK too).
beforeAll(async () => {
  await initParser();
});

function html(node: () => JSX.Element): string {
  const div = document.createElement("div");
  const dispose = render(() => node(), div);
  const out = div.innerHTML;
  dispose();
  return out;
}
const inl = (xs: Inline[]) => html(() => renderInlines(xs));
const blk = (xs: Block[]) => html(() => renderBlocks(xs));

describe("renderInlines", () => {
  it("plain + emphasis", () => {
    const h = inl([
      { k: "plain", text: "a " },
      { k: "emphasis", emph: "Bold", children: [{ k: "plain", text: "b" }] },
      { k: "plain", text: " " },
      { k: "emphasis", emph: "Italic", children: [{ k: "plain", text: "c" }] },
    ]);
    expect(h).toContain("<strong>");
    expect(h).toContain("b");
    expect(h).toContain("<em>");
  });

  it("code + highlight + strike + underline", () => {
    expect(inl([{ k: "code", text: "x" }])).toContain('class="inline-code"');
    expect(inl([{ k: "emphasis", emph: "Highlight", children: [{ k: "plain", text: "h" }] }])).toContain("<mark>");
    expect(inl([{ k: "emphasis", emph: "Strike_through", children: [{ k: "plain", text: "s" }] }])).toContain("<del>");
    expect(inl([{ k: "emphasis", emph: "Underline", children: [{ k: "plain", text: "u" }] }])).toContain("<u>");
  });

  it("page ref renders brackets + name", () => {
    const h = inl([{ k: "link", url: { type: "page_ref", v: "My Page" }, full: "[[My Page]]" }]);
    expect(h).toContain('class="page-ref"');
    expect(h).toContain("My Page");
  });

  it("page ref with alias label", () => {
    const h = inl([{ k: "link", url: { type: "page_ref", v: "Target" }, full: "[[Target][alias]]", label: [{ k: "plain", text: "alias" }] }]);
    expect(h).toContain('class="page-ref"');
    expect(h).toContain("alias");
  });

  it("tag renders #name", () => {
    const h = inl([{ k: "tag", children: [{ k: "plain", text: "project" }] }]);
    expect(h).toContain('class="tag"');
    expect(h).toContain("#project");
  });

  it("external link", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "example.com/x" }, full: "https://example.com/x" }]);
    expect(h).toContain('class="external-link"');
    expect(h).toContain("https://example.com/x");
  });

  it("image flag → inline-image-wrap (external)", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "x.com/a.png" }, full: "![](…)", image: true, label: [{ k: "plain", text: "alt" }] }]);
    expect(h).toContain("inline-image");
  });

  it("latex inline", () => {
    expect(inl([{ k: "latex", mode: "Inline", body: "x^2" }])).toContain('class="math"');
  });

  it("org active timestamp formats date in <>", () => {
    const h = inl([{ k: "timestamp", ts: "Date", date: { date: { year: 2026, month: 6, day: 28 }, wday: "Sun", active: true } }]);
    expect(h).toContain("org-timestamp");
    expect(h).toContain("2026-06-28");
    expect(h).toContain("Sun");
  });

  it("entity renders the unicode glyph", () => {
    const h = inl([{ k: "entity", name: "Delta", latex: "\\Delta", latex_mathp: true, html: "&Delta;", ascii: "[Delta]", unicode: "Δ" }]);
    expect(h).toContain("Δ");
  });

  it("footnote ref", () => {
    expect(inl([{ k: "fnref", name: "1" }])).toContain('class="footnote-ref"');
  });

  // lsdoc v0.1.4: inline Clojure-hiccup renders its raw bracket text literally.
  it("inline hiccup renders raw text literally", () => {
    expect(inl([{ k: "hiccup", v: "[:span \"y\"]" }])).toContain("[:span");
  });
});

describe("renderBlocks", () => {
  it("bullet header renders inline title", () => {
    expect(blk([{ kind: "bullet", level: 1, inline: [{ k: "plain", text: "hello" }] }])).toContain("hello");
  });

  it("src code block", () => {
    expect(blk([{ kind: "src", lang: "rust", code: "fn main(){}" }])).toContain("code-block");
  });

  it("hr", () => {
    expect(blk([{ kind: "hr" }])).toContain("md-hr");
  });

  it("table renders cells", () => {
    const h = blk([{ kind: "table", header: [[{ k: "plain", text: "A" }], [{ k: "plain", text: "B" }]], rows: [[[{ k: "plain", text: "1" }], [{ k: "plain", text: "2" }]]] }]);
    expect(h).toContain("md-table");
    expect(h).toContain("A");
    expect(h).toContain("1");
  });

  it("md [!NOTE] callout: title once, body separate (no dup)", () => {
    const h = blk([
      {
        kind: "quote",
        children: [
          {
            kind: "paragraph",
            inline: [
              { k: "plain", text: "[!NOTE] Heads up" },
              { k: "break" },
              { k: "plain", text: "body line" },
            ],
          },
        ],
      },
    ]);
    expect(h).toContain("callout-note");
    expect(h).toContain("Heads up");
    expect(h).toContain("body line");
    // The title text must NOT be duplicated into the body.
    expect(h.split("Heads up").length - 1).toBe(1);
  });

  it("org custom callout", () => {
    const h = blk([{ kind: "custom", name: "TIP", children: [{ kind: "paragraph", inline: [{ k: "plain", text: "do this" }] }] }]);
    expect(h).toContain("callout-tip");
  });

  it("properties block filters id::, shows user props", () => {
    const h = blk([{ kind: "properties", props: [["id", "x"], ["author", "Martin"]] }]);
    expect(h).toContain("author");
    expect(h).not.toContain(">id<");
  });

  it("displayed math block", () => {
    expect(blk([{ kind: "displayed_math", text: "E=mc^2" }])).toContain("math-display");
  });

  it("drawer/comment render nothing", () => {
    expect(blk([{ kind: "comment", text: "c" }])).not.toContain("c");
  });

  // lsdoc v0.1.4: Clojure-hiccup blocks render their raw bracket text literally.
  it("hiccup block renders raw text literally", () => {
    const h = blk([{ kind: "hiccup", v: "[:div.note \"hi\"]" }]);
    expect(h).toContain("ast-hiccup");
    expect(h).toContain("[:div.note");
  });

  // A `# heading` block's size applies ONLY to its first (heading) line — a `> quote`
  // continuation in the same block stays normal-size (OG parity, not h1). The heading
  // level is passed to renderBlocks; only block 0 is wrapped in heading-text.
  it("heading size wraps only the first block, not a continuation quote", () => {
    const h = html(() =>
      renderBlocks(
        [
          { kind: "paragraph", inline: [{ k: "plain", text: "Title" }] },
          { kind: "quote", children: [{ kind: "paragraph", inline: [{ k: "plain", text: "quoted" }] }] },
        ],
        undefined,
        1,
      ),
    );
    const headingSpan = /<span class="heading-text h1">.*?<\/span>/s.exec(h)?.[0] ?? "";
    expect(headingSpan).toContain("Title"); // heading line is h1
    expect(headingSpan).not.toContain("quoted"); // the quote is OUTSIDE the heading span
  });
});

// InlineText is the inline-only renderer for property values, breadcrumbs,
// ref-preview lines, etc. It parses via wasm; audit fix C1: a line that lsdoc
// parses as a BLOCK construct (and so has no inline-flow content) must fall back
// to the literal text, never render blank.
describe("InlineText", () => {
  const txt = (s: string, fmt?: "md" | "org") =>
    html(() => InlineText({ text: s, format: fmt }) as JSX.Element);
  // strip tags + un-escape the few entities the renderer emits, to read visible text
  const visible = (h: string) =>
    h.replace(/<[^>]*>/g, "").replace(/&gt;/g, ">").replace(/&lt;/g, "<").replace(/&amp;/g, "&");

  it("renders normal inline markup", () => {
    expect(txt("a **b**")).toContain("<strong>");
  });

  // Audit fix C4: the format prop must drive parsing, so org inline syntax renders
  // as org (italic), not literally, when the caller is on an org page.
  it("parses org inline markup as org when format=org (C4)", () => {
    expect(txt("/italic/", "org")).toContain("<em>");
    expect(txt("/italic/", "md")).not.toContain("<em>"); // md: literal slashes, no italic
  });

  it.each([
    ["> quote", "quote"],
    ["---", "---"],
    ["| a | b |", "a"],
    ["[^1]: def", "def"],
    ["$$E=mc^2$$", "mc"],
  ])("falls back to literal text for block-construct line %j (never blank)", (line, token) => {
    const text = visible(txt(line));
    expect(text.trim().length).toBeGreaterThan(0);
    expect(text).toContain(token);
  });
});
