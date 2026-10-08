import { describe, expect, it } from "vitest";
import { aggregate } from "./aggregate";
import { parseIsoDateLike } from "./typed";
import { evaluate } from "./formula/eval";
import { parseFormula } from "./formula/parser";
import { nullValue, parseDateValue } from "./formula/value";

describe("D18 sheet calendar and scalar policies", () => {
  it("rejects impossible dates in the aggregate footer as in typed cells", () => {
    expect(parseIsoDateLike("2026-02-31")).toBeNull();
    expect(aggregate("earliest", ["2026-02-31", "<2026-03-01 Sun>"])).toBe("2026-03-01 (1 skipped)");
    expect(aggregate("latest", ["1900-02-29", "2000-02-29"])).toBe("2000-02-29 (1 skipped)");
  });

  it("preserves explicit full years through typed cells and formula arithmetic", () => {
    expect(parseIsoDateLike("0099-01-01")).toEqual({ y: 99, m: 0, d: 1, time: null });
    const expression = parseFormula('d.format("YYYY-MM-DD")');
    expect(expression.ok).toBe(true);
    if (!expression.ok) return;
    expect(evaluate(expression.ast, {
      field: () => parseDateValue("0099-01-01") ?? nullValue(),
      formulaAst: () => null,
      now: new Date(0),
    })).toEqual({ kind: "text", value: "0099-01-01" });
    const nextYear = parseFormula('d + "1y"');
    expect(nextYear.ok).toBe(true);
    if (!nextYear.ok) return;
    expect(evaluate(nextYear.ast, {
      field: () => parseDateValue("0000-02-29") ?? nullValue(),
      formulaAst: () => null,
      now: new Date(0),
    })).toEqual({ kind: "date", value: { y: 1, m: 1, d: 28, time: null }, source: "0001-02-28" });
  });

  it("keeps stable's numeric-prefix aggregate policy distinct from decimal cells", () => {
    expect(aggregate("sum", ["123abc", "1e2", "0x10", "2026-02-31"])).toBe("2249");
    expect(aggregate("sum", ["2", "3.5h", "bad"])).toBe("5.5 (1 skipped)");
  });
});
