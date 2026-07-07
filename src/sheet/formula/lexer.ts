import { isPlainDecimalNumber } from "../typed";

export type Token =
  | { kind: "number"; raw: string; value: number; offset: number; end: number }
  | { kind: "string"; raw: string; value: string; quote: "'" | "\""; offset: number; end: number }
  | { kind: "identifier"; text: string; offset: number; end: number }
  | { kind: "operator"; op: OperatorToken; offset: number; end: number }
  | { kind: "punct"; punct: "(" | ")" | "." | ","; offset: number; end: number }
  | { kind: "eof"; offset: number; end: number };

export type OperatorToken =
  | "+"
  | "-"
  | "*"
  | "/"
  | "%"
  | "<"
  | "<="
  | ">"
  | ">="
  | "=="
  | "!="
  | "&&"
  | "||"
  | "!";

export interface FormulaParseError {
  offset: number;
  message: string;
}

export type LexResult = { ok: true; tokens: Token[] } | { ok: false; error: FormulaParseError };

function identifierStart(ch: string): boolean {
  return /[A-Za-z_]/.test(ch);
}

function identifierPart(ch: string): boolean {
  return /[A-Za-z0-9_-]/.test(ch);
}

function isDigit(ch: string): boolean {
  return ch >= "0" && ch <= "9";
}

function lexString(src: string, offset: number): { token: Token; next: number } | { error: FormulaParseError } {
  const quote = src[offset] as "'" | "\"";
  let i = offset + 1;
  let value = "";

  while (i < src.length) {
    const ch = src[i];
    if (ch === quote) {
      return {
        token: { kind: "string", raw: src.slice(offset, i + 1), value, quote, offset, end: i + 1 },
        next: i + 1,
      };
    }
    if (ch === "\\") {
      const escaped = src[i + 1];
      if (escaped == null) return { error: { offset: i, message: "Unterminated string escape" } };
      if (escaped === "'" || escaped === "\"" || escaped === "\\" || escaped === "#") {
        value += escaped;
        i += 2;
        continue;
      }
      return { error: { offset: i, message: `Unsupported string escape \\${escaped}` } };
    }
    value += ch;
    i += 1;
  }

  return { error: { offset, message: "Unterminated string literal" } };
}

function lexNumber(src: string, offset: number): { token: Token; next: number } | { error: FormulaParseError } {
  let i = offset;
  while (isDigit(src[i] ?? "")) i += 1;

  if (src[i] === ".") {
    if (!isDigit(src[i + 1] ?? "")) return { error: { offset: i, message: "Invalid decimal number" } };
    i += 1;
    while (isDigit(src[i] ?? "")) i += 1;
  }

  if (identifierStart(src[i] ?? "")) return { error: { offset: i, message: "Invalid number suffix" } };

  const raw = src.slice(offset, i);
  if (!isPlainDecimalNumber(raw)) return { error: { offset, message: "Invalid decimal number" } };
  return { token: { kind: "number", raw, value: Number(raw), offset, end: i }, next: i };
}

export function lexFormula(src: string): LexResult {
  const tokens: Token[] = [];
  let i = 0;

  while (i < src.length) {
    const ch = src[i];
    if (ch === " " || ch === "\t" || ch === "\n" || ch === "\r") {
      i += 1;
      continue;
    }

    if (isDigit(ch)) {
      const result = lexNumber(src, i);
      if ("error" in result) return { ok: false, error: result.error };
      tokens.push(result.token);
      i = result.next;
      continue;
    }

    if (ch === "'" || ch === "\"") {
      const result = lexString(src, i);
      if ("error" in result) return { ok: false, error: result.error };
      tokens.push(result.token);
      i = result.next;
      continue;
    }

    if (identifierStart(ch)) {
      const start = i;
      i += 1;
      while (identifierPart(src[i] ?? "")) i += 1;
      tokens.push({ kind: "identifier", text: src.slice(start, i), offset: start, end: i });
      continue;
    }

    const two = src.slice(i, i + 2);
    if (two === "<=" || two === ">=" || two === "==" || two === "!=" || two === "&&" || two === "||") {
      tokens.push({ kind: "operator", op: two, offset: i, end: i + 2 });
      i += 2;
      continue;
    }

    if (ch === "+" || ch === "-" || ch === "*" || ch === "/" || ch === "%" || ch === "<" || ch === ">" || ch === "!") {
      tokens.push({ kind: "operator", op: ch, offset: i, end: i + 1 });
      i += 1;
      continue;
    }

    if (ch === "(" || ch === ")" || ch === "." || ch === ",") {
      tokens.push({ kind: "punct", punct: ch, offset: i, end: i + 1 });
      i += 1;
      continue;
    }

    return { ok: false, error: { offset: i, message: `Unexpected character ${ch}` } };
  }

  tokens.push({ kind: "eof", offset: src.length, end: src.length });
  return { ok: true, tokens };
}
