// Contract 1 of static-export sheets (family 7): exported cells match the app;
// query totals summarize exported rows, while live totals cover the complete query. Each fixture is mounted through the LIVE Block/SheetTable/SheetBoard/
// SheetGrid components and also fed to `computeSheetExport` as detached DTOs; the two
// observations agree cell for cell; query export totals use exported rows. A sheet feature the
// export cannot see (or renders differently) fails here, not in a user's published site.
import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Block } from "../components/Block";
import { SheetBoard } from "../components/SheetBoard";
import { SheetTable } from "../components/SheetTable";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { facetsOf } from "../render/facets";
import { initParser } from "../render/parse";
import { setWorkflow } from "../ui";
import type { BlockDto } from "../types";
import type { CellView } from "./cellPresentation";
import type { ViewSettings } from "../editor/queryIr";
import { computeSheetExport, computeSheetExports, type QuerySource, type SheetExport, type SheetInput } from "./staticExport";

const NOW = new Date(2026, 8, 29, 12, 0, 0);

beforeAll(async () => {
  await initParser();
});
beforeEach(() => setWorkflow("todo"));
afterEach(() => {
  disposers.splice(0).forEach((dispose) => dispose());
  resetStore();
  document.body.innerHTML = "";
});

interface Row { raw: string; kids?: string[] }
interface Fixture { owner: string; rows: Row[] }

/** A BlockDto the way the Rust projection ships one: raw plus lsdoc-derived facets. */
function dto(raw: string, children: BlockDto[] = []): BlockDto {
  const f = facetsOf(raw, "md");
  return {
    id: "x", raw, collapsed: false, children,
    marker: f.marker ?? undefined, priority: f.priority ?? undefined,
    scheduled: f.scheduled ?? undefined, deadline: f.deadline ?? undefined,
    tags: f.tags, properties: f.properties as [string, string][],
  };
}

function inputOf(fx: Fixture): SheetInput {
  return {
    page: "Sheet", path: [0], fp: "fp", omitted: 0,
    owner: dto(fx.owner),
    rows: fx.rows.map((r) => dto(r.raw, (r.kids ?? []).map((k) => dto(k)))),
  };
}

const disposers: (() => void)[] = [];

function mountLive(fx: Fixture): HTMLElement {
  const byId: Record<string, any> = {};
  const rowIds = fx.rows.map((_, i) => `r${i}`);
  byId.tbl = { id: "tbl", raw: fx.owner, collapsed: false, parent: null, page: "Sheet", children: rowIds };
  fx.rows.forEach((r, i) => {
    const kids = (r.kids ?? []).map((_, c) => `r${i}c${c}`);
    byId[`r${i}`] = { id: `r${i}`, raw: r.raw, collapsed: false, parent: "tbl", page: "Sheet", children: kids };
    (r.kids ?? []).forEach((k, c) => {
      byId[`r${i}c${c}`] = { id: `r${i}c${c}`, raw: k, collapsed: false, parent: `r${i}`, page: "Sheet", children: [] };
    });
  });
  setDoc({
    byId,
    pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["tbl"], format: "md", readOnly: false, guide: false }],
    feed: ["Sheet"],
    loaded: true,
  } as any);
  const root = document.createElement("div");
  document.body.appendChild(root);
  disposers.push(render(() => <Block id="tbl" />, root));
  return root;
}

const clean = (el: Element): string => {
  const copy = el.cloneNode(true) as Element;
  copy.querySelectorAll(".sheet-cell-handle,.sheet-table-column-resize-handle").forEach((n) => n.remove());
  return (copy.textContent ?? "").replace(/\s+/g, " ").trim();
};

/** The text a viewer reads in an exported cell. */
function viewText(v: CellView): string {
  switch (v.k) {
    case "none": return "";
    case "chips": return v.values.join(" ");
    case "check": return v.checked ? "[x]" : "[ ]";
    case "error": return "⚠";
    default: return v.text;
  }
}

