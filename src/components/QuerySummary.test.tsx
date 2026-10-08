// Ported from master src/components/QuerySummary.test.tsx (the summary renders the
// engine's `statistics`, og QueryResultParts.QueryStatisticsSummary).
// **Every aggregate the view asks for, over the rows the Board renders** (P5B).
//
// The list/summary panel read `aggregates[0]` and `group_by` and did its own
// property lookup on a private `{page, props}` shadow row. Two defects came out
// of that: a view carrying three aggregates rendered one, and the grouping the
// summary used could disagree with the Board mounted directly beneath it —
// `group-by:: state` meant the task marker to one and an ordinary property named
// `state` to the other.
//
// The summary now renders the whole ordered list, and groups the SAME result
// records `SheetTable` and `SheetBoard` flatten, through the one shared reader.

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { blockProperty, resetStore } from "../document";
import { setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { BlockDto, RefGroup } from "../types";
import type { ParsedQuery, ViewSettings, QueryResult, QueryStatisticsCell } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function page(roots: string[]): FeedPage {
  return {
    name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots, format: "md", readOnly: false, guide: false,
  };
}

function node(id: string, raw: string): StoreNode {
  return { id, raw, collapsed: false, parent: null, page: "Sheet", children: [] };
}

/** A result row as the BACKEND ships it: the facets are COMPUTED in Rust off
 *  the lsdoc projection and ride on the DTO, so a marker or a tag is a field of
 *  the row and not something the frontend re-derives from its text. */
function resultBlock(
  id: string,
  raw: string,
  properties: [string, string][],
  extra: Partial<BlockDto> = {},
): BlockDto {
  return { id, raw, collapsed: false, children: [], properties, ...extra } as BlockDto;
}

/** Two pages of results, so a `page` grouping has something to say. */
function groups(): RefGroup[] {
  return [
    {
      page: "Alpha",
      kind: "page",
      blocks: [
        resultBlock("r1", "TODO Refresh the Guide #docs", [["cost", "10"], ["state", "open"]], {
          marker: "TODO",
          tags: ["docs", "urgent"],
        }),
        resultBlock("r2", "TODO Draft the notes #docs", [["cost", "4"], ["state", "open"]], {
          marker: "TODO",
          tags: ["docs"],
        }),
      ],
    },
    {
      page: "Beta",
      kind: "page",
      blocks: [
        resultBlock("r3", "DONE Publish the demo", [["cost", "not a number"], ["state", "done"]], {
          marker: "DONE",
        }),
        resultBlock("r4", "DONE Archive", [["state", "done"]], { marker: "DONE" }),
      ],
    },
  ];
}

/** Load a query block and state what the ENGINE reads for it — the merged view,
 *  which is where the six display facts arrive from (§4.1). */
function load(raw: string, view: ViewSettings): void {
  setDoc({ byId: { query: node("query", raw) }, pages: [page(["query"])], feed: ["Sheet"], loaded: true });
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text: string): Promise<ParsedQuery> => ({
    query: {
      anchor: "block",
      filter: { kind: "raw", text, diagnostic_kind: "not_applicable" },
      diagnostics: [],
      source: { kind: "og", original: text, og_options: "" },
    },
    view,
  } as unknown as ParsedQuery));
  vi.spyOn(backend(), "queryRun").mockResolvedValue(statisticsResult(groups(), view));
}

// Fixture producer: the component receives already-computed facts. The final
// Q4 regression deliberately supplies facts inconsistent with visible rows.
function statisticsResult(groups: RefGroup[], view: ViewSettings): QueryResult {
  const rows = groups.flatMap((group) => group.blocks.map((block) => ({ page: group.page, block })));
  const field = view.group_by === "" ? null : view.group_by ?? (view.view === "board" ? "state" : null);
  const aggregates = view.aggregates?.length ? view.aggregates : field ? [["", "count"] as [string, "count"]] : [];
  const fold = (rows: { page: string; block: BlockDto }[]): QueryStatisticsCell[] => aggregates.map(([key, op]) => {
    if (op === "count") return { kind: "number", value: rows.length, skipped: 0 };
    const values = rows.map(({ block }) => parseFloat(block.properties?.find(([name]) => name === key)?.[1] ?? "")).filter(Number.isFinite);
    const skipped = rows.length - values.length;
    if (!values.length) return { kind: "marker", reason: rows.length ? "non_numeric" : "empty_group", skipped };
    const sum = values.reduce((sum, n) => sum + n, 0);
    return { kind: "number", value: op === "sum" ? sum : sum / values.length, skipped };
  });
  const buckets = new Map<string | null, typeof rows>();
  for (const row of rows) {
    const keys = field === "tags" ? row.block.tags ?? [] : [field === "page" ? row.page : field === "state" ? row.block.marker ?? null : row.block.properties?.find(([key]) => key === field?.slice(5))?.[1] ?? null];
    for (const key of keys.length ? keys : [null]) buckets.set(key, [...(buckets.get(key) ?? []), row]);
  }
  return { ...blockRunResult(groups), statistics: aggregates.length ? {
    count: rows.length, aggregates, group_by: field, overall: fold(rows),
    groups: field ? [...buckets].map(([key, rows]) => ({ key, count: rows.length, cells: fold(rows) })) : null,
    grouping_status: field ? "exact" : "none",
  } : undefined };
}

