import { createComputed, createMemo, createRoot, createSignal } from "solid-js";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cellView } from "./cellPresentation";
import { boardCardChips } from "./boardColumns";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { type FeedPage, type Node } from "../document/model";
import { setDoc } from "../document/model";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";
import {
  createFormulaResultsMemo,
  fieldValueToFormulaValue,
  formulaResultKey,
  formulaRowKey,
  formulaValueText,
  liveFormulaRowNode,
  readFormulaRowField,
} from "./formulaEval";
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

afterEach(() => {
  resetStore();
});

beforeAll(async () => {
  await initParser();
});

it.each([8, 32])("GH #886: editing one formula row copies only its own outputs, K=%i", K => {
  const rows = Array.from({ length: K }, (_, i) => ({ id: `r${i}`, page: "Sheet" }));
  const page: FeedPage = { name: "Sheet", title: "Sheet", kind: "page", preBlock: null,
    roots: rows.map(row => row.id), format: "md", readOnly: false, guide: false };
  setDoc({ byId: Object.fromEntries(rows.map(row => [row.id, { ...row, raw: "Row\nscore:: 1",
    collapsed: false, parent: null, children: [] }])), pages: [page], feed: [page.name], loaded: true });
  createRoot(dispose => {
    const formulas = new Map([["one", "score + 1"], ["two", "score + 2"]]);
    createFormulaResultsMemo({ rows: () => rows, formulas: () => formulas, now: () => new Date(0) });
    const original = Map.prototype.set;
    let outputWrites = 0;
    const spy = vi.spyOn(Map.prototype, "set").mockImplementation(function (this: Map<unknown, unknown>, key, value) {
      if (value && typeof value === "object" && value.kind === "number") outputWrites++;
      return original.call(this, key, value);
    });
    try {
      setDoc("byId", "r0", "raw", "Edited\nscore:: 7");
      expect(outputWrites).toBe(2);
    } finally { spy.mockRestore(); dispose(); }
  });
});

it.each([8, 32])("GH #886: keyed formula consumers follow only their row and retire removed rows, K=%i", K => {
  const initial = Array.from({ length: K }, (_, i) => ({ id: `dto${i}`, page: "P", detached: true,
    dto: { id: `dto${i}`, raw: "Row", collapsed: false, children: [], properties: [["score", "1"] as [string, string]] } }));
  createRoot(dispose => {
    const [rows, setRows] = createSignal(initial);
    const formulas = new Map([["score", "score + 1"]]);
    const values = createFormulaResultsMemo({ rows, formulas: () => formulas, now: () => new Date(0) });
    const seen: (FormulaValue | undefined)[] = [];
    let reads = 0;
    for (let i = 0; i < K; i++) createComputed(() => {
      seen[i] = values.get(formulaResultKey(initial[i], "score")); reads++;
    });
    // Membership changes can affect every reader, then each row's own field
    // remains fine-grained within the new answer.
    const [score, setScore] = createSignal("3");
    const changed = { ...initial[0], dto: { ...initial[0].dto, get properties(): [string, string][] { return [["score", score()]]; } } };
    setRows([changed, ...initial.slice(1)]);
    reads = 0;
    setScore("8");
    expect(reads).toBe(1);
    expect(seen[0]).toEqual(numberValue(9));
    expect(seen.slice(1)).toEqual(Array(K - 1).fill(numberValue(2)));
    setRows(initial.slice(1));
    expect(values.get(formulaResultKey(initial[0], "score"))).toBeUndefined();
    expect(values()).toEqual(new Map(initial.slice(1).map(row => [formulaResultKey(row, "score"), numberValue(2)])));
    dispose();
  });
});

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
        now: () => new Date(2026, 0, 1),
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

  it("keeps same-UUID page and journal rows distinct and only reads the matching live node", async () => {
    const loaded: Node = {
      id: "same-id",
      raw: "Loaded page row\nscore:: 1",
      collapsed: false,
      parent: null,
      page: "Twin",
      children: [],
    };
    const page: FeedPage = {
      name: "Twin",
      kind: "page",
      title: "Twin",
      preBlock: null,
      roots: [loaded.id],
      format: "md",
      readOnly: false,
      guide: false,
    };
    setDoc({ byId: { [loaded.id]: loaded }, pages: [page], feed: [page.name], loaded: true });
    await Promise.resolve();

    const pageRow = {
      id: loaded.id,
      page: page.name,
      kind: "page" as const,
      dto: { id: loaded.id, raw: "Page DTO\nscore:: 7", collapsed: false, children: [], properties: [["score", "7"] as [string, string]] },
    };
    const journalRow = {
      id: loaded.id,
      page: page.name,
      kind: "journal" as const,
      dto: { id: loaded.id, raw: "Journal DTO\nscore:: 9", collapsed: false, children: [], properties: [["score", "9"] as [string, string]] },
    };
    adoptResults([pageRow, journalRow], captureResultsTicket());

    expect(formulaRowKey(pageRow)).not.toBe(formulaRowKey(journalRow));
    expect(liveFormulaRowNode(pageRow)?.raw).toBe(loaded.raw);
    expect(liveFormulaRowNode(journalRow)).toBeNull();
    expect(readFormulaRowField(pageRow, "prop:score")?.text).toBe("1");
    expect(readFormulaRowField(journalRow, "prop:score")?.text).toBe("9");
  });

  it("keeps a multi-word tag one member when tags become a formula list", () => {
    const row = {
      id: "t1",
      page: "P",
      kind: "page" as const,
      dto: { id: "t1", raw: "Task\ntags:: [[big idea]], plain", collapsed: false, children: [], tags: ["big idea", "plain"], properties: [["tags", "[[big idea]], plain"] as [string, string]] },
    };
    const value = fieldValueToFormulaValue("tags", readFormulaRowField(row, "tags"));
    expect(value).toEqual({ kind: "list", values: [textValue("big idea"), textValue("plain")] });
  });

  it("shows a multi-word tag as one chip in table cells and board cards", () => {
    const row = {
      id: "t2",
      page: "P",
      kind: "page" as const,
      dto: { id: "t2", raw: "Task", collapsed: false, children: [], tags: ["big idea", "plain"], properties: [] },
    };
    expect(cellView("tags", undefined, readFormulaRowField(row, "tags"))).toEqual({ k: "chips", values: ["#big idea", "#plain"] });
    expect(boardCardChips(row, "state").tags).toEqual(["#big idea", "#plain"]);
    expect(boardCardChips(row, "tags").tags).toEqual([]);
  });
});