function liveCellText(el: Element): string {
  const box = el.querySelector('input[type="checkbox"]') as HTMLInputElement | null;
  return box ? (box.checked ? "[x]" : "[ ]") : clean(el);
}

const live = (root: HTMLElement) => ({
  headers: [...root.querySelectorAll(".sheet-header-cell:not(.sheet-add-field)")].map(clean),
  rows: [...root.querySelectorAll(".sheet-title-cell")].map((title) => {
    const row = (title as HTMLElement).dataset.row!;
    const cells = [...root.querySelectorAll(`.sheet-field-cell[data-row="${row}"]`)].map(liveCellText);
    return [clean(title.querySelector(".sheet-cell-body")!), ...cells];
  }),
  aggregates: [...root.querySelectorAll(".sheet-aggregate-value")].map(clean),
});

function tableOf(x: SheetExport | null) {
  if (!x || x.view !== "table") throw new Error(`expected a table export, got ${x?.view}`);
  return x;
}

const TABLE: Fixture = {
  owner: "Table\ntine.view:: table\ntine.fields:: price=number;qty=number\ntine.formula.total:: price * qty\ntine.col-aggregates:: prop:price=sum;formula:total=sum",
  rows: [
    { raw: "TODO [#A] First #x\nprice:: 5\nqty:: 2\nscheduled:: <2026-09-01 Tue>" },
    { raw: "DONE Second\nprice:: 1\nqty:: 2" },
    { raw: "Third\nprice:: 3\nnote:: plain words" },
  ],
};

describe("static sheet export equals the live app (contract 1)", () => {
  it("table: columns, cells, formula values and aggregates match the live SheetTable", () => {
    const exported = tableOf(computeSheetExport(inputOf(TABLE), { now: NOW, workflow: "todo" }));
    const app = live(mountLive(TABLE));
    expect(exported.columns.map((c) => (c.formula ? "ƒ" : "") + c.label)).toEqual(app.headers);
    expect(exported.rows.map((r) => [r.title, ...r.cells.map(viewText)])).toEqual(app.rows);
    const totals = (exported.footer ?? []).flatMap((a) => (a ? [a.text] : []));
    expect(totals).toEqual(app.aggregates);
    // The fixture must actually exercise a formula and an aggregate, not vacuously agree.
    expect(app.aggregates.length).toBeGreaterThan(0);
    expect(app.headers.some((h) => h.startsWith("ƒ"))).toBe(true);
  });

  it("table: a filter drops the same rows the live table drops", () => {
    const fx = { ...TABLE, owner: TABLE.owner + "\ntine.filter:: price > 2" };
    const exported = tableOf(computeSheetExport(inputOf(fx), { now: NOW, workflow: "todo" }));
    const app = live(mountLive(fx));
    expect(app.rows).toHaveLength(2);
    expect(exported.rows.map((r) => [r.title, ...r.cells.map(viewText)])).toEqual(app.rows);
    expect(exported.filterError).toBeNull();
  });

  it("board: columns, order and cards match the live SheetBoard, grouped by state and by tags", () => {
    for (const [groupBy, wf] of [["state", "todo"], ["state", "now"], ["tags", "todo"]] as const) {
      disposers.splice(0).forEach((dispose) => dispose());
      setWorkflow(wf);
      const fx: Fixture = {
        owner: `Board\ntine.view:: board\ntine.group-by:: ${groupBy}`,
        rows: [
          { raw: "TODO Write tests #a" },
          { raw: "DOING Implement #a #b" },
          { raw: "DONE Shipped\nscheduled:: <2026-09-02 Wed>" },
          { raw: "No marker" },
        ],
      };
      const exported = computeSheetExport(inputOf(fx), { now: NOW, workflow: wf });
      if (!exported || exported.view !== "board") throw new Error("expected a board");
      const root = mountLive(fx);
      const columns = [...root.querySelectorAll(".sheet-board-column:not(.sheet-board-add-tag-column)")].map((col) => ({
        label: clean(col.querySelector(".sheet-board-header > span:first-child")!),
        cards: [...col.querySelectorAll(".sheet-board-card-title")].map(clean),
      }));
      expect(columns.length).toBeGreaterThan(1);
      expect(exported.columns.map((c) => ({ label: c.label, cards: c.cards.map((k) => k.title) }))).toEqual(columns);
    }
  });

  it("grid: width, header flag and column aggregates match the live SheetGrid", () => {
    const fx: Fixture = {
      owner: "Grid\ntine.view:: grid\ntine.header:: true\ntine.col-aggregates:: 1=sum",
      rows: [
        { raw: "", kids: ["Name", "Qty", "Note"] },
        { raw: "", kids: ["a", "2", "x"] },
        { raw: "", kids: ["b", "5"] },
      ],
    };
    const exported = computeSheetExport(inputOf(fx), { now: NOW, workflow: "todo" });
    if (!exported || exported.view !== "grid") throw new Error("expected a grid");
    const app = live(mountLive(fx));
    expect(exported.cols).toBe(3);
    expect(exported.header).toBe(true);
    expect((exported.footer ?? []).flatMap((a) => (a ? [a.text] : []))).toEqual(app.aggregates);
    expect(app.aggregates).toEqual(["7"]);
  });
});

