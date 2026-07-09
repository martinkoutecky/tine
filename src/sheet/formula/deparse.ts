import type { Ast, BinaryOp } from "./parser";

const BINARY_PRECEDENCE: Record<BinaryOp, number> = {
  "||": 1,
  "&&": 2,
  "==": 3,
  "!=": 3,
  "<": 4,
  "<=": 4,
  ">": 4,
  ">=": 4,
  "+": 5,
  "-": 5,
  "*": 6,
  "/": 6,
  "%": 6,
};

const PREC_ATOM = 9;
const PREC_MEMBER = 8;
const PREC_UNARY = 7;

function quoteString(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function precedence(ast: Ast): number {
  switch (ast.kind) {
    case "literal":
    case "field":
    case "formulaRef":
    case "call":
      return PREC_ATOM;
    case "member":
      return PREC_MEMBER;
    case "unary":
      return PREC_UNARY;
    case "binary":
      return BINARY_PRECEDENCE[ast.op];
  }
}

function withParens(text: string, needsParens: boolean): string {
  return needsParens ? `(${text})` : text;
}

function deparse(ast: Ast, parentPrecedence = 0, side: "left" | "right" | null = null): string {
  switch (ast.kind) {
    case "literal":
      if (typeof ast.value === "string") return quoteString(ast.value);
      return ast.value === null ? "null" : String(ast.value);
    case "field":
      return ast.name;
    case "formulaRef":
      return `formula.${ast.name}`;
    case "call":
      return `${ast.name}(${ast.args.map((arg) => deparse(arg)).join(", ")})`;
    case "unary": {
      const inner = deparse(ast.expr, PREC_UNARY);
      return withParens(`${ast.op}${inner}`, PREC_UNARY < parentPrecedence);
    }
    case "member": {
      const object = deparse(ast.object);
      const needsObjectParens =
        ast.object.kind === "binary" ||
        ast.object.kind === "unary" ||
        (ast.object.kind === "literal" && typeof ast.object.value === "number");
      const args = ast.args == null ? "" : `(${ast.args.map((arg) => deparse(arg)).join(", ")})`;
      return withParens(`${withParens(object, needsObjectParens)}.${ast.name}${args}`, PREC_MEMBER < parentPrecedence);
    }
    case "binary": {
      const prec = BINARY_PRECEDENCE[ast.op];
      const left = deparse(ast.left, prec, "left");
      const right = deparse(ast.right, prec, "right");
      const text = `${left} ${ast.op} ${right}`;
      return withParens(text, prec < parentPrecedence || (side === "right" && prec === parentPrecedence));
    }
  }
}

export function astToExpr(ast: Ast): string {
  return deparse(ast);
}
