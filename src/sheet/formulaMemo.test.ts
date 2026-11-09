import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import * as formula from "./formula";
import { bumpDataRev, bumpGraphEpoch } from "../graphSession";
import { dataRev } from "../dataRevision";
import { applyChangeSummary } from "../changeSummary";
import { resetStore } from "../document";
import { setDoc, type FeedPage, type Node } from "../document/model";
import { initParser } from "../render/parse";
import { computeFormulaResults, createFormulaResultsMemo, createFormulaFilterMemo, filterFormulaRows, formulaResultKey } from "./formulaEval";
import { localDayKey, resyncCurrentDayForTest, setCurrentDayKeyForTest } from "../journal";

beforeAll(initParser);
afterEach(resetStore);

it("formula cells and filters ignore unrelated saves and refresh their row page, full and gaps", () => {
  bumpGraphEpoch();
  let rev = 0;
  const publish = (pages: string[] = [], full = false) => {
    applyChangeSummary({ rev: String(++rev), pages, blocks: [], linksTo: [], props: {}, inventoryChanged: false, full });
    bumpDataRev(true);
  };
  publish();
  const rows = fixture(1);
  createRoot(dispose => {
    let evaluations = 0;
    const formulas = new Map([["base", "score + 1"]]);
    const results = createFormulaResultsMemo({ rows: () => rows, formulas: () => formulas, now: () => new Date(0), onEvaluate: () => evaluations++ });
    const evaluator = vi.spyOn(formula, "evaluate");
    const filtered = createFormulaFilterMemo({ rows: () => rows, formulas: () => formulas, now: () => new Date(0), filter: () => "score >= 0" });
    try {
      results(); filtered(); evaluations = 0; evaluator.mockClear();
      publish(["other"]); results(); filtered();
      expect(evaluations).toBe(0); expect(evaluator).not.toHaveBeenCalled();
      for (const reason of ["related", "full", "gap"]) {
        if (reason === "gap") rev += 2;
        publish(reason === "related" ? ["sheet"] : [], reason === "full");
        results(); filtered();
        expect(evaluations).toBe(1); expect(evaluator).toHaveBeenCalledTimes(2);
        evaluations = 0; evaluator.mockClear();
      }
    } finally { evaluator.mockRestore(); dispose(); }
  });
});

function fixture(count: number) {
  const nodes: Node[] = Array.from({ length: count }, (_, i) => ({
    id: `r${i}`, raw: `Row ${i}\nscore:: ${i}`, page: "Sheet", parent: null, children: [], collapsed: false,
  }));
  const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots: nodes.map(n => n.id), format: "md", readOnly: false, guide: false };
  setDoc({ byId: Object.fromEntries(nodes.map(n => [n.id, n])), pages: [page], feed: [page.name], loaded: true });
  return nodes.map(n => ({ id: n.id, page: n.page }));
}

it("one live row edit evaluates F cells, not N × F", () => {
  const rows = fixture(1000);
  createRoot(dispose => {
    let evaluations = 0;
    const formulas = new Map(Array.from({ length: 10 }, (_, i) => [`f${i}`, `score + ${i}`]));
    const results = createFormulaResultsMemo({ rows: () => rows, formulas: () => formulas,
      now: () => new Date(0), onEvaluate: () => evaluations++, warnThreshold: Infinity });
    expect(results().size).toBe(10000);
    evaluations = 0;
    const revision = dataRev();
    setDoc("byId", "r17", "raw", "Edited\nscore:: 100");
    expect(results()).toEqual(computeFormulaResults(rows, formulas, new Date(0)).results);
    expect(evaluations).toBe(10);
    expect(dataRev()).toBe(revision);
    dispose();
  });
});

// Card CBEnA: past midnight only the cells whose evaluation read the date are
// computed again (a row whose `if` never reached today() is not), with no
// injected clock, as in the app.
it("a day change recomputes exactly the formula cells that read the date", () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2030, 6, 15, 23, 0));
  setCurrentDayKeyForTest(localDayKey());
  try {
    const nodes = ["r0", "r1"].map((id, i) => ({ id, raw: `Row\nscore:: ${i ? 9 : 1}`, page: "Sheet", parent: null, children: [], collapsed: false }));
    const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["r0", "r1"], format: "md", readOnly: false, guide: false };
    setDoc({ byId: Object.fromEntries(nodes.map(n => [n.id, n])), pages: [page], feed: [page.name], loaded: true });
    const rows = nodes.map(n => ({ id: n.id, page: n.page }));
    createRoot(dispose => {
      const evaluated: string[] = [];
      const formulas = new Map([["when", 'if(score > 5, today().format("YYYY-MM-DD"), "later")']]);
      const results = createFormulaResultsMemo({ rows: () => rows, formulas: () => formulas, onEvaluate: () => evaluated.push("cell") });
      const text = (id: string) => { const v = results.get(formulaResultKey(id, "when")); return v?.kind === "text" ? v.value : undefined; };
      try {
        results();
        expect([text("r0"), text("r1")]).toEqual(["later", new Date(2030, 6, 15, 23, 0).toISOString().slice(0, 10)]);
        evaluated.length = 0;
        vi.setSystemTime(new Date(2030, 6, 16, 23, 30)); // 24.5 hours on: another local and UTC day
        resyncCurrentDayForTest();
        expect([text("r0"), text("r1")]).toEqual(["later", new Date(2030, 6, 16, 23, 30).toISOString().slice(0, 10)]);
        expect(evaluated).toHaveLength(1);
      } finally { dispose(); }
    });
  } finally { vi.useRealTimers(); setCurrentDayKeyForTest(localDayKey()); }
});

