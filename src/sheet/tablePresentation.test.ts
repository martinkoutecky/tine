import { describe, expect, it } from "vitest";
import { nextQuerySort, queryColumnFieldId, reorderedQueryColumns } from "./tablePresentation";

describe("query table display persistence", () => {
  it("maps Page to the sheet field and saves an ordered complete column list", () => {
    expect(queryColumnFieldId("page")).toBe("page");
    expect(reorderedQueryColumns(["prop:cost", "page", "priority"], "page", "prop:cost", true))
      .toEqual(["page", "cost", "priority"]);
  });
  it("refuses a reorder that would silently hide a formula column", () => {
    expect(reorderedQueryColumns(["prop:cost", "formula:rate", "page"], "page", "prop:cost", true)).toBeNull();
  });
  it("cycles one saved header sort without losing the clear", () => {
    expect(nextQuerySort(undefined, "page")).toEqual([["page", "asc"]]);
    expect(nextQuerySort([["page", "asc"]], "page")).toEqual([["page", "desc"]]);
    expect(nextQuerySort([["page", "desc"]], "page")).toEqual([]);
  });
});
