// REG-FORMULA-LOCAL-DATE: `today()`, `now()` and `.relative()` in a sheet formula
// read the user's calendar day and wall clock, not the UTC date.
//
// The app clock (`appNow()`, src/journal.ts) is an instant whose LOCAL getters
// carry the user's wall clock; the evaluator used to read it with UTC getters, so
// in the hours between local and UTC midnight `today()` was yesterday (Prague,
// 00:00-01:00) or tomorrow (Los Angeles, from 16:00).
//
// Each case pins the process time zone and asserts what a user sees: the shown
// value of a formula cell in a live sheet (default app clock), whether a row's
// typed date equals `today()` (cell and `tine.filter`), and the cell of the
// static export. Every case first proves the zone really took effect and the
// local and UTC dates differ (the UTC control excepted), so a pin that silently
// failed cannot pass.
import { createRoot } from "solid-js";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { resetStore } from "../document";
import { setDoc, type FeedPage, type Node } from "../document/model";
import { facetsOf } from "../render/facets";
import { initParser } from "../render/parse";
import { setWorkflow } from "../ui";
import type { BlockDto } from "../types";
import type { CellView } from "./cellPresentation";
import { createFormulaFilterMemo, createFormulaResultsMemo, formulaResultKey, formulaValueText } from "./formulaEval";
import { computeSheetExports } from "./staticExport";

interface ZoneCase {
  zone: string;
  /** The instant the app's clock reads. */
  at: string;
  /** `getTimezoneOffset()` of that instant in the zone. */
  offsetMinutes: number;
  /** The user's calendar day at `at`, the day before and the day after it. */
  yesterday: string;
  today: string;
  tomorrow: string;
  /** The user's wall-clock time at `at`. */
  time: string;
  /** The UTC date at `at`: differs from `today` everywhere but the control. */
  utcDate: string;
}

const CASES: ZoneCase[] = [
  { zone: "UTC", at: "2026-01-15T12:34:00Z", offsetMinutes: 0, yesterday: "2026-01-14", today: "2026-01-15", tomorrow: "2026-01-16", time: "12:34", utcDate: "2026-01-15" },
  // 19:05 on the 15th at UTC-8: the UTC date is already the 16th.
  { zone: "America/Los_Angeles", at: "2026-01-16T03:05:00Z", offsetMinutes: 480, yesterday: "2026-01-14", today: "2026-01-15", tomorrow: "2026-01-16", time: "19:05", utcDate: "2026-01-16" },
  // Daylight time (UTC-7), across a month end.
  { zone: "America/Los_Angeles", at: "2026-08-01T02:05:00Z", offsetMinutes: 420, yesterday: "2026-07-30", today: "2026-07-31", tomorrow: "2026-08-01", time: "19:05", utcDate: "2026-08-01" },
  // 05:10 on the 15th at UTC+9: the UTC date is still the 14th.
  { zone: "Asia/Tokyo", at: "2026-01-14T20:10:00Z", offsetMinutes: -540, yesterday: "2026-01-14", today: "2026-01-15", tomorrow: "2026-01-16", time: "05:10", utcDate: "2026-01-14" },
  // 00:30 on the 15th at UTC+1.
  { zone: "Europe/Prague", at: "2026-01-14T23:30:00Z", offsetMinutes: -60, yesterday: "2026-01-14", today: "2026-01-15", tomorrow: "2026-01-16", time: "00:30", utcDate: "2026-01-14" },
];

const FORMULAS = new Map([
  ["day", "today()"],
  ["clock", "now()"],
  ["next", 'today() + "1d"'],
  ["istoday", "due == today()"],
  ["rel", "due.relative()"],
]);

const originalTz = process.env.TZ;

beforeAll(async () => {
  await initParser();
});
afterEach(() => {
  vi.useRealTimers();
  resetStore();
});
afterAll(() => {
  if (originalTz === undefined) delete process.env.TZ;
  else process.env.TZ = originalTz;
});

