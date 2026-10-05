import { describe, expect, it } from "vitest";
import { queryColumnFieldId, reorderedQueryColumns } from "./tablePresentation";

describe("query table display persistence", () => {
  it("maps Page to the sheet field and saves an ordered complete column list", () => {
    expect(queryColumnFieldId("page")).toBe("page");
    expect(reorderedQueryColumns(["prop:cost", "page", "priority"], "page", "prop:cost", true))
      .toEqual(["page", "cost", "priority"]);
  });
  it("refuses a reorder that would silently hide a formula column", () => {
    expect(reorderedQueryColumns(["prop:cost", "formula:rate", "page"], "page", "prop:cost", true)).toBeNull();
  });
});
