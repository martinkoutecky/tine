import { describe, expect, it } from "vitest";
import { HTML_PASTE_LIMITS, structuredHtmlOutline } from "./htmlPaste";

describe("structuredHtmlOutline", () => {
  it("preserves explicit nested lists", () => {
    expect(structuredHtmlOutline(
      "<ul><li>Parent<ul><li>Child <strong>bold</strong></li></ul></li><li>Sibling</li></ul>",
      "Parent\nChild bold\nSibling",
    )).toEqual([
      { raw: "Parent", children: [{ raw: "Child **bold**", children: [] }] },
      { raw: "Sibling", children: [] },
    ]);
  });

  it("keeps literal square brackets instead of backslash-escaping them (GH: Martin)", () => {
    // Multi-block matches outlineToHtml's own output for a multi-line block copy,
    // so the single-node plain-text bypass does not apply and the outline is asserted.
    expect(structuredHtmlOutline(
      "<ul><li>see [ref] here</li><li>and [two]</li></ul>",
      "see [ref] here\nand [two]",
    )).toEqual([
      { raw: "see [ref] here", children: [] },
      { raw: "and [two]", children: [] },
    ]);
  });

  it("keeps headings, quotes, links, emphasis, and fenced code", () => {
    const outline = structuredHtmlOutline([
      "<h2>Heading</h2>",
      "<p>A <a href='https://example.com/x'>link</a> and <em>note</em>.</p>",
      "<blockquote><p>Quoted</p></blockquote>",
      "<pre><code>const x = 1;</code></pre>",
    ].join(""), "Heading\nA link and note.\nQuoted\nconst x = 1;");
    expect(outline?.map((node) => node.raw)).toEqual([
      "## Heading",
      "A [link](https://example.com/x) and _note_.",
      "> Quoted",
      "```\nconst x = 1;\n```",
    ]);
  });

  it("converts a rendered table to one GFM table block", () => {
    const outline = structuredHtmlOutline(
      "<table><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody><tr><td>Alpha</td><td>1</td></tr></tbody></table>",
      "Name Value\nAlpha 1",
    );
    expect(outline).toHaveLength(1);
    expect(outline![0].raw).toContain("| Name | Value |");
    expect(outline![0].raw).toContain("| Alpha | 1 |");
  });

  it("sanitizes active content and unsafe attributes before conversion", () => {
    const outline = structuredHtmlOutline(
      "<p onclick='steal()' style='color:red'>Safe <a href='javascript:steal()'>link</a><script>bad()</script></p>",
      "Safe link",
    );
    const raw = outline?.map((node) => node.raw).join("\n") ?? "";
    expect(raw).not.toMatch(/steal|script|style|onclick|javascript/i);
  });

  it("keeps surrounding text while converting safe images and declining unsafe encoded data URLs", () => {
    const outline = structuredHtmlOutline(
      [
        "<p>Before ",
        "<img src='https://example.com/cat.png' alt='Cat'>",
        " middle ",
        "<img src='data:image/svg+xml,%3Csvg%3E%3C/svg%3E' alt='Unsafe'>",
        " after.</p>",
      ].join(""),
      "Before Cat middle Unsafe after.",
    );
    const raw = outline?.map((node) => node.raw).join("\n") ?? "";
    expect(raw).toContain("Before");
    expect(raw).toContain("![Cat](https://example.com/cat.png)");
    expect(raw).toContain("middle");
    expect(raw).toContain("after.");
    expect(raw).not.toMatch(/data:image|Unsafe/);
  });

  it.each([
    ["md", "![Diagram](https://example.com/diagram.png)"],
    ["org", "[[https://example.com/diagram.png][Diagram]]"],
  ] as const)("converts an image-only %s payload with format-aware syntax", (format, expected) => {
    expect(structuredHtmlOutline(
      "<img src='https://example.com/diagram.png' alt='Diagram'>",
      "Diagram",
      format,
    )).toEqual([{ raw: expected, children: [] }]);
  });

  it("falls back for non-semantic, malformed, deep, and oversized payloads", () => {
    expect(structuredHtmlOutline("<span>plain</span>", "plain")).toBeNull();
    expect(() => structuredHtmlOutline("<ul><li>broken", "broken")).not.toThrow();
    const deep = "<div>".repeat(HTML_PASTE_LIMITS.depth + 2) + "x" + "</div>".repeat(HTML_PASTE_LIMITS.depth + 2);
    expect(structuredHtmlOutline(deep, "x")).toBeNull();
    const tooManyNodes = `<p>${"<span>x</span>".repeat(HTML_PASTE_LIMITS.nodes + 1)}</p>`;
    expect(structuredHtmlOutline(tooManyNodes, "x")).toBeNull();
    const oversized = `<p>${"x".repeat(HTML_PASTE_LIMITS.inputBytes + 1)}</p>`;
    expect(structuredHtmlOutline(oversized, "x")).toBeNull();
  });
});