async function mountQuery(): Promise<{ root: HTMLElement; dispose: () => void }> {
  const mounted = mount(() => <Block id="query" />);
  try {
    await vi.waitFor(() => {
      if (!mounted.root.querySelector(".query-summary, .query-summary-table")) {
        throw new Error("the summary never rendered");
      }
    });
    await settle();
    return mounted;
  } catch (error) {
    mounted.dispose();
    throw error;
  }
}

const cells = (root: HTMLElement, selector: string) =>
  [...root.querySelectorAll(selector)].map((element) => element.textContent?.trim());

it("q4_query_summary_and_footer_use_returned_statistics", async () => {
  load("{{query (todo TODO)}}\ntine.view:: table", { view: "table", aggregates: [["cost", "sum"]] });
  vi.mocked(backend().queryRun).mockResolvedValue({
    ...blockRunResult(groups()),
    statistics: {
      count: 100, aggregates: [["cost", "sum"]], group_by: null,
      overall: [{ kind: "number", value: 12345, skipped: 0 }],
      groups: null, grouping_status: "none",
    },
  } as ReturnType<typeof blockRunResult>);
  const { root, dispose } = await mountQuery();
  try {
    expect(cells(root, ".query-summary .qs-value")).toContain("12345");
    // og: the table FOOTER half of this master test (`.sheet-aggregate-value`)
    // is Q4b's — SheetTable's footer reads column aggregates, the Display surface.
  } finally { dispose(); }
});

describe("the overall summary", () => {
  it("q4_statistics_resource_limit_is_visible_not_zero", async () => {
    load("{{query (todo TODO)}}", { aggregates: [["cost", "sum"]] });
    const message = "Exact query statistics exceed the available memory limit. Narrow the query or remove grouping or aggregates.";
    vi.mocked(backend().queryRun).mockRejectedValue(new Error(message));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain(message));
      expect(root.querySelector(".query-summary .qs-value")).toBeNull();
      expect(backend().queryRun).toHaveBeenCalledTimes(1);
    } finally { dispose(); }
  });

  it("renders EVERY requested aggregate, in order, repeats included", async () => {
    // FAIL-BEFORE: only `aggregates[0]` was folded, so this rendered "4".
    load("{{query (todo TODO)}}", {
      aggregates: [["", "count"], ["cost", "sum"], ["cost", "avg"], ["cost", "sum"]],
    });
    const { root, dispose } = await mountQuery();
    try {
      expect(cells(root, ".query-summary .qs-label")).toEqual([
        "Count:",
        "Sum of cost:",
        "Avg of cost:",
        "Sum of cost:",
      ]);
      expect(cells(root, ".query-summary .qs-value")).toEqual(["4", "14", "7", "14"]);
    } finally {
      dispose();
    }
  });

  it("keeps aggregate property names literal even when they look like field identities", async () => {
    load("{{query (todo TODO)}}", {
      aggregates: [["prop:cost", "sum"], ["formula:cost", "sum"], ["cost", "sum"]],
    });
    vi.mocked(backend().queryRun).mockResolvedValue(statisticsResult([{
      page: "Alpha", kind: "page", blocks: [
        resultBlock("literal", "A result", [["prop:cost", "7"], ["formula:cost", "11"], ["cost", "2"]]),
      ],
    }], { aggregates: [["prop:cost", "sum"], ["formula:cost", "sum"], ["cost", "sum"]] }));
    const { root, dispose } = await mountQuery();
    try {
      expect(cells(root, ".query-summary .qs-value")).toEqual(["7", "11", "2"]);
    } finally {
      dispose();
    }
  });

  it.each(["table", "board", "search"] as const)("renders every aggregate in the %s view", async (view) => {
    load(`{{query (todo TODO)}}\ntine.view:: ${view}\ntine.group-field:: `, { view, group_by: "", aggregates: [["", "count"], ["cost", "sum"], ["cost", "sum"]] });
    const { root, dispose } = await mountQuery();
    try {
      expect(cells(root, ".query-summary .qs-value")).toEqual(["4", "14", "14"]);
    } finally { dispose(); }
  });

  it("counts the rows that could not contribute", async () => {
    load("{{query (todo TODO)}}", { aggregates: [["cost", "sum"]] });
    const { root, dispose } = await mountQuery();
    try {
      expect(root.querySelector(".query-summary .qs-skip")?.textContent).toContain("2 non-numeric");
    } finally {
      dispose();
    }
  });
});

