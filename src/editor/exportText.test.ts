import { describe, it, expect } from "vitest";
import { exportOutline, DEFAULT_EXPORT_OPTIONS, type ExportNode, type ExportOptions } from "./exportText";

// These tests cover the SOURCE serialization (raw + regex transforms); rendered
// mode needs the wasm parser and is tested in src/render/renderedText.test.tsx.
const opt = (o: Partial<ExportOptions>): ExportOptions => ({ ...DEFAULT_EXPORT_OPTIONS, content: "source", ...o });

// parent / child / grandchild tree for indent-style tests.
const tree: ExportNode[] = [
  { raw: "parent", children: [{ raw: "child", children: [{ raw: "grand", children: [] }] }] },
];

describe("exportOutline", () => {
  it("dashes = Logseq outline (tabs + bullets)", () => {
    expect(exportOutline(tree, opt({ indent: "dashes" }))).toBe("- parent\n\t- child\n\t\t- grand");
  });

  it("spaces = indentation, no bullets (2 spaces/level)", () => {
    expect(exportOutline(tree, opt({ indent: "spaces" }))).toBe("parent\n  child\n    grand");
  });

  it("no-indent = flat, no bullets", () => {
    expect(exportOutline(tree, opt({ indent: "no-indent" }))).toBe("parent\nchild\ngrand");
  });

  it("strips [[links]] to text", () => {
    const n: ExportNode[] = [{ raw: "see [[Foo Bar]] now", children: [] }];
    expect(exportOutline(n, opt({ indent: "no-indent", stripLinks: true }))).toBe("see Foo Bar now");
  });

  it("removes #tags (and tidies the gap)", () => {
    const n: ExportNode[] = [{ raw: "hello #tag and #[[Two Words]] world", children: [] }];
    expect(exportOutline(n, opt({ indent: "no-indent", removeTags: true }))).toBe("hello and world");
  });

  it("removes emphasis markers", () => {
    const n: ExportNode[] = [{ raw: "**bold** _it_ ~~s~~ ==h==", children: [] }];
    expect(exportOutline(n, opt({ indent: "no-indent", removeEmphasis: true }))).toBe("bold it s h");
  });

  it("removes property lines", () => {
    const n: ExportNode[] = [{ raw: "text\nkey:: value\nother:: x", children: [] }];
    expect(exportOutline(n, opt({ indent: "no-indent", removeProperties: true }))).toBe("text");
    // dashes keeps the bullet on the first (only) remaining line.
    expect(exportOutline(n, opt({ indent: "dashes", removeProperties: true }))).toBe("- text");
  });

  it("newline after block separates siblings, trims trailing", () => {
    const n: ExportNode[] = [
      { raw: "a", children: [] },
      { raw: "b", children: [] },
    ];
    expect(exportOutline(n, opt({ indent: "no-indent", newlineAfterBlock: true }))).toBe("a\n\nb");
  });

  it("keeps multi-line block continuation aligned (dashes)", () => {
    const n: ExportNode[] = [{ raw: "first\nsecond", children: [{ raw: "kid", children: [] }] }];
    expect(exportOutline(n, opt({ indent: "dashes" }))).toBe("- first\n  second\n\t- kid");
  });
});
