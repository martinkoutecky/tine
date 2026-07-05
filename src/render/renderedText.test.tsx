import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./parse";
import { renderedBlockText, type RenderedTextOptions } from "./renderedText";
import { exportOutline } from "../editor/exportText";

beforeAll(async () => {
  await initParser();
});

const O: RenderedTextOptions = {
  typographicGlyphs: true,
  stripLinks: false,
  removeTags: false,
  removeProperties: false,
};
const REF_ID = "11111111-1111-4111-8111-111111111111";

describe("renderedBlockText", () => {
  it("drops markup markers and applies typographic glyphs", () => {
    expect(renderedBlockText("**bold** and a -> b -- c", "md", O)).toBe("bold and a → b – c");
    expect(renderedBlockText("**bold** and a -> b", "md", { ...O, typographicGlyphs: false })).toBe(
      "bold and a -> b",
    );
  });

  it("keeps visible link brackets, honors stripLinks, and uses aliases", () => {
    expect(renderedBlockText("see [[Foo Bar]]", "md", O)).toBe("see [[Foo Bar]]");
    expect(renderedBlockText("see [[Foo Bar]]", "md", { ...O, stripLinks: true })).toBe("see Foo Bar");
    expect(renderedBlockText("see [alias]([[Foo Bar]])", "md", O)).toBe("see alias");
    expect(renderedBlockText("go to [site](https://x.example)", "md", O)).toBe("go to site");
  });

  it("keeps #tags visibly, honors removeTags", () => {
    expect(renderedBlockText("a #tag here", "md", O)).toBe("a #tag here");
    expect(renderedBlockText("a #tag here", "md", { ...O, removeTags: true })).toBe("a  here");
  });

  it("prefixes marker/priority like the rendered chips and keeps multi-line bodies", () => {
    expect(renderedBlockText("TODO [#A] fix it\nsecond line", "md", O)).toBe("TODO [#A] fix it\nsecond line");
  });

  it("renders entities as unicode and code as its text", () => {
    expect(renderedBlockText("\\Delta and `x->y`", "md", O)).toBe("Δ and x->y");
  });

  it("keeps math delimiters and only unicode-converts trivial inline math when asked", () => {
    expect(renderedBlockText("Euler $x^2$ and $$y$$", "md", O)).toBe("Euler $x^2$ and $$y$$");
    expect(renderedBlockText("$$E=mc^2$$", "md", O)).toBe("$$\nE=mc^2\n$$");
    expect(renderedBlockText("Euler $E=mc^2$ and $\\frac{1}{2}$", "md", { ...O, mathAsUnicode: true })).toBe(
      "Euler E=mc² and $\\frac{1}{2}$",
    );
  });

  it("flattens tables to cell rows and honors removeProperties", () => {
    expect(renderedBlockText("|a|b|\n|-|-|\n|1|2|", "md", O)).toBe("a | b\n1 | 2");
    expect(renderedBlockText("text\nkey:: val", "md", O)).toBe("text\nkey val");
    expect(renderedBlockText("text\nkey:: val", "md", { ...O, removeProperties: true })).toBe("text");
  });

  it("keeps refs and macros byte-compatible when no resolvers are supplied", () => {
    expect(renderedBlockText(`see ((${REF_ID})) and {{poem red, blue}}`, "md", O)).toBe(
      `see ${REF_ID} and {{poem red, blue}}`,
    );
  });

  it("resolves bare block refs to the referenced rendered first line", () => {
    expect(
      renderedBlockText(`see ((${REF_ID}))`, "md", {
        ...O,
        resolveBlockRef: (uuid) =>
          uuid === REF_ID ? { raw: "**Referenced** first line\nsecond line", format: "md" } : null,
      }),
    ).toBe("see Referenced first line");
  });

  it("can resolve bare block refs to the referenced full body", () => {
    expect(
      renderedBlockText(`see ((${REF_ID}))`, "md", {
        ...O,
        resolveBlockRefsFully: true,
        resolveBlockRef: (uuid) =>
          uuid === REF_ID ? { raw: "**Referenced** first line\nsecond line", format: "md" } : null,
      }),
    ).toBe("see Referenced first line\nsecond line");
  });

  it("keeps labeled block refs as labels and does not consult the resolver", () => {
    let calls = 0;
    expect(
      renderedBlockText(`see [chosen](((${REF_ID})))`, "md", {
        ...O,
        resolveBlockRef: () => {
          calls++;
          return { raw: "wrong", format: "md" };
        },
      }),
    ).toBe("see chosen");
    expect(calls).toBe(0);
  });

  it("resolves user macros to rendered expanded text", () => {
    expect(
      renderedBlockText("{{poem red, blue}}", "md", {
        ...O,
        resolveMacro: (name, args) =>
          name === "poem" ? { raw: `Roses are ${args[0]}, violets are ${args[1]}.`, format: "md" } : null,
      }),
    ).toBe("Roses are red, violets are blue.");
  });

  it("keeps built-in macro names literal when the resolver returns null", () => {
    expect(
      renderedBlockText("{{query (task TODO)}}", "md", {
        ...O,
        resolveMacro: () => null,
      }),
    ).toBe("{{query (task TODO)}}");
  });

  it("resolves provider macros from warmed text leaves", () => {
    expect(
      renderedBlockText("{{video https://example.test/v.mp4}}", "md", {
        ...O,
        resolveMacro: (name, args) =>
          name === "video" ? { raw: "", format: "md", text: args.join(", ") } : null,
      }),
    ).toBe("https://example.test/v.mp4");
  });

  it("bails to the fallback at the resolver recursion cap", () => {
    expect(
      renderedBlockText(`((${REF_ID}))`, "md", {
        ...O,
        resolveBlockRef: (uuid) => ({ raw: `((${uuid}))`, format: "md" }),
      }),
    ).toBe(REF_ID);
  });
});

describe("exportOutline rendered mode", () => {
  it("renders the forest with indentation, defaulting md", () => {
    const nodes = [
      { raw: "**parent** -> x", children: [{ raw: "child [[P]]", children: [] }] },
    ];
    const text = exportOutline(nodes, {
      content: "rendered",
      indent: "dashes",
      stripLinks: false,
      removeEmphasis: false,
      removeTags: false,
      removeProperties: false,
      newlineAfterBlock: false,
      typographicGlyphs: true,
    });
    expect(text).toBe("- parent → x\n\t- child [[P]]");
  });
});