it.skipIf(!process.env.L10_BENCH)("measures the audit matrix", () => {
  for (const n of [100, 1000, 3000, 5000]) for (const f of [0, 1, 5, 10]) {
    const rows = fixture(n);
    createRoot(dispose => {
      const formulas = new Map(Array.from({ length: f }, (_, i) => [`f${i}`, `score + ${i}`]));
      const now = new Date(0);
      let evaluations = 0;
      const results = createFormulaResultsMemo({ rows: () => rows, formulas: () => formulas,
        now: () => now, onEvaluate: () => evaluations++, warnThreshold: Infinity });
      results();
      const times: number[] = [];
      for (let i = 0; i < 9; i++) {
        evaluations = 0;
        const start = performance.now();
        setDoc("byId", "r17", "raw", `Edited\nscore:: ${100 + i}`);
        results();
        times.push(performance.now() - start);
      }
      console.log(JSON.stringify({ n, f, evaluations, medianMs: times.sort((a, b) => a - b)[4] }));
      dispose();
    });
  }
});

it("matches full evaluation through fields, references, time, definitions, membership and filters", () => {
  const initial = fixture(4);
  createRoot(dispose => {
    const [rows, setRows] = createSignal(initial);
    const [formulas, setFormulas] = createSignal(new Map([
      ["base", "score + 1"], ["ref", "formula.base * 2"], ["day", "today()"],
      ["clock", "now()"], ["relative", "today().relative()"],
    ]));
    const [now, setNow] = createSignal(new Date("2026-10-22T12:00:00Z"));
    const [filter, setFilter] = createSignal<string | null>("formula.base > 2");
    const results = createFormulaResultsMemo({ rows, formulas, now });
    const filtered = createFormulaFilterMemo({ rows, formulas, now, filter });
    const check = () => {
      expect(results()).toEqual(computeFormulaResults(rows(), formulas(), now()).results);
      expect(filtered()).toEqual(filterFormulaRows(rows(), filter(), formulas(), now()));
    };
    check();
    setDoc("byId", "r1", "raw", "Edited\nscore:: 9"); check();
    setNow(new Date("2026-10-23T15:30:00Z")); check();
    setFormulas(new Map([...formulas(), ["base", "score + 20"]])); check();
    setRows([...initial].reverse()); check();
    setRows(initial.slice(1)); check();
    setRows(initial); check();
    setFilter("today() < now()"); check();
    setFilter("score + 1"); check();
    setFilter("("); check();
    setFilter(null); check();
    setFormulas(new Map()); check();
    dispose();
  });
});

it("reuses filter answers and retires removed or disposed row computations", () => {
  const initial = fixture(40);
  const evaluator = vi.spyOn(formula, "evaluate");
  try {
    createRoot(dispose => {
      const [rows, setRows] = createSignal(initial);
      const formulas = new Map([["base", "score + 1"]]);
      const [filter, setFilter] = createSignal<string | null>("formula.base > 2");
      const now = new Date(0);
      const filtered = createFormulaFilterMemo({ rows, formulas: () => formulas, now: () => now, filter });
      filtered(); evaluator.mockClear();
      setDoc("byId", "r17", "raw", "Row\nscore:: 100");
      filtered(); expect(evaluator).toHaveBeenCalledTimes(1);
      bumpDataRev(); filtered();
      expect(evaluator).toHaveBeenCalledTimes(41);
      setRows(initial.slice(1)); filtered(); evaluator.mockClear();
      setDoc("byId", "r0", "raw", "Removed\nscore:: 100");
      filtered(); expect(evaluator).not.toHaveBeenCalled();
      setFilter(null); filtered(); evaluator.mockClear();
      setDoc("byId", "r1", "raw", "Unfiltered\nscore:: 100");
      filtered(); expect(evaluator).not.toHaveBeenCalled();
      setFilter("score > 0"); filtered(); evaluator.mockClear();
      dispose();
      setDoc("byId", "r1", "raw", "Disposed\nscore:: 101");
      expect(evaluator).not.toHaveBeenCalled();
    });
  } finally { evaluator.mockRestore(); }
});

it("does not inspect rows with no formulas and refreshes replaced DTO inputs", () => {
  createRoot(dispose => {
    const [formulas, setFormulas] = createSignal(new Map<string, string>());
    const [rows, setRows] = createSignal([{ id: "dto", page: "P", detached: true,
      dto: { id: "dto", raw: "Row", children: [], collapsed: false, properties: [["score", "3"] as [string, string]] } }]);
    let reads = 0;
    const now = new Date(0);
    const results = createFormulaResultsMemo({ rows: () => { reads++; return rows(); }, formulas, now: () => now });
    expect(results().size).toBe(0); expect(reads).toBe(0);
    setFormulas(new Map([["value", "score * 2"]]));
    expect([...results().values()]).toEqual([{ kind: "number", value: 6 }]);
    setRows([{ ...rows()[0], dto: { ...rows()[0].dto, properties: [["score", "7"]] } }]);
    expect(results()).toEqual(computeFormulaResults(rows(), formulas(), now).results);
    expect([...results().values()]).toEqual([{ kind: "number", value: 14 }]);
    dispose();
  });
});
