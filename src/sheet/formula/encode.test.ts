import { describe, expect, it } from "vitest";
import { decodeFormulaExpr, encodeFormulaExpr, formulaNameValid } from "./encode";
import { lexFormula } from "./lexer";

describe("formula property-line encoding", () => {
  it("round-trips nested parens and preserves real spaces between parens", () => {
    const corpus = [
      "((a + b) * c)",
      "(((a)))",
      "( (a))",
      "(  (a))",
      '"(( inside string)"',
      'if(((a + b) > c), "yes", "no")',
    ];
    for (const expr of corpus) {
      const encoded = encodeFormulaExpr(expr);
      expect(encoded.includes("(("), expr).toBe(false);
      expect(decodeFormulaExpr(encoded)).toBe(expr);
      expect(encodeFormulaExpr(decodeFormulaExpr(encoded))).toBe(encoded);
    }
  });

  it("round-trips hash armor inside strings without touching hashes outside strings", () => {
    const corpus = [
      '"#tag"',
      "'color #fff'",
      String.raw`"\#already"`,
      String.raw`"\\#backslash-and-hash"`,
      '"quote \\" # still string"',
      "field # outside-string",
    ];
    for (const expr of corpus) {
      const encoded = encodeFormulaExpr(expr);
      expect(decodeFormulaExpr(encoded)).toBe(expr);
      expect(encodeFormulaExpr(decodeFormulaExpr(encoded))).toBe(encoded);
    }
    expect(encodeFormulaExpr("field # outside-string")).toBe("field # outside-string");
  });

  it("keeps property armor out of lexer input", () => {
    const decoded = decodeFormulaExpr(encodeFormulaExpr('"#tag" + "((x))"'));
    expect(decoded).toBe('"#tag" + "((x))"');
    const lexed = lexFormula(decoded);
    expect(lexed.ok).toBe(true);
    if (!lexed.ok) throw new Error(lexed.error.message);
    expect(lexed.tokens[0]).toMatchObject({ kind: "string", value: "#tag" });
    expect(lexed.tokens[2]).toMatchObject({ kind: "string", value: "((x))" });
  });

  it("validates formula property names", () => {
    expect(formulaNameValid("total")).toBe(true);
    expect(formulaNameValid("due-soon-7")).toBe(true);
    expect(formulaNameValid("Due")).toBe(false);
    expect(formulaNameValid("due_soon")).toBe(false);
    expect(formulaNameValid("")).toBe(false);
  });
});
