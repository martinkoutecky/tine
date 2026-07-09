import { describe, expect, it } from "vitest";
import { evaluate } from "./eval";
import { parseFormula, type Ast, type BinaryOp } from "./parser";
import {
  booleanValue,
  errorValue,
  nullValue,
  numberValue,
  parseDateValue,
  textValue,
  type FormulaValue,
} from "./value";

function parseOk(src: string): Ast {
  const result = parseFormula(src);
  expect(result.ok, result.ok ? "" : result.error.message).toBe(true);
  if (!result.ok) throw new Error(result.error.message);
  return result.ast;
}

function dateValue(source: string): FormulaValue {
  const parsed = parseDateValue(source);
  if (!parsed) throw new Error(`Bad date fixture ${source}`);
  return parsed;
}

function evalExpr(src: string, fields: Record<string, FormulaValue> = {}, formulas: Record<string, Ast> = {}, now = new Date(Date.UTC(2026, 0, 15, 12, 34))): FormulaValue {
  return evaluate(parseOk(src), {
    field: (name) => fields[name] ?? errorValue(`Missing field ${name}`),
    formulaAst: (name) => formulas[name] ?? null,
    now,
  });
}

function binaryAst(op: BinaryOp): Ast {
  return { kind: "binary", op, left: { kind: "field", name: "left" }, right: { kind: "field", name: "right" } };
}

