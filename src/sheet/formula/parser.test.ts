import { describe, expect, it } from "vitest";
import { decodeFormulaExpr, encodeFormulaExpr } from "./encode";
import { parseFormula, type Ast, type BinaryOp } from "./parser";

function parseOk(src: string): Ast {
  const result = parseFormula(src);
  expect(result.ok, result.ok ? "" : result.error.message).toBe(true);
  if (!result.ok) throw new Error(result.error.message);
  return result.ast;
}

function field(name: string): Ast {
  return { kind: "field", name };
}

function printAst(ast: Ast): string {
  switch (ast.kind) {
    case "literal":
      if (typeof ast.value === "string") return `"${ast.value.replace(/\\/g, "\\\\").replace(/"/g, "\\\"").replace(/#/g, "\\#")}"`;
      return ast.value === null ? "null" : String(ast.value);
    case "field":
      return ast.name;
    case "formulaRef":
      return `formula.${ast.name}`;
    case "unary":
      return `(${ast.op}${printAst(ast.expr)})`;
    case "binary":
      return `(${printAst(ast.left)} ${ast.op} ${printAst(ast.right)})`;
    case "call":
      return `${ast.name}(${ast.args.map(printAst).join(", ")})`;
    case "member": {
      const object = ast.object.kind === "literal" && typeof ast.object.value === "number" ? `(${printAst(ast.object)})` : printAst(ast.object);
      return `${object}.${ast.name}${ast.args ? `(${ast.args.map(printAst).join(", ")})` : ""}`;
    }
  }
}

function lcg(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state;
  };
}

describe("formula parser", () => {
  it("implements the ADR precedence table pairwise", () => {
    const ops: readonly { op: BinaryOp; precedence: number }[] = [
      { op: "*", precedence: 6 },
      { op: "/", precedence: 6 },
      { op: "%", precedence: 6 },
      { op: "+", precedence: 5 },
      { op: "-", precedence: 5 },
      { op: "<", precedence: 4 },
      { op: "<=", precedence: 4 },
      { op: ">", precedence: 4 },
      { op: ">=", precedence: 4 },
      { op: "==", precedence: 3 },
      { op: "!=", precedence: 3 },
      { op: "&&", precedence: 2 },
      { op: "||", precedence: 1 },
    ];

    for (const leftOp of ops) {
      for (const rightOp of ops) {
        const ast = parseOk(`a ${leftOp.op} b ${rightOp.op} c`);
        expect(ast.kind, `${leftOp.op} then ${rightOp.op}`).toBe("binary");
        if (ast.kind !== "binary") continue;
        if (leftOp.precedence < rightOp.precedence) {
          expect(ast.op).toBe(leftOp.op);
          expect(ast.right).toMatchObject({ kind: "binary", op: rightOp.op });
        } else {
          expect(ast.op).toBe(rightOp.op);
          expect(ast.left).toMatchObject({ kind: "binary", op: leftOp.op });
        }
      }
    }
  });

  it("parses unary operators tighter than binary operators", () => {
    expect(parseOk("!-a * b")).toEqual({
      kind: "binary",
      op: "*",
      left: { kind: "unary", op: "!", expr: { kind: "unary", op: "-", expr: field("a") } },
      right: field("b"),
    });
  });

  it("parses formula refs, calls, properties, and method chains", () => {
    expect(parseOk('formula.total.round().toFixed(2).contains("0")')).toEqual({
      kind: "member",
      name: "contains",
      args: [{ kind: "literal", value: "0" }],
      object: {
        kind: "member",
        name: "toFixed",
        args: [{ kind: "literal", value: 2 }],
        object: {
          kind: "member",
          name: "round",
          args: [],
          object: { kind: "formulaRef", name: "total" },
        },
      },
    });
    expect(parseOk("tasks.length")).toEqual({ kind: "member", object: field("tasks"), name: "length", args: null });
  });

  it("reports parse errors with offsets", () => {
    expect(parseFormula("a..b()")).toEqual({ ok: false, error: { offset: 2, message: "Expected member name after ." } });
    expect(parseFormula("(a")).toEqual({ ok: false, error: { offset: 2, message: "Expected )" } });
    expect(parseFormula("a +")).toEqual({ ok: false, error: { offset: 3, message: "Expected expression" } });
  });

  it("handles deep grouping without stack overflow at 500 parens", () => {
    const src = `${"(".repeat(500)}a${")".repeat(500)}`;
    expect(parseOk(src)).toEqual(field("a"));
  });

  it("round-trips printed random ASTs and property armor with a seeded fuzz", () => {
    const rand = lcg(0x5eed1234);
    const names = ["price", "qty", "status", "due-date", "tags"];
    const formulas = ["total", "due-soon", "flag"];
    const strings = ["alpha", "#tag", "((nested))", "a \"quote\"", "slash \\"];
    const binaries: BinaryOp[] = ["+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!=", "&&", "||"];

    const pick = <T,>(items: readonly T[]): T => items[rand() % items.length];
    const gen = (depth: number): Ast => {
      if (depth <= 0) {
        const leaf = rand() % 7;
        if (leaf === 0) return { kind: "literal", value: (rand() % 1000) / 10 };
        if (leaf === 1) return { kind: "literal", value: pick(strings) };
        if (leaf === 2) return { kind: "literal", value: (rand() & 1) === 1 };
        if (leaf === 3) return { kind: "literal", value: null };
        if (leaf === 4) return { kind: "formulaRef", name: pick(formulas) };
        return { kind: "field", name: pick(names) };
      }
      switch (rand() % 6) {
        case 0:
          return { kind: "unary", op: (rand() & 1) === 1 ? "!" : "-", expr: gen(depth - 1) };
        case 1:
          return { kind: "binary", op: pick(binaries), left: gen(depth - 1), right: gen(depth - 1) };
        case 2:
          return { kind: "call", name: pick(["if", "isEmpty", "now", "today", "custom-fn"]), args: [gen(depth - 1), gen(depth - 1)].slice(0, rand() % 3) };
        case 3:
          return { kind: "member", object: gen(depth - 1), name: pick(["length", "lower", "round", "contains"]), args: null };
        case 4:
          return { kind: "member", object: gen(depth - 1), name: pick(["trim", "toFixed", "join", "format"]), args: [gen(depth - 1)].slice(0, rand() % 2) };
        default:
          return gen(0);
      }
    };

    for (let i = 0; i < 500; i += 1) {
      const ast = gen(4);
      const printed = printAst(ast);
      expect(parseOk(printed)).toEqual(ast);
      const encoded = encodeFormulaExpr(printed);
      expect(decodeFormulaExpr(encoded)).toBe(printed);
      expect(encodeFormulaExpr(decodeFormulaExpr(encoded))).toBe(encoded);
    }
  });
});
