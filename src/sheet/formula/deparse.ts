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

const PREC_MEMBER = 8;
const PREC_UNARY = 7;

function quoteString(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function withParens(text: string, needsParens: boolean): string {
  return needsParens ? `(${text})` : text;
}

type Binary = Extract<Ast, { kind: "binary" }>;
type Member = Extract<Ast, { kind: "member" }>;

/** Print an AST. The parser builds left-associative operator chains and
 *  postfix member chains in a loop, so either may be thousands of links long
 *  inside the 64 KiB source cap; both spines are walked iteratively here, as
 *  `evaluate` walks the binary spine (og C, I-22). Recursion remains only
 *  where the parser itself recursed (right operands, unary, call arguments,
 *  parentheses), which its 1024-level guard bounds. */
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
      const spine: Member[] = [];
      let cursor: Ast = ast;
      while (cursor.kind === "member") {
        spine.push(cursor);
        cursor = cursor.object;
      }
      // Each link prints its object with no parent precedence, so only the
      // top link (spine[0]) can need outer parentheses.
      let text = deparse(cursor);
      for (let i = spine.length - 1; i >= 0; i--) {
        const link = spine[i];
        const object = link.object;
        const needsObjectParens =
          object.kind === "binary" ||
          object.kind === "unary" ||
          (object.kind === "literal" && typeof object.value === "number");
        const args = link.args == null ? "" : `(${link.args.map((arg) => deparse(arg)).join(", ")})`;
        text = withParens(`${withParens(text, needsObjectParens)}.${link.name}${args}`, PREC_MEMBER < (i === 0 ? parentPrecedence : 0));
      }
      return text;
    }
    case "binary": {
      const spine: Binary[] = [];
      let cursor: Ast = ast;
      while (cursor.kind === "binary") {
        spine.push(cursor);
        cursor = cursor.left;
      }
      // spine[i] is printed as the left operand of spine[i - 1]; spine[0] is
      // printed in this call's own context.
      let text = deparse(cursor, BINARY_PRECEDENCE[spine[spine.length - 1].op], "left");
      for (let i = spine.length - 1; i >= 0; i--) {
        const node = spine[i];
        const prec = BINARY_PRECEDENCE[node.op];
        const outer = i === 0 ? parentPrecedence : BINARY_PRECEDENCE[spine[i - 1].op];
        const outerSide = i === 0 ? side : "left";
        const right = deparse(node.right, prec, "right");
        text = withParens(`${text} ${node.op} ${right}`, prec < outer || (outerSide === "right" && prec === outer));
      }
      return text;
    }
  }
}

export function astToExpr(ast: Ast): string {
  return deparse(ast);
}
