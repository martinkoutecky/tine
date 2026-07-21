import { describe, it, expect, beforeAll } from "vitest";
import { initParser } from "../render/parse";
import { calcSource, evalCalc, serializeCalcExitCommit, wrapCalc } from "./calc";
import { normalizePlanning } from "./planning";

beforeAll(async () => {
  await initParser();
});

// Just the output column, for terse assertions.
const out = (src: string) => evalCalc(src).map((l) => l.output);

describe("calc evaluator (Logseq parity)", () => {
  it("basic arithmetic + precedence", () => {
    expect(out("1 + 2 * 3")).toEqual(["7"]);
    expect(out("(1 + 2) * 3")).toEqual(["9"]);
    expect(out("2 ^ 10")).toEqual(["1024"]);
    expect(out("7 mod 3")).toEqual(["1"]);
    expect(out("-5 + 2")).toEqual(["-3"]);
  });

  it("percent is ÷100 (OG semantics) — NOT 'percent of'", () => {
    expect(out("10%")).toEqual(["0.1"]);
    expect(out("100 + 10%")).toEqual(["100.1"]); // NOT 110
    expect(out("50% * 1000")).toEqual(["500"]);
  });

  it("variables, reassignment, and `last`", () => {
    expect(out("a = 2\nb = a + 3\nb * 2")).toEqual(["2", "5", "10"]);
    expect(out("6 * 7\nlast * 2")).toEqual(["42", "84"]);
  });

  it("functions and constants", () => {
    expect(out("sqrt(16)")).toEqual(["4"]);
    expect(out("abs(-3)")).toEqual(["3"]);
    // floor/ceil/round are a deliberate Tine superset of OG's calc.bnf.
    expect(out("floor(3.9)")).toEqual(["3"]);
    expect(out("ceil(3.1)")).toEqual(["4"]);
    expect(out("round(3.5)")).toEqual(["4"]);
  });

  it("commas stripped; comments and blank lines yield no output", () => {
    expect(out("1,000 + 1")).toEqual(["1001"]);
    expect(out("2 * 5 # double")).toEqual(["10"]);
    expect(out("")).toEqual([null]);
    expect(out("# just a comment")).toEqual([null]);
  });

  it("unparseable line is flagged, no output, doesn't abort later lines", () => {
    const r = evalCalc("1 +\n3 + 4");
    expect(r[0].output).toBeNull();
    expect(r[0].error).toBe(true);
    expect(r[1].output).toBe("7");
  });

  it("accepts hexadecimal, octal, and binary literals and renders the selected base", () => {
    expect(out("0x1F + 1\n0o10 + 0b11")).toEqual(["32", "11"]);
    expect(out(":hex\n0x1F + 1\n:bin\n10 + 1\n:oct\n8\n:decimal"))
      .toEqual([null, "0x20", "0b100000", "0b1011", "0o13", "0o10", "8"]);
  });

  it("accepts scientific and mixed-number literals", () => {
    expect(out("1.25e2 + .5\n3 1/2 + 0.5")).toEqual(["125.5", "4"]);
  });

  it("implements inverse trig and OG's factorial domain and precedence", () => {
    expect(out("asin(1)\nacos(1)\natan(1)\n5!\n0!\n-3!"))
      .toEqual(["1.5707963267948966", "0", "0.7853981633974483", "120", "1", "-6"]);
  });

  it("keeps stateful format directives and renders their current last value", () => {
    expect(out("12\n:format fixed 2\n1200\n:format sci 2\n1.5\n:format fractions\n:format improper\n:format normal"))
      .toEqual(["12", "12.00", "1200.00", "1.20e+3", "1.50e+0", "1 1/2", "3/2", "1.5"]);
  });

  it("uses decimal arithmetic instead of JavaScript floating-point rounding", () => {
    expect(out("0.1 + 0.2\n0.123456789012345678 + 0.000000000000000001"))
      .toEqual(["0.3", "0.123456789012345679"]);
  });

  it("passes parse failures through as errors, including when later lines read last", () => {
    const result = evalCalc("2\n1 +\nlast\n3 + 4");
    expect(result.map((line) => line.output)).toEqual(["2", null, null, "7"]);
    expect(result.map((line) => line.error ?? false)).toEqual([false, true, true, false]);
  });

  it("retains OG's right-associative exponentiation while extending the language", () => {
    expect(out("2 ^ 3 ^ 2")).toEqual(["512"]);
  });
});

describe("calcSource (extract ```calc fence for the live editor preview)", () => {
  it("returns the inner source of a complete fence", () => {
    expect(calcSource("```calc\n1+1\n2*3\n```")).toBe("1+1\n2*3");
  });
  it("tolerates a missing closing fence (mid-edit)", () => {
    expect(calcSource("```calc\n1+1")).toBe("1+1");
  });
  it("is null for non-calc text or other code fences", () => {
    expect(calcSource("just text")).toBeNull();
    expect(calcSource("```js\n1+1\n```")).toBeNull();
  });
});

describe("wrapCalc / round-trip", () => {
  it("wrapCalc is the inverse of calcSource (editor edits the inner, re-fences on save)", () => {
    const raw = "```calc\n1 + 2\n2+4\nx = 12 * 3\nx / 4\n```";
    const inner = calcSource(raw);
    expect(inner).toBe("1 + 2\n2+4\nx = 12 * 3\nx / 4");
    expect(wrapCalc(inner!)).toBe(raw);
  });
  it("re-fences edited expressions (incl. a newly added line)", () => {
    expect(wrapCalc("1 + 2\n100 + 10%")).toBe("```calc\n1 + 2\n100 + 10%\n```");
  });
});

describe("calc exit commit serialization", () => {
  it("re-fences the bare edit buffer and skips planning normalization", () => {
    const edited = "1 + 1\n2 + 2\nSCHEDULED: <2026-07-06 Mon>";

    expect(normalizePlanning(edited, "md")).toBe("1 + 1\nSCHEDULED: <2026-07-06 Mon>\n2 + 2");

    const committed = serializeCalcExitCommit(edited);
    expect(committed).toBe("```calc\n1 + 1\n2 + 2\nSCHEDULED: <2026-07-06 Mon>\n```");
    expect(calcSource(committed)).toBe(edited);
    expect(evalCalc(calcSource(committed)!).map((line) => line.input)).toEqual(edited.split("\n"));
  });

  it("does not double-fence an already fenced calc value", () => {
    const raw = "```calc\n1 + 2\n```";
    expect(serializeCalcExitCommit(raw)).toBe(raw);
  });
});
