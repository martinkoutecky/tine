import { readFileSync } from "node:fs";
import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { implicitPropertyParts } from "./PagePropertyValue";

// I-12: member boundaries come from the parser's `split_linkable_property`
// (the native separator), so this component cannot grow a separator of its own.
describe("implicit page-property members", () => {
  beforeAll(async () => { await initParser(); });

  it("keeps ASCII and fullwidth separators and the authored spacing", () => {
    expect(implicitPropertyParts("a, b，c ,  d")).toEqual([
      { leading: "", value: "a", trailing: "" }, ",",
      { leading: " ", value: "b", trailing: "" }, "，",
      { leading: "", value: "c", trailing: " " }, ",",
      { leading: "  ", value: "d", trailing: "" },
    ]);
  });

  it("keeps empty members in place", () => {
    expect(implicitPropertyParts(",a,, ")).toEqual(["", ",", { leading: "", value: "a", trailing: "" }, ",", "", ",", " "]);
  });

  it("reassembles to the stored text", () => {
    for (const value of ["", "a", "a,b", "，a，，b，", " x , y "]) {
      const text = implicitPropertyParts(value).map((p) => typeof p === "string" ? p : p.leading + p.value + p.trailing).join("");
      expect(text, JSON.stringify(value)).toBe(value);
    }
  });

  it("names no separator of its own (I-12; exemplar crates/tine-core/src/block_regions.rs is_linkable_property_separator)", () => {
    const source = readFileSync("src/components/PagePropertyValue.tsx", "utf8").replace(/\/\*[\s\S]*?\*\/|\/\/[^\n]*/g, "");
    expect(source).not.toMatch(/[，,]\s*["'\]/]|\[,，\]|\[，,\]/);
  });
});
