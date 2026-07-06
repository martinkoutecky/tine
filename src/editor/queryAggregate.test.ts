import { describe, it, expect } from "vitest";
import { foldAggregate, groupRows, type AggRow } from "./queryAggregate";

const rows: AggRow[] = [
  { page: "A", props: { hours: "2", status: "open" } },
  { page: "A", props: { hours: "3.5", status: "open" } },
  { page: "B", props: { hours: "not a number", status: "done" } },
  { page: "B", props: { status: "done" } }, // no hours at all
];

describe("foldAggregate", () => {
  it("count ignores properties and counts rows", () => {
    expect(foldAggregate(rows, null)).toEqual({ text: "4", skipped: 0 });
    expect(foldAggregate(rows, { agg: "count", field: null })).toEqual({ text: "4", skipped: 0 });
  });

  it("sum adds numeric values and skips non-numeric / absent", () => {
    // 2 + 3.5 = 5.5; the "not a number" and the missing-hours rows are skipped.
    expect(foldAggregate(rows, { agg: "sum", field: "hours" })).toEqual({ text: "5.5", skipped: 2 });
  });

  it("avg divides by the count of numeric contributors, not the row count", () => {
    // (2 + 3.5) / 2 = 2.75, NOT / 4.
    expect(foldAggregate(rows, { agg: "avg", field: "hours" })).toEqual({ text: "2.75", skipped: 2 });
  });

  it("avg of an all-non-numeric set is 0 with everything skipped (no NaN)", () => {
    const r: AggRow[] = [{ page: "X", props: { hours: "x" } }];
    expect(foldAggregate(r, { agg: "avg", field: "hours" })).toEqual({ text: "0", skipped: 1 });
  });

  it("parseFloat is lenient: a trailing unit still contributes its leading number", () => {
    const r: AggRow[] = [{ page: "X", props: { hours: "3 hrs" } }];
    expect(foldAggregate(r, { agg: "sum", field: "hours" })).toEqual({ text: "3", skipped: 0 });
  });

  it("rounds float noise to 3 decimals", () => {
    const r: AggRow[] = [
      { page: "X", props: { n: "0.1" } },
      { page: "X", props: { n: "0.2" } },
    ];
    expect(foldAggregate(r, { agg: "sum", field: "n" })).toEqual({ text: "0.3", skipped: 0 });
  });
});

describe("groupRows", () => {
  it("groups by page, preserving first-seen order", () => {
    const g = groupRows(rows, "page");
    expect([...g.keys()]).toEqual(["A", "B"]);
    expect(g.get("A")!.length).toBe(2);
    expect(g.get("B")!.length).toBe(2);
  });

  it("groups by a property, bucketing absent values under (none)", () => {
    const g = groupRows(rows, "status");
    expect([...g.keys()]).toEqual(["open", "done"]);
    expect(g.get("open")!.length).toBe(2);
    // A per-group count is exactly what the summary table renders.
    const perGroup = [...g.entries()].map(([k, set]) => [k, foldAggregate(set, null).text]);
    expect(perGroup).toEqual([
      ["open", "2"],
      ["done", "2"],
    ]);
  });

  it("missing property value falls into (none)", () => {
    const r: AggRow[] = [{ page: "X", props: {} }];
    const g = groupRows(r, "status");
    expect([...g.keys()]).toEqual(["(none)"]);
  });
});
