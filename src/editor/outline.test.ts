import { describe, it, expect } from "vitest";
import { parseOutline } from "./outline";

describe("parseOutline", () => {
  it("flat multiline text -> sibling blocks", () => {
    expect(parseOutline("one\ntwo\nthree")).toEqual([
      { raw: "one", children: [] },
      { raw: "two", children: [] },
      { raw: "three", children: [] },
    ]);
  });

  it("skips blank lines in flat text", () => {
    expect(parseOutline("a\n\nb")).toEqual([
      { raw: "a", children: [] },
      { raw: "b", children: [] },
    ]);
  });

  it("bulleted outline with nesting", () => {
    const text = "- parent\n\t- child a\n\t- child b\n- sibling";
    expect(parseOutline(text)).toEqual([
      {
        raw: "parent",
        children: [
          { raw: "child a", children: [] },
          { raw: "child b", children: [] },
        ],
      },
      { raw: "sibling", children: [] },
    ]);
  });

  it("space-indented bullets nest too", () => {
    const text = "- a\n  - b\n    - c";
    expect(parseOutline(text)).toEqual([
      { raw: "a", children: [{ raw: "b", children: [{ raw: "c", children: [] }] }] },
    ]);
  });

  it("recognizes common unordered and ordered Markdown list markers", () => {
    const text = "+ plus\n  * star child\n1. first\n2) second";
    expect(parseOutline(text)).toEqual([
      { raw: "plus", children: [{ raw: "star child", children: [] }] },
      { raw: "first", children: [] },
      { raw: "second", children: [] },
    ]);
  });

  it("keeps a contiguous GFM table as one block", () => {
    const table = "| Name | Value |\n| --- | ---: |\n| Alpha | 1 |\n| Beta | 2 |";
    expect(parseOutline(table)).toEqual([{ raw: table, children: [] }]);
  });

  it("keeps a fenced code block as one block", () => {
    const code = "```ts\nconst x = 1;\n```";
    expect(parseOutline(code)).toEqual([{ raw: code, children: [] }]);
  });

  it("keeps a bullet's indented continuation lines in the same block", () => {
    const text = "- first line\n  second line\n- next";
    expect(parseOutline(text)).toEqual([
      { raw: "first line\nsecond line", children: [] },
      { raw: "next", children: [] },
    ]);
  });

  it("does NOT drop headings/paragraphs that sit before or between bullets", () => {
    // Markdown with headings + paragraphs + a `- ` list, intermixed. Regression
    // for the paste bug where pre-bullet content was dropped and post-bullet
    // content was swallowed into the last bullet as a continuation.
    const text = [
      "# Summary",
      "",
      "The paper studies shortest-path network design.",
      "",
      "## Strengths",
      "",
      "- the paper is well written",
      "- it studies a relevant problem",
      "",
      "## Weaknesses",
      "",
      "I would be in favor of accepting.",
      "",
      "The ILP is solved by branch-and-price.",
    ].join("\n");
    expect(parseOutline(text)).toEqual([
      { raw: "# Summary", children: [] },
      { raw: "The paper studies shortest-path network design.", children: [] },
      { raw: "## Strengths", children: [] },
      { raw: "the paper is well written", children: [] },
      { raw: "it studies a relevant problem", children: [] },
      { raw: "## Weaknesses", children: [] },
      { raw: "I would be in favor of accepting.", children: [] },
      { raw: "The ILP is solved by branch-and-price.", children: [] },
    ]);
  });

  it("is lossless: every non-blank source line survives somewhere", () => {
    const text =
      "# Summary\n\npara one\n\n- bullet a\n- bullet b\n\n## Weaknesses\n\npara two\n";
    const flatten = (ns: ReturnType<typeof parseOutline>): string[] =>
      ns.flatMap((n) => [...n.raw.split("\n"), ...flatten(n.children)]);
    const got = flatten(parseOutline(text))
      .map((l) => l.trim())
      .filter((l) => l.length > 0);
    const want = text
      .split("\n")
      .map((l) => l.replace(/^- /, "").trim())
      .filter((l) => l.length > 0);
    expect(got.sort()).toEqual(want.sort());
  });
});