/** Three rows, due yesterday, today and tomorrow, as live nodes of one sheet page. */
function liveRows(c: ZoneCase) {
  const dues = [c.yesterday, c.today, c.tomorrow];
  const nodes: Node[] = dues.map((due, i) => ({ id: `r${i}`, raw: `Row ${i}\ndue:: ${due}`, page: "Sheet", parent: null, children: [], collapsed: false }));
  const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: nodes.map((n) => n.id), format: "md", readOnly: false, guide: false };
  setDoc({ byId: Object.fromEntries(nodes.map((n) => [n.id, n])), pages: [page], feed: [page.name], loaded: true });
  return nodes.map((n) => ({ id: n.id, page: n.page }));
}

function dto(raw: string): BlockDto {
  const f = facetsOf(raw, "md");
  return { id: "x", raw, collapsed: false, children: [], properties: f.properties as [string, string][] };
}

function viewText(v: CellView): string {
  switch (v.k) {
    case "none": return "";
    case "check": return v.checked ? "[x]" : "[ ]";
    case "error": return `error: ${v.message}`;
    case "chips": return v.values.join(" ");
    default: return v.text;
  }
}

describe.each(CASES)("sheet formulas follow the user's local date ($zone at $at)", (c) => {
  beforeAll(() => {
    process.env.TZ = c.zone;
  });

  it("is really running in the zone, on a day the UTC date gets wrong", () => {
    const at = new Date(c.at);
    expect(at.getTimezoneOffset()).toBe(c.offsetMinutes);
    expect(at.toISOString().slice(0, 10)).toBe(c.utcDate);
    if (c.zone === "UTC") expect(c.utcDate).toBe(c.today);
    else expect(c.utcDate).not.toBe(c.today);
  });

  it("a live sheet shows today's date, the wall-clock time and today's relative dates", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(c.at));
    const rows = liveRows(c);
    createRoot((dispose) => {
      const results = createFormulaResultsMemo({ rows: () => rows, formulas: () => FORMULAS });
      const shown = (row: number, name: string) => formulaValueText(results().get(formulaResultKey(rows[row], name)));
      expect(shown(1, "day")).toBe(c.today);
      expect(shown(1, "clock")).toBe(`${c.today} ${c.time}`);
      expect(shown(1, "next")).toBe(c.tomorrow);
      // A date typed as today equals today(); yesterday's and tomorrow's do not.
      expect([0, 1, 2].map((row) => shown(row, "istoday"))).toEqual(["false", "true", "false"]);
      expect([0, 1, 2].map((row) => shown(row, "rel"))).toEqual(["1d ago", "today", "in 1d"]);
      dispose();
    });
  });

  it("a tine.filter on today() keeps the row dated today", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(c.at));
    const rows = liveRows(c);
    createRoot((dispose) => {
      const filtered = createFormulaFilterMemo({ rows: () => rows, formulas: () => new Map(), filter: () => "due == today()" });
      expect(filtered().error).toBeNull();
      expect(filtered().rows.map((row) => row.id)).toEqual(["r1"]);
      dispose();
    });
  });

  it("the static export shows the same cells as the live sheet", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(c.at));
    setWorkflow("todo");
    const owner = ["Sheet", "tine.view:: table", "tine.fields:: due=date",
      ...[...FORMULAS].map(([name, expr]) => `tine.formula.${name}:: ${expr}`)].join("\n");
    const dues = [c.yesterday, c.today, c.tomorrow];
    const [exported] = computeSheetExports([{
      page: "Sheet", path: [0], fp: "fp", omitted: 0,
      owner: dto(owner),
      rows: dues.map((due, i) => dto(`Row ${i}\ndue:: ${due}`)),
    }], "todo");
    if (!exported || exported.view !== "table") throw new Error(`expected a table export, got ${exported?.view}`);
    // The first column is the block title, which rows carry apart from `cells`.
    const labels = exported.columns.map((col) => col.label);
    const cell = (row: number, name: string) => viewText(exported.rows[row].cells[labels.indexOf(name) - 1]);
    expect(labels).toEqual(expect.arrayContaining(["day", "clock", "next", "istoday", "rel"]));
    expect(cell(1, "day")).toBe(c.today);
    expect(cell(1, "clock")).toBe(`${c.today} ${c.time}`);
    expect(cell(1, "next")).toBe(c.tomorrow);
    expect([0, 1, 2].map((row) => cell(row, "istoday"))).toEqual(["[ ]", "[x]", "[ ]"]);
    expect([0, 1, 2].map((row) => cell(row, "rel"))).toEqual(["1d ago", "today", "in 1d"]);
  });
});
