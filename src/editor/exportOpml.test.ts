import { describe, expect, it } from "vitest";
import { exportOpml } from "./exportOpml";
import type { ExportNode } from "./exportText";

const forest: ExportNode[] = [
  {
    raw: 'Root & "quoted" [[Page]] **bold** #tag\nproperty:: hidden',
    children: [
      {
        raw: "Child <node> & 'quote'",
        children: [{ raw: "Grandchild", children: [] }],
      },
    ],
  },
];

describe("exportOpml", () => {
  it("serializes an ExportNode forest, omits properties, and escapes XML contexts", () => {
    const result = exportOpml(forest, {
      stripLinks: false,
      removeEmphasis: false,
      removeTags: false,
      maxDepth: "all",
    }, 'Page <one> & "two"');

    expect(result).toContain('<title>Page &lt;one&gt; &amp; "two"</title>');
    expect(result).toContain('text="Root &amp; &quot;quoted&quot; [[Page]] **bold** #tag"');
    expect(result).toContain('text="Child &lt;node&gt; &amp; &apos;quote&apos;"');
    expect(result).not.toContain("property");
    expect(result.indexOf("Child")).toBeGreaterThan(result.indexOf("Root"));
    expect(result.indexOf("Grandchild")).toBeGreaterThan(result.indexOf("Child"));
  });

  it("applies shared cleanup options and maximum depth", () => {
    const result = exportOpml(forest, {
      stripLinks: true,
      removeEmphasis: true,
      removeTags: true,
      maxDepth: 1,
    });

    expect(result).toContain('text="Root &amp; &quot;quoted&quot; Page bold"');
    expect(result).not.toContain("Child");
    expect(result).not.toContain("Grandchild");
  });
});

it("D16 cleanup preserves inline and block literals in both formats", () => {
  for (const [format, raw] of [
    ["md", "`[[literal]] #tag **code**` [[Page]] #real **bold**"],
    ["org", "~[[literal]] #tag *code*~ [[Page]] #real *bold*"],
    ["org", "#+BEGIN_SRC text\n[[literal]] #tag *code*\n#+END_SRC\n[[Page]] #real *bold*"],
  ] as const) {
    const xml = exportOpml([{raw, format, children:[]}], {stripLinks:true, removeTags:true, removeEmphasis:true});
    expect(xml).toContain("[[literal]] #tag");
    expect(xml).not.toContain("[[Page]]");
    expect(xml).not.toContain("#real");
  }
});
