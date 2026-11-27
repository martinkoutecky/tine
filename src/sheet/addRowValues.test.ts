import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { new_checkbox_row_raw } from "../render/wasm/lsdoc_wasm";

// tine#1047: the values a table's Add row writes on its new row come from the
// core (`tine_core::table_row`); the shipped module gives its answer.
describe("new_checkbox_row_raw", () => {
  beforeAll(async () => { await initParser(); });

  it("writes false for plain, ordinary columns below an empty title line", () => {
    const keys = ["Shipped", "id", "file-path", "a b", "ready"];
    expect(new_checkbox_row_raw(false, keys)).toBe("\nShipped:: false\nready:: false");
    expect(new_checkbox_row_raw(true, keys)).toBe("\n:PROPERTIES:\n:Shipped: false\n:ready: false\n:END:");
  });

  it("leaves the row empty for two spellings of one key, and in Org for an end or properties column", () => {
    for (const org of [false, true]) expect(new_checkbox_row_raw(org, ["Shipped", "shipped", "ready"])).toBe("");
    expect(new_checkbox_row_raw(true, ["ready", "end"])).toBe("");
    expect(new_checkbox_row_raw(true, ["ready", "Properties"])).toBe("");
    expect(new_checkbox_row_raw(false, ["ready", "end"])).toBe("\nready:: false");
  });

  it("gives an empty row when no column can take a value", () => {
    for (const org of [false, true]) {
      expect(new_checkbox_row_raw(org, [])).toBe("");
      expect(new_checkbox_row_raw(org, ["id", "État", "EnD"])).toBe("");
    }
  });
});
