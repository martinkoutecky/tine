import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { SheetTable } from "./SheetTable";
import { Block } from "./Block";
import { backend } from "../backend";
import { backendReadsQueries, blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { ContextMenu } from "./ContextMenu";
import { initParser } from "../render/parse";
import { blockProperty, resetStore } from "../document";
import { setDoc } from "../document/model";
import type { ViewSettings, QueryStatistics } from "../editor/queryIr";
import type { RefGroup } from "../types";
import { setContextMenu } from "../ui";

beforeAll(() => initParser());
afterEach(() => { resetStore(); resetSharedQueryResultsForTests(); setContextMenu(null); document.body.innerHTML = ""; vi.restoreAllMocks(); });

function mount(view: ViewSettings, statistics?: QueryStatistics) {
  setDoc({ byId: { query: { id: "query", raw: "{{query (task TODO)}}", children: [], parent: null, page: "Sheet", collapsed: false } },
    pages: [{ name: "Sheet", title: "Sheet", kind: "page", roots: ["query"], preBlock: null, format: "md", readOnly: false, guide: false }], feed: ["Sheet"], loaded: true });
  const groups: RefGroup[] = [{ page: "Results", kind: "page", blocks: [{ id: "r1", raw: "Visible row", properties: [["cost", "2"]], children: [], collapsed: false }] }];
  let current!: () => ViewSettings;
  let setCurrent!: (next: ViewSettings) => ViewSettings;
  const apply = vi.fn((next: ViewSettings) => setCurrent(next));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => {
    [current, setCurrent] = createSignal(view);
    return <><SheetTable ownerId="query" rowSource="query" groups={groups}
    queryDisplay={{ get view() { return current(); }, apply, statistics }} /><ContextMenu /></>;
  }, root);
  return { root, dispose, apply, current };
}

const statistics: QueryStatistics = {
  count: 4,
  aggregates: [["cost", "sum"], ["cost", "avg"]],
  overall: [{ kind: "number", value: 120, skipped: 0 }, { kind: "number", value: 30, skipped: 0 }],
  groups: null, group_by: null, grouping_status: "none",
};

describe("query table footer", () => {
  it("receives complete statistics through the live query macro", async () => {
    setDoc({ byId: { query: { id: "query", raw: "{{query (task TODO)}}\ntine.view:: table\ntine.col-aggregates:: cost=sum", children: [], parent: null, page: "Sheet", collapsed: false } },
      pages: [{ name: "Sheet", title: "Sheet", kind: "page", roots: ["query"], preBlock: null, format: "md", readOnly: false, guide: false }], feed: ["Sheet"], loaded: true });
    backendReadsQueries({ "(task TODO)": { form: "(task TODO)", view: { view: "table", columns: ["cost"], aggregates: [["cost", "sum"]], sample: 1 } } });
    const groups: RefGroup[] = [{ page: "Results", kind: "page", blocks: [{ id: "r1", raw: "TODO Visible row\ncost:: 2", properties: [["cost", "2"]], children: [], collapsed: false }] }];
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
    vi.spyOn(backend(), "queryRun").mockResolvedValue({ ...blockRunResult(groups), statistics });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <><Block id="query" /><ContextMenu /></>, root);
    try {
      await vi.waitFor(() => expect(root.querySelector(".sheet-table")).not.toBeNull());
      const value = root.querySelector<HTMLButtonElement>(".sheet-aggregate-value")!;
      expect(value.textContent).toBe("120");
      value.click();
      const count = [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((item) => item.textContent?.trim() === "Count");
      count!.click();
      await vi.waitFor(() => expect(blockProperty("query", "tine.col-aggregates")).toBe("cost=count"));
    } finally { dispose(); }
  });
  it("uses the complete backend sample and edits the ordered query aggregates through the display writer", () => {
    const m = mount({ columns: ["cost"], aggregates: [["cost", "sum"], ["", "count"], ["cost", "avg"]] }, statistics);
    try {
      const value = m.root.querySelector<HTMLButtonElement>(".sheet-aggregate-value");
      expect(value?.textContent).toBe("120"); // visible row is 2; complete sample is 120
      value!.click();
      const items = [...document.querySelectorAll<HTMLElement>(".ctx-item")];
      expect(items.map((a) => a.textContent?.trim())).toEqual(["None", "Count", "✓ Sum", "Average"]);
      items.find((a) => a.textContent?.trim() === "Average")!.click();
      expect(m.current().aggregates).toEqual([["cost", "avg"], ["", "count"], ["cost", "avg"]]);
      expect(m.apply).toHaveBeenCalledOnce();
      expect(value!.textContent).toBe("30");
      value!.click();
      [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((a) => a.textContent?.trim() === "Count")!.click();
      expect(m.current().aggregates).toEqual([["cost", "count"], ["", "count"], ["cost", "avg"]]);
      value!.click();
      [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((a) => a.textContent?.trim() === "None")!.click();
      expect(m.current().aggregates).toEqual([["", "count"], ["cost", "avg"]]);
    } finally { m.dispose(); }
  });
  it("does not offer query property aggregates for table builtin columns", () => {
    const m = mount({ columns: ["cost", "state"], aggregates: [["cost", "sum"], ["state", "count"]] }, statistics);
    try { expect(m.root.querySelectorAll(".sheet-aggregate-value")).toHaveLength(1); }
    finally { m.dispose(); }
  });
  it("shows no invented total when statistics are absent", () => {
    const m = mount({ columns: ["cost"], aggregates: [["cost", "sum"]] });
    try { expect(m.root.querySelector(".sheet-aggregate-value")?.textContent).toBe(""); }
    finally { m.dispose(); }
  });
  it("leaves a requested total blank when the supplied statistics have no matching entry", () => {
    const m = mount({ columns: ["cost"], aggregates: [["cost", "count"]] }, statistics);
    try { expect(m.root.querySelector(".sheet-aggregate-value")?.textContent).toBe(""); }
    finally { m.dispose(); }
  });
  it("formats unavailable backend totals through the shared query summary", () => {
    const m = mount({ columns: ["cost"], aggregates: [["cost", "sum"]] }, {
      ...statistics, overall: [{ kind: "marker", reason: "non_numeric", skipped: 4 }],
    });
    try { expect(m.root.querySelector(".sheet-aggregate-value")?.textContent).toBe("Unavailable (non numeric)"); }
    finally { m.dispose(); }
  });
  it("labels an unsavable title sort as table-only and clears it through the label", () => {
    const m = mount({ columns: ["cost"] });
    try {
      m.root.querySelector<HTMLElement>(".sheet-title-header")!.click();
      const label = m.root.querySelector<HTMLButtonElement>(".sheet-table-only-sort");
      expect(label?.textContent).toContain("Table-only sort: Title");
      expect(m.apply).not.toHaveBeenCalled();
      label!.click();
      expect(m.root.querySelector(".sheet-table-only-sort")).toBeNull();
    } finally { m.dispose(); }
  });
});