describe("the grouped breakdown", () => {
  it("groups by the CANONICAL field the engine resolved, not by a bare token", async () => {
    // `prop:state` is the ordinary property named `state`. The old summary read
    // `r.props["state"]` for a bare `state` while the Board beside it grouped by
    // the task marker; the canonical id is what tells the two apart.
    load("{{query (todo TODO)}}", { group_by: "prop:state", aggregates: [["", "count"]] });
    const { root, dispose } = await mountQuery();
    try {
      expect(cells(root, ".query-summary-table thead th")).toEqual(["state", "Count"]);
      expect(cells(root, ".query-summary-table tbody td")).toEqual(["open", "2", "done", "2"]);
    } finally {
      dispose();
    }
  });

  it("groups by the TASK MARKER when the field is the builtin", async () => {
    load("{{query (todo TODO)}}", { group_by: "state", aggregates: [["", "count"]] });
    const { root, dispose } = await mountQuery();
    try {
      expect(cells(root, ".query-summary-table thead th")).toEqual(["State", "Count"]);
      expect(cells(root, ".query-summary-table tbody td")).toEqual(["TODO", "2", "DONE", "2"]);
    } finally {
      dispose();
    }
  });

  it("groups by the source page, which the shadow row could only guess at", async () => {
    load("{{query (todo TODO)}}", { group_by: "page", aggregates: [["", "count"]] });
    const { root, dispose } = await mountQuery();
    try {
      expect(cells(root, ".query-summary-table tbody td")).toEqual(["Alpha", "2", "Beta", "2"]);
    } finally {
      dispose();
    }
  });

  it("gives every group one cell per requested aggregate", async () => {
    load("{{query (todo TODO)}}", {
      group_by: "state",
      aggregates: [["", "count"], ["cost", "sum"]],
    });
    const { root, dispose } = await mountQuery();
    try {
      expect(cells(root, ".query-summary-table thead th")).toEqual(["State", "Count", "Sum of cost"]);
      expect(cells(root, ".query-summary-table tbody td")).toEqual([
        "TODO", "2", "14",
        "DONE", "2", "Unavailable (non numeric) (2 skipped)",
      ]);
    } finally {
      dispose();
    }
  });

  it("shows per-group counts when a grouping asks for no aggregate", async () => {
    load("{{query (todo TODO)}}", { group_by: "state" });
    const { root, dispose } = await mountQuery();
    try {
      expect(cells(root, ".query-summary-table tbody td")).toEqual(["TODO", "2", "DONE", "2"]);
    } finally {
      dispose();
    }
  });

  it("says that a tags grouping is not a partition", async () => {
    load("{{query (todo TODO)}}", { group_by: "tags", aggregates: [["", "count"]] });
    const { root, dispose } = await mountQuery();
    try {
      expect(root.querySelector(".query-summary-note")?.textContent).toContain(
        "appears in every matching group",
      );
    } finally {
      dispose();
    }
  });

  it("renders no breakdown for an EXPLICIT clear", async () => {
    // `""` is the user's "no grouping"; it is not a field named "".
    load("{{query (todo TODO)}}", { group_by: "", aggregates: [["", "count"]] });
    const { root, dispose } = await mountQuery();
    try {
      expect(root.querySelector(".query-summary-table")).toBeNull();
      expect(cells(root, ".query-summary .qs-value")).toEqual(["4"]);
    } finally {
      dispose();
    }
  });
});

// QBV: exercise the real macro surface, including the guarded clear action.
it.each(["search", "list", "table", "board"] as const)("labels and clears grouping in %s", async (view) => {
  load(`{{query (task TODO)}}\ntine.view:: ${view}\ntine.group-field:: prop:state`, { view, group_by: "prop:state" });
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
  const { root, dispose } = await mountQuery();
  try {
    expect(root.textContent).toContain("Grouped by state");
    root.querySelector<HTMLButtonElement>('[aria-label="Clear grouping"]')!.click();
    await vi.waitFor(() => expect(blockProperty("query", "tine.group-field")).toBe(""));
  } finally { dispose(); }
});

it("shows overall statistics instead of a table for only missing grouping values", async () => {
  load("{{query (task TODO)}}\ntine.view:: search\ntine.group-field:: prop:anchor", { view: "search", group_by: "prop:anchor" });
  const { root, dispose } = await mountQuery();
  try {
    expect(root.querySelector(".query-summary-table")).toBeNull();
    expect(cells(root, ".query-summary .qs-value")).toEqual(["4"]);
    expect(root.textContent).toContain("Grouped by anchor");
  } finally { dispose(); }
});
