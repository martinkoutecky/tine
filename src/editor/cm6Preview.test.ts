import { describe, expect, it } from "vitest";
import { concealedSpans, previewSpans } from "./cm6Preview";
import { cm6PreviewCases } from "../tests/cm6PreviewCases";

describe("experimental CM6 preview from lsdoc", () => {
  it("uses UTF-16 coordinates after Unicode and styles parsed syntax", () => {
    const raw = "TODO 中文😀 **bold** *italic* `code` [[Target]]";
    const spans = previewSpans(raw, "md");
    expect(spans.map(s => [s.kind, raw.slice(s.from, s.to)])).toEqual([
      ["task", "TODO"], ["bold", "**bold**"], ["italic", "*italic*"],
      ["code", "`code`"], ["page", "[[Target]]"],
    ]);
  });
  it("keeps code opaque and treats an escaped plain run as one parser-owned span", () => {
    const spans = previewSpans("`[[not a link]]` \\*plain*", "md");
    expect(spans.map(s => s.kind)).toEqual(["code", "plain"]);
  });
  it("takes Org syntax and task ranges from the same parser", () => {
    const raw = "DOING *bold* /italic/ ~code~ [[Target]]";
    expect(previewSpans(raw, "org").map(s => [s.kind, raw.slice(s.from, s.to)]))
      .toEqual([["task", "DOING"], ["bold", "*bold*"], ["italic", "/italic/"], ["code", "~code~"], ["page", "[[Target]]"]]);
  });
  it.each(cm6PreviewCases)("applies the shared policy to $name", fixture => {
    const raw = fixture.raw ?? `before ${fixture.source} after`;
    const spans = previewSpans(raw, fixture.format);
    const span = spans.find(s => s.kind === fixture.kind)!;
    expect(span).toBeDefined();
    const outside = span.from > 0 ? 0 : raw.length;
    expect(concealedSpans(spans, [{ from: outside, to: outside }])).toContain(span);
    for (const [from, to] of [[span.from, span.from], [span.to, span.to], [span.from + 1, span.from + 1], [Math.max(0, span.from - 1), Math.min(raw.length, span.to + 1)]]) {
      expect(concealedSpans(spans, [{ from, to }])).not.toContain(span);
    }
  });
  it("honors secondary selections and hides descendants only under a rendered parent", () => {
    const spans = previewSpans("before **bold *inner* tail** and `code` after", "md");
    const [bold, italic, code] = spans;
    expect(concealedSpans(spans, [{ from: 0, to: 0 }])).toEqual([bold, code]);
    expect(concealedSpans(spans, [{ from: bold.from + 3, to: bold.from + 3 }])).toEqual([italic, code]);
    expect(concealedSpans(spans, [{ from: 0, to: 0 }, { from: italic.from + 1, to: code.to }])).toEqual([]);
  });
});
