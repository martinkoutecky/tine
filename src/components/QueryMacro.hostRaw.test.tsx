// GH #855: a journal section renders its query on the journal feed away from
// its block on the sections page. The View settings the user chose there are
// `tine.*` properties of that block, and the feed must run and display the
// query with them, exactly as the sections page does. The feed only shows: it
// has no block of its own to write a View change into.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QueryMacro } from "./Macro";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetStore } from "../document";
import { SurfaceContext } from "./Block";
import { AstBody } from "../render/body";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import type { ParsedQuery, ViewSettings } from "../editor/queryIr";
import type { RefGroup } from "../types";

beforeAll(initParser);

afterEach(() => {
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.innerHTML = "";
});

const SECTION = "{{query (task TODO)}}\ntine.sort:: priority asc\ntine.group-field:: tags\nnotes:: not a view setting";

function groups(): RefGroup[] {
  return [{ page: "Work", kind: "page", blocks: [
    { id: "a", raw: "TODO first row", collapsed: false, children: [] },
    { id: "b", raw: "TODO second row", collapsed: false, children: [] },
  ] }];
}

/** The engine reads the view out of the host block's `tine.*` properties. */
function engineReadsHostProperties(extra: Partial<ParsedQuery> = {}) {
  const parse = vi.spyOn(backend(), "parseQuery").mockImplementation(async (_text, _dialect, properties) => {
    const view: ViewSettings = {};
    for (const [key, value] of properties ?? []) {
      if (key === "tine.sort") view.sort = [[value.split(" ")[0], value.endsWith("desc") ? "desc" : "asc"]];
      if (key === "tine.group-field") view.group_by = value;
    }
    return { query: { anchor: "block", filter: { kind: "true" }, source: { kind: "og", original: "(task TODO)", og_options: "" } }, view, ...extra };
  });
  const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
  return { parse, run };
}

function mount(raw: string) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <QueryMacro body="query (task TODO)" currentPage="Nov 3rd, 2026" hideWhenEmpty
    hostRaw={{ raw, format: "md" }} />, root);
  return { root, dispose };
}

it("the feed runs a section's query with the View properties of its block (GH #855)", async () => {
  const { parse, run } = engineReadsHostProperties();
  const { root, dispose } = mount(SECTION);
  try {
    await vi.waitFor(() => expect(root.querySelector(".query-count")?.textContent).toBe("2"));
    expect(parse.mock.calls.at(-1)?.[2]).toEqual([["tine.sort", "priority asc"], ["tine.group-field", "tags"]]);
    const view = run.mock.calls.at(-1)?.[1] as ViewSettings;
    expect(view.group_by).toBe("tags");
    expect(view.sort).toEqual([["priority", "asc"]]);
  } finally { dispose(); }
});

it.each(["table", "board"])("a section shown as %s on its page keeps both rows on the feed", async face => {
  engineReadsHostProperties({ block_presentation: face as ParsedQuery["block_presentation"] });
  const { root, dispose } = mount(`${SECTION}\ntine.view:: ${face}`);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("first row"));
    expect(root.textContent).toContain("second row");
  } finally { dispose(); }
});

for (const format of ["md", "org"] as const) {
  for (const name of ["query", "tine-query"]) {
    it.each(["pane:main", "sidebar:journal"])(`GH #914: ${format} ${name} keeps its table, columns, sorted rows and total in %s`, async surface => {
      const properties: [string, string][] = [["tine.view", "table"], ["tine.columns", "points"], ["tine.sort", "points desc"], ["tine.col-aggregates", "points=sum"]];
      const body = `${name} ${name === "query" ? "(task NOW)" : "@block and task = 'NOW'"}`;
      const raw = `{{${body}}}\n` + (format === "org"
        ? `:PROPERTIES:\n${properties.map(([key, value]) => `:${key}: ${value}`).join("\n")}\n:END:`
        : properties.map(([key, value]) => `${key}:: ${value}`).join("\n"));
      const view: ViewSettings = { view: "table", columns: ["points"], sort: [["points", "desc"]], aggregates: [["points", "sum"]] };
      const parse = vi.spyOn(backend(), "parseQuery").mockResolvedValue({
        query: { anchor: "block", filter: { kind: "true" }, source: { kind: "og", original: "(task NOW)", og_options: "" } },
        view, block_presentation: "table",
      });
      const run = vi.spyOn(backend(), "queryRun").mockResolvedValue({
        ...blockRunResult([{ page: "Work", kind: "page", blocks: [
          { id: "high", raw: "NOW High points", properties: [["points", "7"], ["other", "hidden"]], collapsed: false, children: [] },
          { id: "low", raw: "NOW Low points", properties: [["points", "2"]], collapsed: false, children: [] },
        ] }]),
        statistics: { count: 2, aggregates: [["points", "sum"]], overall: [{ kind: "number", value: 9, skipped: 0 }], groups: null, group_by: null, grouping_status: "none" },
      });
      const root = document.createElement("div"); document.body.append(root);
      const dispose = render(() => <SurfaceContext.Provider value={surface}>
        <QueryMacro body={body} currentPage="Nov 8th, 2026" hideWhenEmpty hostRaw={{ raw, format }} />
      </SurfaceContext.Provider>, root);
      try {
        await vi.waitFor(() => expect(root.querySelector(".sheet-table")).not.toBeNull());
        expect(root.querySelectorAll(".sheet-header-cell")).toHaveLength(2);
        expect(root.querySelector(".sheet-field-header")?.textContent).toContain("points");
        const rows = [...root.querySelectorAll(".sheet-title-cell")].map(cell => cell.textContent);
        expect(rows[0]).toContain("High points"); expect(rows[1]).toContain("Low points");
        expect(root.querySelector(".sheet-aggregate-value")?.textContent).toBe("9");
        expect(root.querySelector("button.sheet-aggregate-value")).toBeNull();
        expect(root.querySelector(".qc-edit")).toBeNull();
        const actions = root.querySelector<HTMLDetailsElement>(".qc-actions")!;
        actions.open = true; actions.dispatchEvent(new Event("toggle"));
        await vi.waitFor(() => expect(document.querySelector(".qc-menu")).not.toBeNull());
        expect([...document.querySelectorAll(".qc-menu button")].map(button => button.textContent)).not.toContain("View");
        expect(parse).toHaveBeenCalledTimes(1);
        expect(parse.mock.calls[0][2]).toEqual(properties);
        expect(run).toHaveBeenCalledTimes(1);
        expect(run.mock.calls[0][1]).toEqual(view);
      } finally { dispose(); }
    });
  }
}

