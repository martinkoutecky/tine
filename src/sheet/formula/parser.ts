import { lexFormula, type FormulaParseError, type OperatorToken, type Token } from "./lexer";

export type LiteralValue = string | number | boolean | null;

export type Ast =
  | { kind: "literal"; value: LiteralValue }
  | { kind: "field"; name: string }
  | { kind: "formulaRef"; name: string }
  | { kind: "unary"; op: UnaryOp; expr: Ast }
  | { kind: "binary"; op: BinaryOp; left: Ast; right: Ast }
  | { kind: "call"; name: string; args: Ast[] }
  | { kind: "member"; object: Ast; name: string; args: Ast[] | null };

export type UnaryOp = "!" | "-";
export type BinaryOp = "*" | "/" | "%" | "+" | "-" | "<" | "<=" | ">" | ">=" | "==" | "!=" | "&&" | "||";

export type ParseResult = { ok: true; ast: Ast } | { ok: false; error: FormulaParseError };

const MAX_PARSE_DEPTH = 1024;

function binaryPrecedence(op: OperatorToken): number | null {
  switch (op) {
    case "||":
      return 1;
    case "&&":
      return 2;
    case "==":
    case "!=":
      return 3;
    case "<":
    case "<=":
    case ">":
    case ">=":
      return 4;
    case "+":
    case "-":
      return 5;
    case "*":
    case "/":
    case "%":
      return 6;
    default:
      return null;
  }
}

class Parser {
  private pos = 0;
  private error: FormulaParseError | null = null;

  constructor(private readonly tokens: readonly Token[]) {}

  parse(): ParseResult {
    const ast = this.parseExpression(0, 0);
    if (!ast) return { ok: false, error: this.error ?? { offset: this.current().offset, message: "Expected expression" } };
    if (this.current().kind !== "eof") {
      return { ok: false, error: { offset: this.current().offset, message: "Unexpected token after expression" } };
    }
    return { ok: true, ast };
  }

  private current(): Token {
    return this.tokens[this.pos] ?? this.tokens[this.tokens.length - 1];
  }

  private next(): Token {
    return this.tokens[this.pos + 1] ?? this.tokens[this.tokens.length - 1];
  }

  private third(): Token {
    return this.tokens[this.pos + 2] ?? this.tokens[this.tokens.length - 1];
  }

  private advance(): Token {
    const token = this.current();
    if (token.kind !== "eof") this.pos += 1;
    return token;
  }

  private fail(offset: number, message: string): null {
    if (!this.error) this.error = { offset, message };
    return null;
  }

  private matchPunct(punct: "(" | ")" | "." | ","): boolean {
    const token = this.current();
    if (token.kind === "punct" && token.punct === punct) {
      this.advance();
      return true;
    }
    return false;
  }

  private parseExpression(minPrecedence: number, depth: number): Ast | null {
    if (depth > MAX_PARSE_DEPTH) return this.fail(this.current().offset, "Formula is nested too deeply");
    let left = this.parsePrefix(depth);
    if (!left) return null;
    left = this.parsePostfix(left, depth);
    if (!left) return null;

    while (true) {
      const token = this.current();
      if (token.kind !== "operator") break;
      const precedence = binaryPrecedence(token.op);
      if (precedence == null || precedence < minPrecedence) break;
      this.advance();
      const right = this.parseExpression(precedence + 1, depth + 1);
      if (!right) return null;
      left = { kind: "binary", op: token.op as BinaryOp, left, right };
    }

    return left;
  }

  private parsePrefix(depth: number): Ast | null {
    const token = this.current();
    if (token.kind === "number") {
      this.advance();
      return { kind: "literal", value: token.value };
    }
    if (token.kind === "string") {
      this.advance();
      return { kind: "literal", value: token.value };
    }
    if (token.kind === "identifier") {
      if (token.text === "true" || token.text === "false") {
        this.advance();
        return { kind: "literal", value: token.text === "true" };
      }
      if (token.text === "null") {
        this.advance();
        return { kind: "literal", value: null };
      }
      // `formula.<name>` is reserved for formula refs before ordinary member
      // access so named formulas can be addressed as pseudo-fields.
      const next = this.next();
      const third = this.third();
      if (token.text === "formula" && next.kind === "punct" && next.punct === "." && third.kind === "identifier") {
        this.advance();
        this.advance();
        const name = this.advance();
        return { kind: "formulaRef", name: name.kind === "identifier" ? name.text : "" };
      }
      this.advance();
      if (this.matchPunct("(")) {
        const args = this.parseArguments(depth + 1);
        if (!args) return null;
        return { kind: "call", name: token.text, args };
      }
      return { kind: "field", name: token.text };
    }
    if (token.kind === "operator" && (token.op === "!" || token.op === "-")) {
      this.advance();
      const expr = this.parseExpression(7, depth + 1);
      if (!expr) return null;
      return { kind: "unary", op: token.op, expr };
    }
    if (this.matchPunct("(")) {
      const expr = this.parseExpression(0, depth + 1);
      if (!expr) return null;
      if (!this.matchPunct(")")) return this.fail(this.current().offset, "Expected )");
      return expr;
    }
    return this.fail(token.offset, "Expected expression");
  }

  private parsePostfix(base: Ast, depth: number): Ast | null {
    let expr = base;
    while (this.matchPunct(".")) {
      const name = this.current();
      if (name.kind !== "identifier") return this.fail(name.offset, "Expected member name after .");
      this.advance();
      let args: Ast[] | null = null;
      if (this.matchPunct("(")) {
        args = this.parseArguments(depth + 1);
        if (!args) return null;
      }
      expr = { kind: "member", object: expr, name: name.text, args };
    }
    return expr;
  }

  private parseArguments(depth: number): Ast[] | null {
    const args: Ast[] = [];
    if (this.matchPunct(")")) return args;

    while (true) {
      const arg = this.parseExpression(0, depth + 1);
      if (!arg) return null;
      args.push(arg);
      if (this.matchPunct(")")) return args;
      if (!this.matchPunct(",")) return this.fail(this.current().offset, "Expected , or )");
    }
  }
}

export function parseFormula(src: string): ParseResult {
  const lexed = lexFormula(src);
  if (!lexed.ok) return { ok: false, error: lexed.error };
  return new Parser(lexed.tokens).parse();
}
