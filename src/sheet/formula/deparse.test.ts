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
