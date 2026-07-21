import { beforeAll, describe, it, expect } from "vitest";
import { exportOutline, DEFAULT_EXPORT_OPTIONS, type ExportNode, type ExportOptions } from "./exportText";
import { initParser } from "../render/parse";

// Most tests cover SOURCE serialization (raw + regex transforms); the focused
// rendered-mode test initializes the wasm parser here.
const opt = (o: Partial<ExportOptions>): ExportOptions => ({ ...DEFAULT_EXPORT_OPTIONS, content: "source", ...o });
const REF_ID = "11111111-1111-4111-8111-111111111111";

beforeAll(async () => {
  await initParser();
});

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

  it("maximum depth 1 keeps export roots and omits descendants", () => {
    expect(exportOutline(tree, opt({ indent: "dashes", maxDepth: 1 }))).toBe("- parent");
  });

  it("maximum depth 2 keeps children but omits grandchildren", () => {
    expect(exportOutline(tree, opt({ indent: "dashes", maxDepth: 2 }))).toBe("- parent\n\t- child");
  });

  it('maximum depth "all" retains the entire forest', () => {
    expect(exportOutline(tree, opt({ indent: "dashes", maxDepth: "all" }))).toBe("- parent\n\t- child\n\t\t- grand");
  });

  it("applies maximum depth before each indentation mode serializes", () => {
    expect(exportOutline(tree, opt({ indent: "spaces", maxDepth: 2 }))).toBe("parent\n  child");
    expect(exportOutline(tree, opt({ indent: "no-indent", maxDepth: 2 }))).toBe("parent\nchild");
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

  it("threads rendered block-ref and macro resolvers", () => {
    const n: ExportNode[] = [{ raw: `((${REF_ID})) and {{poem red, blue}}`, children: [] }];
    expect(
      exportOutline(
        n,
        opt({
          content: "rendered",
          indent: "no-indent",
          resolveBlockRef: (uuid) => (uuid === REF_ID ? { raw: "**Exported** ref\nhidden second", format: "md" } : null),
          resolveMacro: (name, args) =>
            name === "poem" ? { raw: `roses ${args[0]}, violets ${args[1]}`, format: "md" } : null,
        }),
      ),
    ).toBe("Exported ref and roses red, violets blue");
  });

  it("threads rendered full-ref resolution", () => {
    const n: ExportNode[] = [{ raw: `((${REF_ID}))`, children: [] }];
    expect(
      exportOutline(
        n,
        opt({
          content: "rendered",
          indent: "no-indent",
          resolveRefsFully: true,
          resolveBlockRef: (uuid) => (uuid === REF_ID ? { raw: "first\nsecond", format: "md" } : null),
        }),
      ),
    ).toBe("first\nsecond");
  });
});