/** A query block's sheet: the owner, and the rows the query answered (page + raw each). */
interface QueryFixture {
  owner: string;
  presentation?: QuerySource["presentation"];
  view?: ViewSettings;
  rows: { page: string; raw: string }[];
}

function queryInputOf(fx: QueryFixture): SheetInput {
  const rows = fx.rows.map((r) => dto(r.raw));
  return {
    page: "Sheet", path: [0], fp: "fp", omitted: 0,
    owner: dto(fx.owner),
    rows: [],
    query: { fp: "qfp", presentation: fx.presentation ?? null, view: fx.view ?? {}, pages: fx.rows.map((r) => r.page), rows },
  };
}

/** Mount the LIVE query face: the same component + props Macro.tsx gives its results. */
function mountLiveQuery(fx: QueryFixture, face: "table" | "board", groupBy?: string, statistics?: import("../editor/queryIr").QueryStatistics): HTMLElement {
  setDoc({
    byId: { tbl: { id: "tbl", raw: fx.owner, collapsed: false, parent: null, page: "Sheet", children: [] } },
    pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["tbl"], format: "md", readOnly: false, guide: false }],
    feed: ["Sheet"],
    loaded: true,
  } as any);
  const byPage = new Map<string, BlockDto[]>();
  fx.rows.forEach((r, i) => {
    const block = { ...dto(r.raw), id: `q${i}` };
    byPage.set(r.page, [...(byPage.get(r.page) ?? []), block]);
  });
  const groups = [...byPage].map(([page, blocks]) => ({ page, kind: "page" as const, blocks }));
  const root = document.createElement("div");
  document.body.appendChild(root);
  disposers.push(
    render(
      () => face === "table"
        ? <SheetTable ownerId="tbl" rowSource="query" groups={groups} queryDisplay={{ view: fx.view ?? {}, statistics, apply: () => {} }} />
        : <SheetBoard ownerId="tbl" rowSource="query" groupBy={groupBy} groups={groups} />,
      root
    )
  );
  return root;
}

const QUERY_ROWS = [
  { page: "Alpha", raw: "TODO [#A] First #x\nprice:: 5\nqty:: 2" },
  { page: "Alpha", raw: "DONE Second\nprice:: 1\nqty:: 2" },
  { page: "Beta", raw: "Third\nprice:: 3\nnote:: plain words" },
];

