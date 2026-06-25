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
    expect(parseInline("$E=mc^2$")).toEqual([{ t: "math", tex: "E=mc^2", display: false }]);
  });

  it("links and images", () => {
    expect(parseInline("[label](http://x.com)")).toEqual([
      { t: "link", label: "label", url: "http://x.com" },
    ]);
    expect(parseInline("![alt](a.png)")).toEqual([{ t: "image", alt: "alt", url: "a.png" }]);
  });

  it("image sizing metadata", () => {
    expect(parseInline("![a](x.png){:width 200}")).toEqual([
      { t: "image", alt: "a", url: "x.png", width: "200px" },
    ]);
    expect(parseInline("![a](x.png){:height 50%}")).toEqual([
      { t: "image", alt: "a", url: "x.png", height: "50%" },
    ]);
  });

  it("autolinks (bare url + angle)", () => {
    expect(parseInline("see https://x.com/p here")).toEqual([
      { t: "text", v: "see " },
      { t: "link", label: "https://x.com/p", url: "https://x.com/p" },
      { t: "text", v: " here" },
    ]);
    // trailing sentence punctuation excluded
    expect(parseInline("at https://x.com.")).toEqual([
      { t: "text", v: "at " },
      { t: "link", label: "https://x.com", url: "https://x.com" },
      { t: "text", v: "." },
    ]);
    expect(parseInline("<https://x.com>")).toEqual([
      { t: "link", label: "https://x.com", url: "https://x.com" },
    ]);
    // a closing quote after the URL is NOT part of the link
    expect(parseInline('"see https://x.com/p"')).toEqual([
      { t: "text", v: '"see ' },
      { t: "link", label: "https://x.com/p", url: "https://x.com/p" },
      { t: "text", v: '"' },
    ]);
  });

  it("footnote reference", () => {
    expect(parseInline("see note[^1] here")).toEqual([
      { t: "text", v: "see note" },
      { t: "footnote", id: "1" },
      { t: "text", v: " here" },
    ]);
  });

  it("sandboxed iframe embed (http src only)", () => {
    expect(parseInline('<iframe src="https://example.com" width="320" height="200"></iframe>')).toEqual([
      { t: "iframe", src: "https://example.com", width: "320", height: "200" },
    ]);
    // non-http src is not honoured (stays literal text-ish, no iframe seg)
    expect(parseInline('<iframe src="javascript:alert(1)"></iframe>').some((s) => s.t === "iframe")).toBe(false);
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
    // index-scanner: every other unterminated opener also falls through cleanly
    expect(parseInline("x `code")).toEqual([{ t: "text", v: "x `code" }]);
    expect(parseInline("y ((ref")).toEqual([{ t: "text", v: "y ((ref" }]);
    expect(parseInline("z {{macro")).toEqual([{ t: "text", v: "z {{macro" }]);
    expect(parseInline("eq $x+1")).toEqual([{ t: "text", v: "eq $x+1" }]);
  });

  it("adjacent tokens emit no empty text segments (index cursor)", () => {
    // No text between the two refs, and tokens at both ends → no "" text segs.
    expect(parseInline("[[A]][[B]]")).toEqual([
      { t: "pageref", name: "A" },
      { t: "pageref", name: "B" },
    ]);
    expect(parseInline("`c`**b**")).toEqual([
      { t: "code", v: "c" },
      { t: "bold", v: [{ t: "text", v: "b" }] },
    ]);
  });

  it("long plain run is one text segment", () => {
    const long = "x".repeat(5000);
    expect(parseInline(long)).toEqual([{ t: "text", v: long }]);
    // trailing token after a long run still splits correctly
    expect(parseInline(long + "[[P]]")).toEqual([
      { t: "text", v: long },
      { t: "pageref", name: "P" },
    ]);
  });

  describe("org format", () => {
    const org = (s: string) => parseInline(s, "org");

    it("emphasis markers differ from markdown", () => {
      expect(org("*b*")).toEqual([{ t: "bold", v: [{ t: "text", v: "b" }] }]);
      expect(org("/i/")).toEqual([{ t: "italic", v: [{ t: "text", v: "i" }] }]);
      expect(org("_u_")).toEqual([{ t: "underline", v: [{ t: "text", v: "u" }] }]);
      expect(org("+s+")).toEqual([{ t: "strike", v: [{ t: "text", v: "s" }] }]);
      expect(org("~c~")).toEqual([{ t: "code", v: "c" }]);
      expect(org("=v=")).toEqual([{ t: "code", v: "v" }]);
      expect(org("^^h^^")).toEqual([{ t: "highlight", v: [{ t: "text", v: "h" }] }]);
    });

    it("boundary rules avoid false positives in plain text", () => {
      // path slashes, snake_case, arithmetic, ~/home must NOT become emphasis
      for (const s of ["a/b/c", "snake_case_var", "2*3*4", "~/home/x", "x=y=z"]) {
        expect(org(s)).toEqual([{ t: "text", v: s }]);
      }
    });

    it("emphasis only at word boundaries", () => {
      expect(org("a *bold* b")).toEqual([
        { t: "text", v: "a " },
        { t: "bold", v: [{ t: "text", v: "bold" }] },
        { t: "text", v: " b" },
      ]);
    });

    it("org links: plain, aliased, and external", () => {
      expect(org("[[Page Name]]")).toEqual([{ t: "pageref", name: "Page Name" }]);
      expect(org("[[Target][Display]]")).toEqual([
        { t: "pageref", name: "Target", alias: "Display" },
      ]);
      expect(org("[[https://x.com][site]]")).toEqual([
        { t: "link", label: "site", url: "https://x.com" },
      ]);
      expect(org("[[file:./pages/foo.org][Foo]]")).toEqual([
        { t: "pageref", name: "foo", alias: "Foo" },
      ]);
    });

    it("markdown-only inline forms stay literal in org", () => {
      // backtick code is markdown-only; in org `~code~` is used instead.
      expect(org("a `code` b")).toEqual([{ t: "text", v: "a `code` b" }]);
    });
  });
});
