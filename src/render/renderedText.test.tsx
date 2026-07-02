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

  it("flattens tables to cell rows and honors removeProperties", () => {
    expect(renderedBlockText("|a|b|\n|-|-|\n|1|2|", "md", O)).toBe("a | b\n1 | 2");
    expect(renderedBlockText("text\nkey:: val", "md", O)).toBe("text\nkey val");
    expect(renderedBlockText("text\nkey:: val", "md", { ...O, removeProperties: true })).toBe("text");
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
