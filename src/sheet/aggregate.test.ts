import { describe, expect, it } from "vitest";
import { aggregate } from "./aggregate";

describe("sheet aggregate", () => {
  it("computes numeric summaries and counts skipped values", () => {
    const values = ["2", "3.5h", "bad", ""];

    expect(aggregate("sum", values)).toBe("5.5 (2 skipped)");
    expect(aggregate("average", values)).toBe("2.75 (2 skipped)");
    expect(aggregate("median", values)).toBe("2.75 (2 skipped)");
    expect(aggregate("min", values)).toBe("2 (2 skipped)");
    expect(aggregate("max", values)).toBe("3.5 (2 skipped)");
    expect(aggregate("range", values)).toBe("1.5 (2 skipped)");
    expect(aggregate("stddev", values)).toBe("0.75 (2 skipped)");
  });

  it("computes date summaries from YYYY-MM-DD prefixes", () => {
    const values = ["<2026-07-09 Thu>", "2026-07-07", "not a date"];

    expect(aggregate("earliest", values)).toBe("2026-07-07 (1 skipped)");
    expect(aggregate("latest", values)).toBe("2026-07-09 (1 skipped)");
    expect(aggregate("range", values)).toBe("2026-07-07 - 2026-07-09 (1 skipped)");
  });

  it("computes any-value summaries", () => {
    const values = ["A", "", "A", "B", " "];

    expect(aggregate("empty", values)).toBe("2");
    expect(aggregate("filled", values)).toBe("3");
    expect(aggregate("count", values)).toBe("3");
    expect(aggregate("unique", values)).toBe("2");
  });

  it("computes checkbox-like summaries including task states", () => {
    const values = ["DONE", "TODO", "DOING", "", "true"];

    expect(aggregate("checked", values)).toBe("2");
    expect(aggregate("unchecked", values)).toBe("3");
  });

  it("handles empty columns without NaN", () => {
    expect(aggregate("sum", ["", ""])).toBe("0 (2 skipped)");
    expect(aggregate("average", [])).toBe("0");
    expect(aggregate("earliest", [""])).toBe("(1 skipped)");
    expect(aggregate("unique", [])).toBe("0");
  });
});