describe("static query-backed sheets present exported rows (family 7)", () => {
  it("table: rows and cells match the live query face; totals use only exported rows", () => {
    const fx: QueryFixture = {
      owner: "Q {{query (task TODO DONE)}}\ntine.view:: table\ntine.fields:: price=number;qty=number\ntine.formula.total:: price * qty\ntine.col-aggregates:: prop:price=sum;formula:total=sum",
      view: { aggregates: [["price", "sum"]] },
      rows: QUERY_ROWS,
    };
    const exported = tableOf(computeSheetExport(queryInputOf(fx), { now: NOW, workflow: "todo" }));
    const app = live(mountLiveQuery(fx, "table", undefined, {
      count: 30, aggregates: [["price", "sum"]],
      overall: [{ kind: "number", value: 90, skipped: 0 }],
      groups: null, group_by: null, grouping_status: "none",
    }));
    expect(app.rows).toHaveLength(3);
    expect(exported.columns.map((c) => (c.formula ? "ƒ" : "") + c.label)).toEqual(app.headers);
    expect(app.headers.map((h) => h.toLowerCase())).toContain("page");
    expect(exported.rows.map((r) => [r.title, ...r.cells.map(viewText)])).toEqual(app.rows);
    // Master publish/sheet.rs totals the exported rows, even when the live
    // query has a larger complete sample. Private/unexported rows stay out.
    expect((exported.footer ?? []).flatMap((a) => (a ? [a.text] : []))).toEqual(["9", "12 (1 skipped)"]);
    expect(app.aggregates).toEqual(["90"]);
  });

  it("table: the query's `columns` choose the fields and their order, as the live table does", () => {
    const fx: QueryFixture = {
      owner: "Q {{query (task TODO DONE)}}\ntine.view:: table",
      view: { columns: ["page", "priority", "price"] },
      rows: QUERY_ROWS,
    };
    const exported = tableOf(computeSheetExport(queryInputOf(fx), { now: NOW, workflow: "todo" }));
    const app = live(mountLiveQuery(fx, "table"));
    expect(exported.columns.map((c) => c.label)).toEqual(app.headers);
    expect(app.headers).toHaveLength(4);
    expect(exported.rows.map((r) => [r.title, ...r.cells.map(viewText)])).toEqual(app.rows);
  });

  it("table: a `tine.filter` drops the same query rows the live table drops", () => {
    const fx: QueryFixture = {
      owner: "Q {{query (task TODO DONE)}}\ntine.view:: table\ntine.fields:: price=number\ntine.filter:: price > 2",
      rows: QUERY_ROWS,
    };
    const exported = tableOf(computeSheetExport(queryInputOf(fx), { now: NOW, workflow: "todo" }));
    const app = live(mountLiveQuery(fx, "table"));
    expect(app.rows).toHaveLength(2);
    expect(exported.rows.map((r) => [r.title, ...r.cells.map(viewText)])).toEqual(app.rows);
  });

  it("board: the query's rows group into the same columns and cards as the live board", () => {
    for (const groupBy of ["state", "page"]) {
      disposers.splice(0).forEach((dispose) => dispose());
      const fx: QueryFixture = {
        owner: `Q {{query (task TODO DONE DOING)}}\ntine.view:: board\ntine.group-by:: ${groupBy}`,
        rows: [
          { page: "Alpha", raw: "TODO Write tests" },
          { page: "Beta", raw: "DOING Implement" },
          { page: "Beta", raw: "DONE Shipped" },
        ],
      };
      const exported = computeSheetExport(queryInputOf(fx), { now: NOW, workflow: "todo" });
      if (!exported || exported.view !== "board") throw new Error("expected a board");
      const root = mountLiveQuery(fx, "board", groupBy);
      const columns = [...root.querySelectorAll(".sheet-board-column:not(.sheet-board-add-tag-column)")].map((col) => ({
        label: clean(col.querySelector(".sheet-board-header > span:first-child")!),
        cards: [...col.querySelectorAll(".sheet-board-card-title")].map(clean),
      }));
      expect(columns.length).toBeGreaterThan(1);
      expect(exported.columns.map((c) => ({ label: c.label, cards: c.cards.map((k) => k.title) }))).toEqual(columns);
    }
  });

  it("the query's own presentation beats tine.view; a list or grid face has no sheet export", () => {
    const base = { owner: "Q {{query (task TODO)}}\ntine.view:: table", rows: QUERY_ROWS };
    expect(computeSheetExport(queryInputOf({ ...base, presentation: "board" }), { now: NOW, workflow: "todo" })?.view).toBe("board");
    expect(computeSheetExport(queryInputOf({ ...base, presentation: "table" }), { now: NOW, workflow: "todo" })?.view).toBe("table");
    expect(computeSheetExport(queryInputOf({ ...base, presentation: "list" }), { now: NOW, workflow: "todo" })).toBeNull();
    const grid = { ...base, owner: "Q {{query (task TODO)}}\ntine.view:: grid" };
    expect(computeSheetExport(queryInputOf(grid), { now: NOW, workflow: "todo" })).toBeNull();
  });

  it("the export echoes the query's own fingerprint and marks itself query-backed", () => {
    const fx: QueryFixture = { owner: "Q {{query (task TODO)}}\ntine.view:: table", rows: QUERY_ROWS };
    const exported = computeSheetExport(queryInputOf(fx), { now: NOW, workflow: "todo" });
    expect(exported).toMatchObject({ fp: "qfp", query: true, view: "table" });
  });
});

