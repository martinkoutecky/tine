import { describe, it, expect } from "vitest";
import { parseInline } from "./parseInline";

describe("parseInline", () => {
  it("plain text", () => {
    expect(parseInline("hello world")).toEqual([{ t: "text", v: "hello world" }]);
  });

  it("bold / italic / code", () => {
    expect(parseInline("**b**")).toEqual([{ t: "bold", v: [{ t: "text", v: "b" }] }]);
    expect(parseInline("*i*")).toEqual([{ t: "italic", v: [{ t: "text", v: "i" }] }]);
    expect(parseInline("`c`")).toEqual([{ t: "code", v: "c" }]);
    expect(parseInline("~~s~~")).toEqual([{ t: "strike", v: [{ t: "text", v: "s" }] }]);
    expect(parseInline("==h==")).toEqual([{ t: "highlight", v: [{ t: "text", v: "h" }] }]);
  });

  it("page ref and tag", () => {
    expect(parseInline("[[Foo Bar]]")).toEqual([{ t: "pageref", name: "Foo Bar" }]);
    expect(parseInline("#tag")).toEqual([{ t: "tag", name: "tag" }]);
    expect(parseInline("#[[multi word]]")).toEqual([{ t: "tag", name: "multi word" }]);
  });

  it("block ref, macro, math", () => {
    expect(parseInline("((abc-123))")).toEqual([{ t: "blockref", id: "abc-123" }]);
    expect(parseInline("{{query (todo)}}")).toEqual([{ t: "macro", body: "query (todo)" }]);
    expect(parseInline("$E=mc^2$")).toEqual([{ t: "math", tex: "E=mc^2" }]);
  });

  it("links and images", () => {
    expect(parseInline("[label](http://x.com)")).toEqual([
      { t: "link", label: "label", url: "http://x.com" },
    ]);
    expect(parseInline("![alt](a.png)")).toEqual([{ t: "image", alt: "alt", url: "a.png" }]);
  });

  it("mixed line", () => {
    const segs = parseInline("see [[Page]] and **bold** #tag");
    expect(segs).toEqual([
      { t: "text", v: "see " },
      { t: "pageref", name: "Page" },
      { t: "text", v: " and " },
      { t: "bold", v: [{ t: "text", v: "bold" }] },
      { t: "text", v: " " },
      { t: "tag", name: "tag" },
    ]);
  });

  it("nested emphasis (mixed delimiters)", () => {
    // Note: nested *same*-delimiter emphasis (**a *b***) is a CommonMark edge
    // case we don't fully handle; mixed delimiters work.
    expect(parseInline("**bold _italic_**")).toEqual([
      {
        t: "bold",
        v: [
          { t: "text", v: "bold " },
          { t: "italic", v: [{ t: "text", v: "italic" }] },
        ],
      },
    ]);
  });

  it("unterminated markup stays literal (no crash)", () => {
    expect(parseInline("a [[ b")).toEqual([{ t: "text", v: "a [[ b" }]);
    expect(parseInline("half **bold")).toEqual([{ t: "text", v: "half **bold" }]);
  });
});
