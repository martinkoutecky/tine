import { describe, expect, it } from "vitest";
import { looksLikeDelimitedText, parseDelimitedText, serializeTsv } from "./tsv";

describe("sheet TSV/CSV parsing", () => {
  it("parses TSV and lets tabs win over commas", () => {
    expect(parseDelimitedText("a,b\tc\nd,e\tf")).toEqual([
      ["a,b", "c"],
      ["d,e", "f"],
    ]);
  });

  it("parses minimally quoted CSV fields", () => {
    expect(parseDelimitedText('"a,b",c\n"d""e",f')).toEqual([
      ["a,b", "c"],
      ['d"e', "f"],
    ]);
  });

  it("lets callers choose CSV or TSV explicitly for file extensions", () => {
    expect(parseDelimitedText("a,b\tc", "csv")).toEqual([["a", "b\tc"]]);
    expect(parseDelimitedText("a,b\tc", "tsv")).toEqual([["a,b", "c"]]);
  });

  it("serializes holes as empty TSV fields", () => {
    expect(serializeTsv([["a", null, "c"], [undefined, "e"]])).toBe("a\t\tc\n\te");
  });

  it("keeps plain comma-space text out of CSV mode", () => {
    expect(looksLikeDelimitedText("hello, world")).toBe(false);
    expect(looksLikeDelimitedText("a,b")).toBe(true);
  });
});
