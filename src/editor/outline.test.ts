import { describe, it, expect } from "vitest";
import { parseOutline, pastedPlainBlocks } from "./outline";
import type { Format } from "../render/ast";
import pasteReaderBlocks from "../../tests/fixtures/paste-reader-blocks.json";

describe("parseOutline", () => {
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

// GH #1051: the outline conversion every paste, capture and grid paste shares
// imports exactly the blocks the page reader reads from the same text as a file.
// The table's blocks are checked against the native reader, and through save and
// reopen, by tine-core `tests/paste_reader.rs`; a second answer to which lines a
// pasted block holds fails here.
describe("pasted blocks are the page reader's", () => {
  it.each(pasteReaderBlocks.map((c) => [c.name, c] as const))("%s", (_, c) => {
    expect(parseOutline(c.text, c.format as Format)).toEqual(c.blocks);
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
});