it("GH #914: a section keeps its board and an explicitly empty column selection", async () => {
  engineReadsHostProperties({ view: { view: "board" }, block_presentation: "board" });
  const board = mount(`${SECTION}\ntine.view:: board`);
  try { await vi.waitFor(() => expect(board.root.querySelector(".sheet-board")).not.toBeNull()); }
  finally { board.dispose(); }
  vi.restoreAllMocks(); resetSharedQueryResultsForTests();
  engineReadsHostProperties({ view: { view: "table", columns: [] }, block_presentation: "table" });
  const table = mount(`${SECTION}\ntine.view:: table\ntine.columns::`);
  try {
    await vi.waitFor(() => expect(table.root.querySelector(".sheet-table")).not.toBeNull());
    expect(table.root.querySelectorAll(".sheet-header-cell")).toHaveLength(1);
    table.root.querySelector(".block-sheet-container")!.dispatchEvent(new Event("pointerenter"));
    expect(table.root.querySelector(".sheet-aggregate-corner-toggle")).toBeNull();
  } finally { table.dispose(); }
});

it("GH #914: the existing sheet reads formulas, filtering, schema and widths from the same detached host", async () => {
  const { run } = engineReadsHostProperties({ view: { view: "table" }, block_presentation: "table" });
  run.mockResolvedValue(blockRunResult([{ page: "Work", kind: "page", blocks: [
    { id: "high", raw: "NOW High points\npoints:: 7", properties: [["points", "7"]], collapsed: false, children: [] },
    { id: "low", raw: "NOW Low points\npoints:: 2", properties: [["points", "2"]], collapsed: false, children: [] },
  ] }]));
  const { root, dispose } = mount(`${SECTION}\ntine.view:: table\ntine.fields:: points=number\ntine.formula.double:: points * 2\ntine.filter:: points > 3\ntine.table-widths:: title=240;prop%3Apoints=120`);
  try {
    await vi.waitFor(() => expect(root.querySelectorAll(".sheet-title-cell")).toHaveLength(1));
    expect(root.querySelector(".sheet-title-cell")?.textContent).toContain("High points");
    expect(root.querySelector(".sheet-table")?.textContent).not.toContain("Low points");
    expect(root.querySelector('[data-sheet-field="formula:double"]')?.textContent).toContain("double");
    expect(root.querySelector(".sheet-table")?.textContent).toContain("14");
    expect(root.querySelector<HTMLElement>(".sheet-table")?.style.gridTemplateColumns).toContain("240px");
    expect(root.querySelector(".sheet-table-column-resize-handle")).toBeNull();
  } finally { dispose(); }
});

it("GH #914: a raw-only block renderer also supplies its host's View to the shared query renderer", async () => {
  const properties: [string, string][] = [["tine.view", "table"], ["tine.columns", "points"]];
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (_text, _dialect, supplied) => ({
    query: { anchor: "block", filter: { kind: "true" }, source: { kind: "og", original: "(task TODO)", og_options: "" } },
    view: supplied?.length ? { view: "table", columns: ["points"] } : {},
  }));
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <AstBody raw={"{{query (task TODO)}}\ntine.view:: table\ntine.columns:: points"} format="md" />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector(".sheet-table")).not.toBeNull());
    expect(root.querySelector(".sheet-field-header")?.textContent).toContain("points");
    expect(backend().parseQuery).toHaveBeenCalledWith("(task TODO)", "macro_query", properties);
  } finally { dispose(); }
});

it.each(["md", "org"] as const)("the shared View reading keeps case-insensitive property names in %s", async format => {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (_text, _dialect, properties) => ({
    query: { anchor: "block", filter: { kind: "true" }, source: { kind: "og", original: "(task TODO)", og_options: "" } },
    view: { view: properties?.find(([key]) => key.toLowerCase() === "tine.view")?.[1] === "table" ? "table" : "list" },
  }));
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
  const raw = `{{query (task TODO)}}\n${format === "md" ? "TINE.VIEW:: table" : ":PROPERTIES:\n:TINE.VIEW: table\n:END:"}`;
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacro body="query (task TODO)" hostRaw={{ raw, format }} />, root);
  try { await vi.waitFor(() => expect(root.querySelector(".sheet-table")).not.toBeNull()); }
  finally { dispose(); }
});