describe("formula evaluator", () => {
  it("covers the binary coercion matrix", () => {
    const values: Record<string, FormulaValue> = {
      text: textValue("b"),
      number: numberValue(2),
      boolean: booleanValue(true),
      date: dateValue("2026-01-10"),
      duration: { kind: "duration", n: 1, unit: "d" },
      list: { kind: "list", values: [numberValue(2)] },
      null: nullValue(),
    };
    const ops: BinaryOp[] = ["+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!=", "&&", "||"];
    const comparable = new Set(["text", "number", "boolean", "date"]);

    const shouldSucceed = (op: BinaryOp, left: string, right: string): boolean => {
      if (op === "==" || op === "!=") return true;
      if (op === "+" && ((left === "number" && right === "number") || (left === "text" && right === "text") || (left === "date" && right === "duration"))) return true;
      if (op === "-" && ((left === "number" && right === "number") || (left === "date" && right === "duration"))) return true;
      if ((op === "*" || op === "/" || op === "%") && left === "number" && right === "number") return true;
      if ((op === "<" || op === "<=" || op === ">" || op === ">=") && left === right && comparable.has(left)) return true;
      if (op === "&&" && left === "boolean" && right === "boolean") return true;
      if (op === "||" && left === "boolean") return true;
      return false;
    };

    for (const op of ops) {
      for (const [leftKind, left] of Object.entries(values)) {
        for (const [rightKind, right] of Object.entries(values)) {
          const result = evaluate(binaryAst(op), {
            field: (name) => name === "left" ? left : right,
            formulaAst: () => null,
            now: new Date(Date.UTC(2026, 0, 15)),
          });
          expect(result.kind === "error", `${op} ${leftKind}/${rightKind}`).toBe(!shouldSucceed(op, leftKind, rightKind));
        }
      }
    }
  });

  it("evaluates strict operators and logical short-circuiting", () => {
    expect(evalExpr('"a" + "b"')).toEqual(textValue("ab"));
    expect(evalExpr("1 + 2 * 3")).toEqual(numberValue(7));
    expect(evalExpr('"1" + 2').kind).toBe("error");
    expect(evalExpr("false && missing")).toEqual(booleanValue(false));
    expect(evalExpr("true || missing")).toEqual(booleanValue(true));
    expect(evalExpr("true && 1").kind).toBe("error");
    expect(evalExpr("1 || true").kind).toBe("error");
    expect(evalExpr("!false")).toEqual(booleanValue(true));
    expect(evalExpr("-2")).toEqual(numberValue(-2));
  });

  it("implements every free function with arity and type errors", () => {
    expect(evalExpr('if(true, "yes", missing)')).toEqual(textValue("yes"));
    expect(evalExpr('if(false, missing, "no")')).toEqual(textValue("no"));
    expect(evalExpr("if(1, 2, 3)").kind).toBe("error");
    expect(evalExpr("if(true, 1)").kind).toBe("error");
    expect(evalExpr("isEmpty(null)")).toEqual(booleanValue(true));
    expect(evalExpr('isEmpty("")')).toEqual(booleanValue(true));
    expect(evalExpr("isEmpty(items)", { items: { kind: "list", values: [] } })).toEqual(booleanValue(true));
    expect(evalExpr("isEmpty(1)")).toEqual(booleanValue(false));
    expect(evalExpr("isEmpty()").kind).toBe("error");
    expect(evalExpr('now().format("YYYY-MM-DD HH:mm")')).toEqual(textValue("2026-01-15 12:34"));
    expect(evalExpr('today().format("YYYY-MM-DD HH:mm")')).toEqual(textValue("2026-01-15 00:00"));
    expect(evalExpr("now(1)").kind).toBe("error");
    expect(evalExpr("nope()")).toEqual(errorValue("Unknown function nope"));
  });

  it("implements text, number, date, and list stdlib members", () => {
    expect(evalExpr('"Hello".contains("ell")')).toEqual(booleanValue(true));
    expect(evalExpr('"Hello".lower()')).toEqual(textValue("hello"));
    expect(evalExpr('"  Hello  ".trim()')).toEqual(textValue("Hello"));
    expect(evalExpr('"banana".replace("na", "NA")')).toEqual(textValue("baNAna"));
    expect(evalExpr('"abc".length')).toEqual(numberValue(3));
    expect(evalExpr('"abc".contains(1)').kind).toBe("error");
    expect(evalExpr('"abc".lower(1)').kind).toBe("error");

    expect(evalExpr("(2.6).round()")).toEqual(numberValue(3));
    expect(evalExpr("(2.6).floor()")).toEqual(numberValue(2));
    expect(evalExpr("(2.1).ceil()")).toEqual(numberValue(3));
    expect(evalExpr("(-2.1).abs()")).toEqual(numberValue(2.1));
    expect(evalExpr("(2).toFixed(2)")).toEqual(textValue("2.00"));
    expect(evalExpr("(2).round(1)").kind).toBe("error");
    expect(evalExpr('(2).toFixed("x")').kind).toBe("error");

    const d = dateValue("2026-03-04 05:06");
    expect(evalExpr('d.format("YYYY/MM/DD HH:mm")', { d })).toEqual(textValue("2026/03/04 05:06"));
    expect(evalExpr("d.relative()", { d }, {}, new Date(Date.UTC(2026, 2, 1)))).toEqual(textValue("in 3d"));
    expect(evalExpr("d.year", { d })).toEqual(numberValue(2026));
    expect(evalExpr("d.month", { d })).toEqual(numberValue(3));
    expect(evalExpr("d.day", { d })).toEqual(numberValue(4));
    expect(evalExpr("d.format()", { d }).kind).toBe("error");
    expect(evalExpr("d.format(1)", { d }).kind).toBe("error");

    const items: FormulaValue = { kind: "list", values: [textValue("a"), numberValue(2), nullValue()] };
    expect(evalExpr("items.length", { items })).toEqual(numberValue(3));
    expect(evalExpr('items.join("|")', { items })).toEqual(textValue("a|2|"));
    expect(evalExpr("items.contains(2)", { items })).toEqual(booleanValue(true));
    expect(evalExpr("items.join()", { items }).kind).toBe("error");
    expect(evalExpr("items.join(1)", { items }).kind).toBe("error");
    expect(evalExpr('"x".nope()')).toEqual(errorValue("Unknown method nope for text"));
  });

  it("propagates first errors left-to-right", () => {
    expect(evalExpr("a + b", { a: errorValue("left"), b: errorValue("right") })).toEqual(errorValue("left"));
    expect(evalExpr("a + b", { a: numberValue(1), b: errorValue("right") })).toEqual(errorValue("right"));
    expect(evalExpr("isEmpty(a)", { a: errorValue("arg") })).toEqual(errorValue("arg"));
    expect(evalExpr("a.contains(b)", { a: errorValue("target"), b: errorValue("arg") })).toEqual(errorValue("target"));
    expect(evalExpr("if(a, 1, 2)", { a: errorValue("condition") })).toEqual(errorValue("condition"));

    const listError = evaluate(binaryAst("=="), {
      field: (name) => name === "left" ? { kind: "list", values: [errorValue("nested")] } : { kind: "list", values: [numberValue(1)] },
      formulaAst: () => null,
      now: new Date(Date.UTC(2026, 0, 15)),
    });
    expect(listError).toEqual(errorValue("nested"));
  });

  it("does date comparisons and duration arithmetic with month-end clamping", () => {
    expect(evalExpr('d + "7d"', { d: dateValue("2026-01-10") })).toEqual(dateValue("2026-01-17"));
    expect(evalExpr('d - "2w"', { d: dateValue("2026-01-31") })).toEqual(dateValue("2026-01-17"));
    expect(evalExpr('d + "1M"', { d: dateValue("2026-01-31") })).toEqual(dateValue("2026-02-28"));
    expect(evalExpr('d + "1M"', { d: dateValue("2024-01-31") })).toEqual(dateValue("2024-02-29"));
    expect(evalExpr('d + "1y"', { d: dateValue("2024-02-29") })).toEqual(dateValue("2025-02-28"));
    expect(evalExpr("start < end", { start: dateValue("2026-01-01"), end: dateValue("2026-01-02") })).toEqual(booleanValue(true));
    expect(evalExpr('d + "soon"', { d: dateValue("2026-01-10") }).kind).toBe("error");
  });

  it("detects formula cycles and names the chain", () => {
    const self = { a: parseOk("formula.a") };
    expect(evalExpr("formula.a", {}, self)).toEqual(errorValue("Formula cycle: a -> a"));

    const two = { a: parseOk("formula.b"), b: parseOk("formula.a") };
    expect(evalExpr("formula.a", {}, two)).toEqual(errorValue("Formula cycle: a -> b -> a"));

    const three = { a: parseOk("formula.b"), b: parseOk("formula.c"), c: parseOk("formula.a") };
    expect(evalExpr("formula.a", {}, three)).toEqual(errorValue("Formula cycle: a -> b -> c -> a"));
  });

  it("derives now and today only from the injected context time", () => {
    expect(evalExpr('now().format("YYYY-MM-DD HH:mm")', {}, {}, new Date(Date.UTC(2030, 5, 1, 1, 2)))).toEqual(textValue("2030-06-01 01:02"));
    expect(evalExpr('today().format("YYYY-MM-DD HH:mm")', {}, {}, new Date(Date.UTC(2030, 5, 1, 23, 59)))).toEqual(textValue("2030-06-01 00:00"));
  });
});
