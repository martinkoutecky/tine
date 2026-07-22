import { describe, expect, it } from "vitest";
import { exportHtml } from "./exportHtml";
import type { ExportNode } from "./exportText";

const forest: ExportNode[] = [
  {
    raw: "Root <script>alert(1)</script> [[Page]] **bold** #tag\nproperty:: hidden",
    children: [
      { raw: "Child & sibling", children: [{ raw: "Grandchild", children: [] }] },
    ],
  },
];

describe("exportHtml", () => {
  it("serializes an escaped nested HTML fragment and omits properties", () => {
    const result = exportHtml(forest, {
      stripLinks: false,
      removeEmphasis: false,
      removeTags: false,
      maxDepth: "all",
    });

    expect(result).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(result).not.toContain("<script>");
    expect(result).toContain("[[Page]] <strong>bold</strong> #tag");
    expect(result).toContain("Child &amp; sibling");
    expect(result).not.toContain("property");
    expect(result).toMatch(/<li>Root[\s\S]*<ul>[\s\S]*<li>Child[\s\S]*<ul>[\s\S]*<li>Grandchild/);
  });

  it("applies shared cleanup options and maximum depth", () => {
    const result = exportHtml(forest, {
      stripLinks: true,
      removeEmphasis: true,
      removeTags: true,
      maxDepth: 1,
    });

    expect(result).toContain("Page bold</li>");
    expect(result).not.toContain("[[Page]]");
    expect(result).not.toContain("<strong>");
    expect(result).not.toContain("#tag");
    expect(result).not.toContain("Child");
  });
});
