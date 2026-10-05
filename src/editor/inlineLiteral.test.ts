// C5 L13-S1: on-type typography never rewrites literal source. The parser (lsdoc) decides what is
// literal, so double-backtick code and Org `~code~` are protected as well as single-backtick code,
// and a span still being typed stays protected until it closes (I-4, I-12).
import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { typoTypeReplace } from "../render/typography";
import { rangeInLiteral } from "./inlineLiteral";

beforeAll(initParser);

/** Post-input state: `before` plus the typed char at `caret`, as Block.tsx onInput sees it. */
function typed(before: string, caret: number, ch: string, format: "md" | "org" = "md") {
  const value = before.slice(0, caret) + ch + before.slice(caret);
  return typoTypeReplace(value, caret + 1, ch, (from, to) => rangeInLiteral(value, format, from, to));
}

describe("on-type typography leaves literal source alone", () => {
  it("double-backtick inline code", () => {
    expect(typed("prefix ``a-``", 11, ">")).toBeNull();
  });
  it("Org ~code~ and =verbatim=", () => {
    expect(typed("prefix ~a-~", 10, ">", "org")).toBeNull();
    expect(typed("prefix =a-=", 10, ">", "org")).toBeNull();
  });
  it("the closing backtick of `a--` does not rewrite the code it closes", () => {
    expect(typed("prefix `a--", 11, "`")).toBeNull();
  });
  it("a span still being typed is protected until it closes", () => {
    expect(typed("`", 1, ">")).toBeNull(); // nothing to complete
    expect(typed("`a-", 3, ">")).toBeNull();
    expect(typed("``a-", 4, ">")).toBeNull();
    expect(typed("~a-", 3, ">", "org")).toBeNull();
  });
  it("code containers are literal too", () => {
    expect(typed("```js\na-\n```", 8, ">")).toBeNull();
  });
  it("prose around, before and after code still gets its glyph", () => {
    expect(typed("`x` a-", 6, ">")).toEqual({ value: "`x` a→", caret: 6 });
    expect(typed("a- `x`", 2, ">")).toEqual({ value: "a→ `x`", caret: 2 });
    expect(typed("x =", 3, ">", "org")).toEqual({ value: "x ⇒", caret: 3 });
    expect(typed("a-", 2, ">", "org")).toEqual({ value: "a→", caret: 2 });
  });
});
