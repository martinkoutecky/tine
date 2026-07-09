import { describe, expect, it } from "vitest";
import { lexFormula } from "./lexer";

function lexOk(src: string) {
  const result = lexFormula(src);
  expect(result.ok).toBe(true);
  if (!result.ok) throw new Error(result.error.message);
  return result.tokens;
}

describe("formula lexer", () => {
  it("lexes every token family with offsets", () => {
    const tokens = lexOk("price + formula.total >= 10 && name.contains('x')");
    expect(tokens.map((t) => t.kind === "operator" ? t.op : t.kind === "punct" ? t.punct : t.kind)).toEqual([
      "identifier",
      "+",
      "identifier",
      ".",
      "identifier",
      ">=",
      "number",
      "&&",
      "identifier",
      ".",
      "identifier",
      "(",
      "string",
      ")",
      "eof",
    ]);
    expect(tokens[0]).toMatchObject({ kind: "identifier", text: "price", offset: 0, end: 5 });
    expect(tokens[6]).toMatchObject({ kind: "number", raw: "10", value: 10, offset: 25 });
  });

  it("unescapes supported string escapes and keeps emoji inside strings", () => {
    const tokens = lexOk(String.raw`'it\'s \#ok \\ fine 😀' "say \"hi\""`);
    expect(tokens[0]).toMatchObject({ kind: "string", value: "it's #ok \\ fine 😀", offset: 0 });
    expect(tokens[1]).toMatchObject({ kind: "string", value: 'say "hi"' });
  });

  it("reports adversarial inputs at useful offsets", () => {
    expect(lexFormula("1..2")).toEqual({ ok: false, error: { offset: 1, message: "Invalid decimal number" } });
    expect(lexFormula("'unterminated")).toEqual({ ok: false, error: { offset: 0, message: "Unterminated string literal" } });
    expect(lexFormula("12abc")).toEqual({ ok: false, error: { offset: 2, message: "Invalid number suffix" } });
    expect(lexFormula("🙂")).toEqual({ ok: false, error: { offset: 0, message: "Unexpected character \ud83d" } });
  });

  it("leaves malformed member chains to the parser", () => {
    expect(lexOk("a..b()").map((t) => t.kind === "punct" ? t.punct : t.kind)).toEqual([
      "identifier",
      ".",
      ".",
      "identifier",
      "(",
      ")",
      "eof",
    ]);
  });
});
