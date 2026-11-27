import { describe, it, expect } from "vitest";
import { parseOutline, pastedPlainBlocks } from "./outline";
import { markdownBlocksByLine } from "../render/parse";

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

it("keeps list segmentation outside inline parser literals", () => {
  expect(parseOutline("- `inline`\n2) second")).toEqual([
    { raw: "`inline`", children: [] }, { raw: "second", children: [] },
  ]);
});

it("uses parser byte ranges after Unicode text and normalizes CRLF once", () => {
  const literal = "  ```text\n  - 値\n\n  ```";
  expect(parseOutline(`- ž😀\r\n${literal.replaceAll("\n", "\r\n")}\r\n2) second`)).toEqual([
    { raw: `ž😀\n${literal}`, children: [] }, { raw: "second", children: [] },
  ]);
});

// GH #1051: an already-bulleted paste keeps each dash bullet's body as reading
// the same text from a file would (blank lines, tables, any paragraph).
describe("dash bullet bodies are the page reader's", () => {
  const reduced = "- A\n  B\n- C\n  \n  D\n  \n  E";
  const reader = (text: string) => markdownBlocksByLine(text).map(([, , raw]) => raw);

  it("keeps blank-line-separated paragraphs and their blank lines in the bullet, LF and CRLF", () => {
    const want = [{ raw: "A\nB", children: [] }, { raw: "C\n\nD\n\nE", children: [] }];
    expect(pastedPlainBlocks(reduced, "md")).toEqual(want);
    expect(pastedPlainBlocks(reduced.replaceAll("\n", "\r\n"), "md")).toEqual(want);
    expect(want.map((n) => n.raw)).toEqual(reader(reduced));
  });

  it("keeps a table under a bullet in its body, with or without a blank line before it", () => {
    const table = "| 1 | 2 |\n| --- | --- |\n| ![x](https://example.org/a.png) | B |";
    const indented = table.replaceAll("\n", "\n  ");
    expect(pastedPlainBlocks(`- A\n  ${indented}\n- Z`, "md")).toEqual([
      { raw: `A\n${table}`, children: [] }, { raw: "Z", children: [] },
    ]);
    expect(pastedPlainBlocks(`- A\n  \n  ${indented}`, "md")).toEqual([{ raw: `A\n\n${table}`, children: [] }]);
  });

  it("keeps an emoji-led paragraph after a blank line", () => {
    expect(pastedPlainBlocks("- A\n  \n  🏁 B", "md")).toEqual([{ raw: "A\n\n🏁 B", children: [] }]);
  });

  it("still makes an indented bullet a child, with its own body", () => {
    expect(pastedPlainBlocks("- A\n\n  para\n  - child\n\n    child para\n- B", "md")).toEqual([
      { raw: "A\n\npara", children: [{ raw: "child\n\nchild para", children: [] }] },
      { raw: "B", children: [] },
    ]);
  });

  it("leaves text outside the bullet's content column, other markers and trailing blanks as before", () => {
    expect(parseOutline("- A\n\nloose\n  + plus\n\n- B\n\n")).toEqual([
      { raw: "A", children: [] }, { raw: "loose", children: [{ raw: "plus", children: [] }] }, { raw: "B", children: [] },
    ]);
  });
});

// L3-08: OG nests an Org outline paste by its star levels.
describe("Org outline paste", () => {
  it("nests headlines by level", () => {
    expect(pastedPlainBlocks("* Parent\n** Child\n*** Grand\n** Second\n* Next", "org")).toEqual([
      { raw: "Parent", children: [
        { raw: "Child", children: [{ raw: "Grand", children: [] }] },
        { raw: "Second", children: [] },
      ] },
      { raw: "Next", children: [] },
    ]);
  });
  it("leaves emphasis and Markdown alone", () => {
    expect(parseOutline("* Parent\n**bold** text", "org").map((n) => n.raw)).toEqual(["Parent", "**bold** text"]);
    expect(parseOutline("** Child", "md")).toEqual([{ raw: "** Child", children: [] }]);
  });
});
