import { describe, expect, it } from "vitest";
import { encodeFormulaExpr } from "./formula";
import { formulaFieldId, formulaNameFromField, formulasOf, mergeFormulas } from "./formulaFields";

describe("formula fields", () => {
  it("collects valid formula properties in property order and decodes expressions", () => {
    const encoded = encodeFormulaExpr('"#tag" + "((x))"');
    expect(
      Array.from(
        formulasOf([
          ["title", "ignored"],
          ["tine.formula.total", "price * qty"],
          ["tine.formula.Bad", "nope"],
          ["tine.formula.due-soon", encoded],
        ]).entries()
      )
    ).toEqual([
      ["total", "price * qty"],
      ["due-soon", '"#tag" + "((x))"'],
    ]);
  });

  it("merges page and view formulas by name with view values winning", () => {
    const page = formulasOf([
      ["tine.formula.total", "page_total"],
      ["tine.formula.page-only", "page_only"],
    ]);
    const view = formulasOf([
      ["tine.formula.total", "view_total"],
      ["tine.formula.view-only", "view_only"],
    ]);

    expect(Array.from(mergeFormulas(page, view).entries())).toEqual([
      ["total", "view_total"],
      ["page-only", "page_only"],
      ["view-only", "view_only"],
    ]);
  });

  it("maps formula names to formula field ids", () => {
    expect(formulaFieldId("total")).toBe("formula:total");
    expect(formulaNameFromField("formula:total")).toBe("total");
    expect(formulaNameFromField("prop:total")).toBeNull();
  });
});
