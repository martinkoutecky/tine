import { describe, expect, it } from "vitest";
import { concealedSpans, previewSpans } from "./cm6Preview";
import { cm6PreviewCases } from "../tests/cm6PreviewCases";
import { facetsOf } from "../render/facets";

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

// CM6-DEADLINE (Martin, 2026-10-30): a standalone SCHEDULED/DEADLINE line showed as bare source text or as a
// `<date>` code chip in edit mode but as the view's date chip when read.
describe("planning lines follow the view's date chip", () => {
  const raw = "Pay the invoice\nDEADLINE: <2026-10-31 Fri>";
  it("makes one parser-owned planning span that carries the line break before it", () => {
    const [span, ...rest] = previewSpans(raw, "md");
    expect(rest).toEqual([]);
    expect(span.kind).toBe("planning");
    expect(span.planning).toEqual({ ts: "Deadline", value: "2026-10-31 Fri" });
    expect(raw.slice(span.from, span.to)).toBe("\nDEADLINE: <2026-10-31 Fri>");
    expect(raw.slice(span.revealFrom!, span.to)).toBe("DEADLINE: <2026-10-31 Fri>");
  });
  it("shows the same text as the view's chip, repeaters and times included", () => {
    for (const [text, ts] of [["Write\nSCHEDULED: <2026-10-31 Fri>", "Scheduled"], ["Water\nDEADLINE: <2026-10-31 Fri 14:00 +1w>", "Deadline"],
      ["Water\nDEADLINE: <2026-10-31 Fri .+2d>", "Deadline"], ["Indented\n  SCHEDULED: <2026-10-31 Fri 09:00-10:00>", "Scheduled"]] as const) {
      const span = previewSpans(text, "md").find(s => s.kind === "planning")!;
      const facets = facetsOf(text, "md");
      expect(span.planning).toEqual({ ts, value: ts === "Deadline" ? facets.deadline : facets.scheduled });
    }
  });
  it("renders when the caret is elsewhere, including the end of the header line, and reveals only inside the line", () => {
    const spans = previewSpans(raw, "md");
    const at = (n: number) => concealedSpans(spans, [{ from: n, to: n }]).length;
    expect([at(0), at(7), at(15)]).toEqual([1, 1, 1]);
    expect([at(16), at(20), at(raw.length)]).toEqual([0, 0, 0]);
    expect(concealedSpans(spans, [{ from: 0, to: 15 }]).length).toBe(1);
    expect(concealedSpans(spans, [{ from: 0, to: raw.length }]).length).toBe(0);
  });
  it("treats scheduled and deadline lines separately and leaves lookalikes in the text", () => {
    const both = "Review\nSCHEDULED: <2026-10-30 Thu>\nDEADLINE: <2026-11-02 Mon>";
    expect(previewSpans(both, "md").map(s => s.planning?.ts)).toEqual(["Scheduled", "Deadline"]);
    expect(previewSpans("Discuss DEADLINE: <2026-10-31 Fri> later", "md").some(s => s.kind === "planning")).toBe(false);
    expect(previewSpans("`DEADLINE: <2026-10-31 Fri>`", "md").some(s => s.kind === "planning")).toBe(false);
  });
  it("keeps body text that follows the timestamp on its own line, and takes CRLF and indentation with the break", () => {
    const text = "Head\nDEADLINE: <2026-10-31 Fri> and a note";
    const [span] = previewSpans(text, "md").filter(s => s.kind === "planning");
    expect(text.slice(span.from, span.to)).toBe("DEADLINE: <2026-10-31 Fri>");
    expect(span.revealFrom).toBeUndefined();
    const crlf = "Head\r\n  DEADLINE: <2026-10-31 Fri>  ";
    const [wrapped] = previewSpans(crlf, "md").filter(s => s.kind === "planning");
    expect(crlf.slice(wrapped.from, wrapped.to)).toBe("\r\n  DEADLINE: <2026-10-31 Fri>  ");
    expect(crlf.slice(wrapped.revealFrom!, wrapped.to)).toBe("  DEADLINE: <2026-10-31 Fri>  ");
  });
  it("renders under a heading as the same chip", () => {
    const text = "# Title\nDEADLINE: <2026-10-31 Fri>";
    expect(previewSpans(text, "md").map(s => [s.kind, text.slice(s.from, s.to)])).toEqual([["bullet", "# Title"], ["planning", "\nDEADLINE: <2026-10-31 Fri>"]]);
  });
});