describe("static sheet export failure handling (contract 3, TS half)", () => {
  it("a block whose tine.view is not a sheet view yields no export", () => {
    const fx = { owner: "Plain\ntine.view:: nonsense", rows: [] };
    expect(computeSheetExport(inputOf(fx), { now: NOW, workflow: "todo" })).toBeNull();
  });

  it("a query-backed table or board is the macro's, not a children sheet (same answer as the live Block)", () => {
    for (const view of ["table", "board"]) {
      const fx = { owner: `Tasks {{query (task TODO)}}\ntine.view:: ${view}`, rows: [{ raw: "a" }] };
      expect(computeSheetExport(inputOf(fx), { now: NOW, workflow: "todo" })).toBeNull();
    }
    const grid = { owner: "Grid {{query (task TODO)}}\ntine.view:: grid", rows: [{ raw: "", kids: ["a"] }] };
    expect(computeSheetExport(inputOf(grid), { now: NOW, workflow: "todo" })?.view).toBe("grid");
  });

  it("a sheet that throws exports as an error record, not an exception", () => {
    const input = inputOf(TABLE);
    (input as any).rows = [null];
    const exported = computeSheetExport(input, { now: NOW, workflow: "todo" });
    expect(exported?.view).toBe("error");
    expect(() => computeSheetExports([input, inputOf(TABLE)], "todo", NOW)).not.toThrow();
    expect(computeSheetExports([input, inputOf(TABLE)], "todo", NOW).map((x) => x.view)).toEqual(["error", "table"]);
  });
});

// The middle layer of the Rust -> TS -> Rust chain over one fixture graph. The Rust
// half (`crates/tine-graph-features/tests/sheets_export.rs`) asserts it produces
// `inputs.json` and that it lays `exports.json` out; this half asserts the app turns
// the first into the second. Refresh with BLESS_SHEETS=1 after the Rust side.
describe("Rust hand-off chain", () => {
  const dir = path.resolve(__dirname, "../../crates/tine-graph-features/tests/fixtures/sheets");
  it("the app's answer to the Rust inputs is the golden exports.json", () => {
    const inputs = JSON.parse(fs.readFileSync(path.join(dir, "inputs.json"), "utf8")) as SheetInput[];
    const actual = JSON.stringify(computeSheetExports(inputs, "now", NOW), null, 2) + "\n";
    if (process.env.BLESS_SHEETS) fs.writeFileSync(path.join(dir, "exports.json"), actual);
    expect(actual).toBe(fs.readFileSync(path.join(dir, "exports.json"), "utf8"));
    // The non-sheet candidate is dropped by the app, not by Rust (I-12).
    expect(inputs.length).toBe(7);
    expect(JSON.parse(actual).map((x: SheetExport) => x.view)).toEqual(["table", "board", "grid", "table", "board"]);
    // The two query-backed blocks are answered from their result rows; the query-face
    // grid stays a result list (no export), exactly as the live query block does.
    expect(JSON.parse(actual).map((x: SheetExport) => !!x.query)).toEqual([false, false, false, true, true]);
  });
});
