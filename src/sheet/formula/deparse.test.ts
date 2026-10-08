import { describe, expect, it } from "vitest";
import { astToExpr } from "./deparse";
import { parseFormula, type Ast } from "./parser";

const CORPUS = [
  "price",
  "formula.total",
  "0",
  "12.5",
  '"done"',
  '"a \\"quote\\""',
  "true",
  "false",
  "null",
  "-points",
  "!shipped",
  "price * qty",
  "price / qty",
  "price % qty",
  "price + fee",
  "price - discount",
  "a < b",
  "a <= b",
  "a > b",
  "a >= b",
  "a == b",
  "a != b",
  "a && b",
  "a || b",
  "a + b * c",
  "(a + b) * c",
  "a - (b - c)",
  "a && b || c",
  "a && (b || c)",
  'if(isEmpty(status), "todo", status)',
  'if(points > 2, "big", if(shipped, "done", "small"))',
  "isEmpty(status)",
  "now()",
  "today()",
  '"Hello".lower().contains("h")',
  "name.trim().replace(\" \", \"-\")",
  "tasks.length",
  "items.join(\",\")",
  "(2.6).round()",
  "(2).toFixed(2)",
  "due.year",
  'due.format("YYYY-MM-DD")',
  "due.relative()",
  "a - b - c",
  "a - (b - c) - d",
  "(a || b) && c && d",
  "-(a + b) * c / d",
  "(a + b).round().toFixed(2) + c",
  "x.y.z(a + b, c).w",
  "!(a.b + c)",
] as const;

function parseOk(src: string): Ast {
  const result = parseFormula(src);
  expect(result.ok, result.ok ? "" : result.error.message).toBe(true);
  if (!result.ok) throw new Error(result.error.message);
  return result.ast;
}

describe("astToExpr", () => {
  it(`round-trips parsed formula ASTs over a ${CORPUS.length}-formula corpus`, () => {
    expect(CORPUS.length).toBeGreaterThanOrEqual(25);
    for (const src of CORPUS) {
      const ast = parseOk(src);
      const printed = astToExpr(ast);
      expect(parseOk(printed), `${src} -> ${printed}`).toEqual(ast);
    }
  });
});

describe("astToExpr on long chains (og C, I-22)", () => {
  // A valid imported expression may be a left-associative chain thousands of
  // links long (the parser builds it iteratively); printing it must not spend
  // one stack frame per link, or the formula editor's builder throws RangeError.
  it("prints a 10,000-term sum and a 10,000-link member chain", () => {
    for (const src of [
      `${"x - (y - z) + ".repeat(4_000)}w`,
      `if(true, ${"1 + ".repeat(10_000)}1, 0)`,
      `a${".b".repeat(10_000)}.c(1, 2)`,
      `(${"p * ".repeat(5_000)}q).round()${".abs()".repeat(5_000)} + r`,
    ]) {
      const printed = astToExpr(parseOk(src));
      expect(printed).toBe(src);
    }
  });
});
