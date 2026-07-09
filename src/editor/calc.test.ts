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
    expect(out("floor(3.9)")).toEqual(["3"]);
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
