import { describe, expect, it } from "vitest";
import { previewSpans } from "./cm6Preview";

describe("nonshipping CM6 preview from lsdoc", () => {
  it("uses UTF-16 coordinates after Unicode and styles parsed syntax", () => {
    const raw = "TODO 中文😀 **bold** *italic* `code` [[Target]]";
    const spans = previewSpans(raw, "md");
    expect(spans.map(s => [s.kind, raw.slice(s.from, s.to)])).toEqual([
      ["task", "TODO"], ["bold", "**bold**"], ["italic", "*italic*"],
      ["code", "`code`"], ["page", "[[Target]]"],
    ]);
  });
  it("does not decorate apparent syntax inside code or escaped source", () => {
    const spans = previewSpans("`[[not a link]]` \\*plain*", "md");
    expect(spans.map(s => s.kind)).toEqual(["code"]);
  });
  it("takes Org syntax and task ranges from the same parser", () => {
    const raw = "DOING *bold* /italic/ ~code~ [[Target]]";
    expect(previewSpans(raw, "org").map(s => [s.kind, raw.slice(s.from, s.to)]))
      .toEqual([["task", "DOING"], ["bold", "*bold*"], ["italic", "/italic/"], ["code", "~code~"], ["page", "[[Target]]"]]);
  });
});
