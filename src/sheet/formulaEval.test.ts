import { createMemo, createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import { createFormulaResultsMemo, fieldValueToFormulaValue, formulaResultKey, formulaValueText } from "./formulaEval";
import {
  booleanValue,
  nullValue,
  numberValue,
  parseDateValue,
  textValue,
  type FormulaValue,
} from "./formula/value";

function dateValue(source: string): FormulaValue {
  const parsed = parseDateValue(source);
  if (!parsed) throw new Error(`Bad date fixture ${source}`);
  return parsed;
}

describe("formula eval context", () => {
  it("converts field values to FormulaValue using the documented table", () => {
    expect(fieldValueToFormulaValue("prop:missing", null)).toEqual(nullValue());
    expect(fieldValueToFormulaValue("prop:qty", { text: "12.5", raw: "12.5" })).toEqual(numberValue(12.5));
    expect(fieldValueToFormulaValue("deadline", { text: "2026-07-08", raw: "2026-07-08" })).toEqual(
      dateValue("2026-07-08")
    );
    expect(fieldValueToFormulaValue("prop:done", { text: "TRUE", raw: "TRUE" })).toEqual(booleanValue(true));
    expect(fieldValueToFormulaValue("tags", { text: "#alpha #beta", raw: "alpha beta" })).toEqual({
      kind: "list",
      values: [textValue("alpha"), textValue("beta")],
    });
    expect(fieldValueToFormulaValue("prop:estimate", { text: "2h", raw: "2h" })).toEqual(textValue("2h"));
    // planning facets carry OG's day-name tail — must still coerce to date
    // (orchestrator fix: they degraded to text, breaking `deadline < today()`)
    expect(fieldValueToFormulaValue("scheduled", { text: "2026-07-08 Wed", raw: "2026-07-08 Wed" })).toEqual(
      dateValue("2026-07-08")
    );
    expect(fieldValueToFormulaValue("deadline", { text: "2026-07-10 Fri .+1w", raw: "2026-07-10 Fri .+1w" })).toEqual(
      dateValue("2026-07-10")
    );
  });

  it("memoizes formula evaluation across unrelated signal reads", () => {
    createRoot((dispose) => {
      const [unrelated, setUnrelated] = createSignal(0);
      let evaluations = 0;
      const results = createFormulaResultsMemo({
        rows: () => [{ id: "r1", page: "Sheet" }],
        formulas: () => new Map([["total", "1 + 1"]]),
        now: () => new Date(Date.UTC(2026, 0, 1)),
        onEvaluate: () => {
          evaluations += 1;
        },
      });
      const view = createMemo(() => {
        unrelated();
        return formulaValueText(results().get(formulaResultKey("r1", "total")));
      });

      expect(view()).toBe("2");
      expect(evaluations).toBe(1);
      setUnrelated(1);
      expect(view()).toBe("2");
      expect(evaluations).toBe(1);
      dispose();
    });
  });
});
