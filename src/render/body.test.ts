import { describe, it, expect } from "vitest";
import { segmentBody } from "./body";

describe("segmentBody block constructs", () => {
  it("blockquote run", () => {
    expect(segmentBody(["> a", "> b"])).toEqual([{ kind: "quote", lines: ["a", "b"] }]);
  });

  it("callout (> [!NOTE] title)", () => {
    expect(segmentBody(["> [!NOTE] Heads up", "> body"])).toEqual([
      { kind: "callout", kind2: "note", title: "Heads up", lines: ["body"] },
    ]);
  });

  it("callout with no title or body", () => {
    expect(segmentBody(["> [!WARNING]"])).toEqual([
      { kind: "callout", kind2: "warning", title: "", lines: [] },
    ]);
  });

  it("horizontal rule", () => {
    expect(segmentBody(["---"])).toEqual([{ kind: "hr" }]);
    expect(segmentBody(["***"])).toEqual([{ kind: "hr" }]);
  });

  it("table with alignment", () => {
    const segs = segmentBody(["| a | b | c |", "|:--|:-:|--:|", "| 1 | 2 | 3 |"]);
    expect(segs).toEqual([
      {
        kind: "table",
        rows: [
          ["a", "b", "c"],
          ["1", "2", "3"],
        ],
        aligns: ["left", "center", "right"],
      },
    ]);
  });

  it("quote does not swallow following plain lines", () => {
    expect(segmentBody(["> q", "plain"])).toEqual([
      { kind: "quote", lines: ["q"] },
      { kind: "lines", lines: ["plain"] },
    ]);
  });
});
