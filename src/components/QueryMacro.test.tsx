import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { blockProperty, doc, resetStore, setDoc, undo, type FeedPage, type Node as StoreNode } from "../store";
import { clearSimpleForm, getSimpleForm, stashSimpleForm } from "../editor/queryBuilder";
import type { QueryExecution, RefGroup } from "../types";
import { bumpDataRev } from "../ui";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  clearSimpleForm("query");
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(roots: string[]): FeedPage {
  return {
    name: "Sheet",
    kind: "page",
    title: "Sheet",
    preBlock: null,
    roots,
    format: "md",
    readOnly: false,
    guide: false,
  };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function queryGroups(ids: string[]): RefGroup[] {
  return [
    {
      page: "Sheet",
      kind: "page",
      blocks: ids.map((id) => ({
        id,
        raw: doc.byId[id].raw,
        collapsed: false,
        children: [],
        marker: doc.byId[id].raw.startsWith("TODO") ? "TODO" : undefined,
        properties: [["owner", "Martin"]],
      })),
    },
  ];
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function settleQuery(): Promise<void> {
  await tick();
  await tick();
}

function clickView(root: HTMLElement, label: "Search" | "List" | "Table" | "Board"): void {
  const button = [...root.querySelectorAll(".query-view-switcher button")].find(
    (el) => el.textContent?.trim() === label
  ) as HTMLButtonElement | undefined;
  if (!button) throw new Error(`missing query view button ${label}`);
  button.click();
}

function activeView(root: HTMLElement): string | undefined {
  return root.querySelector(".query-view-switcher button.active")?.textContent?.trim();
}

function presentedResultNumbers(
  root: HTMLElement,
  view: "Search" | "List" | "Table" | "Board"
): number[] {
  const selectors = {
    Search: ".query-search-hit",
    List: '.query-group [data-block-id^="todo-"]',
    Table: '.sheet-title-cell[data-block-id^="todo-"]',
    Board: '.sheet-board-card[data-block-id^="todo-"]',
  } as const;
  return [...root.querySelectorAll(selectors[view])].map((element) => {
    const match = /Result\s+(\d+)/.exec(element.textContent ?? "");
    if (!match) throw new Error(`${view} result did not expose its fixture identity: ${element.textContent}`);
    return Number(match[1]);
  });
}

function loadQueryDoc(queryRaw: string) {
  setDoc({
    byId: {
      query: node("query", queryRaw, null),
      todo: node("todo", "TODO From query\nowner:: Martin", null),
    },
    pages: [page(["query", "todo"])],
    feed: ["Sheet"],
    loaded: true,
  });
  vi.spyOn(backend(), "runQuery").mockResolvedValue(queryGroups(["todo"]));
}

function loadAdvancedQueryDoc(queryRaw: string) {
  setDoc({
    byId: {
      query: node("query", queryRaw, null),
      todo: node("todo", "TODO From query\nowner:: Martin", null),
    },
    pages: [page(["query", "todo"])],
    feed: ["Sheet"],
    loaded: true,
  });
  vi.spyOn(backend(), "runAdvancedQuery").mockResolvedValue({
    groups: queryGroups(["todo"]),
    ran: ["task"],
    ignored: [],
    supported: true,
  });
  vi.spyOn(backend(), "runQuery").mockResolvedValue(queryGroups(["todo"]));
}

describe("QueryMacro sheet integration", () => {
  it("shows bounded ancestor context for list-query hits", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    vi.mocked(backend().runQuery).mockResolvedValue([
      {
        page: "Sheet",
        kind: "page",
        blocks: [{
          id: "todo",
          raw: doc.byId.todo.raw,
          collapsed: false,
          children: [],
          breadcrumb: ["Projects", "Tine"],
        }],
      },
    ]);

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      await vi.waitFor(() => expect(root.querySelector(".ref-breadcrumb")?.textContent ?? "").toContain("Projects"));
      expect(root.querySelectorAll(".ref-breadcrumb")).toHaveLength(1);
    } finally {
      dispose();
    }
  });

  it("retains a local query-tree disclosure across fresh result object identities", async () => {
    setDoc({
      byId: {
        query: node("query", "{{query (task LATER)}}", null),
        "hit-root": node("hit-root", "TODO Query hit", null, ["hit-child"]),
        "hit-child": node("hit-child", "Query child", "hit-root", ["hit-grandchild"]),
        "hit-grandchild": node("hit-grandchild", "Query grandchild", "hit-child"),
      },
      pages: [page(["query", "hit-root"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const freshResult = (): RefGroup[] => [{
      page: "Sheet",
      kind: "page",
      blocks: [{ id: "hit-root", raw: "TODO Query hit", collapsed: false, children: [] }],
    }];
    const runQuery = vi.spyOn(backend(), "runQuery").mockImplementation(async () => freshResult());

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      await vi.waitFor(() => expect(root.textContent).toContain("Query child"));
      expect(root.textContent).not.toContain("Query grandchild");

      root.querySelector<HTMLElement>(
        '[data-block-id="hit-child"] > .block-main .collapse-toggle.has-children',
      )!.click();
      await vi.waitFor(() => expect(root.textContent).toContain("Query grandchild"));

      bumpDataRev();
      await vi.waitFor(() => expect(runQuery).toHaveBeenCalledTimes(2));
      await vi.waitFor(() => expect(root.textContent).toContain("Query grandchild"));
      expect(doc.byId["hit-child"].collapsed).toBe(false);
    } finally {
      dispose();
    }
  });

  it("reopens a materialized friendly search without exposing it as raw DSL", async () => {
    loadQueryDoc('{{query (search "alpha beta")}}\ntine.view:: search');
    const execution: QueryExecution = {
      hits: [{
        entity: "block",
        page: "Sheet",
        kind: "page",
        block: { id: "todo", raw: "TODO From query", collapsed: false, children: [], breadcrumb: [] },
        display_text: "alpha and beta",
        evidence: [{
          clause_id: 1,
          field: "visible_content",
          mode: "contains",
          spans: [{ start: 0, end: 5 }, { start: 10, end: 14 }],
        }],
      }],
      diagnostics: [],
      explanation: { branches: [] },
      cancelled: false,
    };
    const graphSearch = vi.spyOn(backend(), "runGraphSearch").mockResolvedValue(execution);

    const { root, dispose } = mount(() => <Block id="query" />);
    await settleQuery();

    expect(activeView(root)).toBe("Search");
    expect(root.querySelector(".qb-chip")?.textContent).toBe("search: alpha beta");
    expect(root.querySelector(".qb-chip-raw")).toBeNull();
    expect([...root.querySelectorAll("mark")].map((mark) => mark.textContent)).toEqual(["alpha", "beta"]);
    expect(graphSearch).toHaveBeenCalledWith("alpha beta", 500, 5_000, "inline-query:query", false);

    dispose();
  });

  it("keeps ordinary DSL query membership across Search, List, Table, and Board presentations", async () => {
    const ids = Array.from({ length: 9 }, (_, index) => `todo-${index + 1}`);
    setDoc({
      byId: {
        query: node(
          "query",
          "{{query (and (task TODO) (priority A) (not (page Templates)) (sort-by modified desc))}}\ntine.view:: search",
          null
        ),
        ...Object.fromEntries(ids.map((id, index) => [id, node(id, `TODO [#A] Result ${index + 1}`, null)])),
      },
      pages: [page(["query", ...ids])],
      feed: ["Sheet"],
      loaded: true,
    });
    vi.spyOn(backend(), "runQuery").mockResolvedValue(queryGroups(ids));
    const graphSearch = vi.spyOn(backend(), "runGraphSearch");

    const { root, dispose } = mount(() => <Block id="query" />);
    await settleQuery();

    expect(activeView(root)).toBe("Search");
    expect(root.querySelector(".query-count")?.textContent).toBe("9");
    expect(root.querySelectorAll(".query-search-results .query-search-hit")).toHaveLength(9);
    expect(root.querySelector(".query-search-hit")?.textContent).toContain("Result 1");
    expect(presentedResultNumbers(root, "Search")).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(graphSearch).not.toHaveBeenCalled();

    for (const view of ["List", "Table", "Board", "Search"] as const) {
      clickView(root, view);
      await settleQuery();
      expect(activeView(root)).toBe(view);
      expect(root.querySelector(".query-count")?.textContent).toBe("9");
      expect(presentedResultNumbers(root, view)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }
    expect(graphSearch).not.toHaveBeenCalled();

    dispose();
  });

  it("renders the query header and builder above a sheet-faced query exactly once", async () => {
    loadQueryDoc("{{query (todo TODO)}}\ntine.view:: table");

    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();

    expect(root.querySelector(".query-header")).not.toBeNull();
    expect(root.querySelector(".qb-bar")).not.toBeNull();
    expect(root.querySelector(".qb-chip")).not.toBeNull();
    expect(root.querySelectorAll(".sheet-table")).toHaveLength(1);
    expect(root.querySelectorAll(".query-table")).toHaveLength(0);
    expect(root.textContent).toContain("From query");

    dispose();
  });

  it("applies the Sheets formula filter to query-sourced Table and Board faces", async () => {
    setDoc({
      byId: {
        query: node(
          "query",
          "{{query (and (todo TODO) \"score\")}}\ntine.view:: table\ntine.fields:: points=number\ntine.filter:: points > 2",
          null
        ),
        low: node("low", "TODO Low score\npoints:: 1", null),
        high: node("high", "TODO High score\npoints:: 3", null),
      },
      pages: [page(["query", "low", "high"])],
      feed: ["Sheet"],
      loaded: true,
    });
    vi.spyOn(backend(), "runQuery").mockResolvedValue(queryGroups(["low", "high"]));

    const { root, dispose } = mount(() => <Block id="query" />);
    await settleQuery();

    expect(activeView(root)).toBe("Table");
    expect(
      [...root.querySelectorAll(".sheet-title-cell .sheet-cell-body")].map((cell) => cell.textContent?.trim())
    ).toEqual(["High score"]);

    clickView(root, "Board");
    await settleQuery();
    expect(activeView(root)).toBe("Board");
    expect([...root.querySelectorAll(".sheet-board-card-title")].map((card) => card.textContent?.trim())).toEqual([
      "High score",
    ]);
    // View switching must retain the coarse query and the formula refinement.
    expect(blockProperty("query", "tine.filter")).toBe("points > 2");
    expect(backend().runQuery).toHaveBeenCalledWith('(and (todo TODO) "score")');

    dispose();
  });

  it("persists List, Table, and Board through tine.view properties with one undo unit per switch", async () => {
    loadQueryDoc("{{query (todo TODO)}}");
    const originalRaw = doc.byId.query.raw;

    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();

    expect(activeView(root)).toBe("List");

    clickView(root, "Table");
    expect(activeView(root)).toBe("Table");
    expect(blockProperty("query", "tine.view")).toBe("table");
    expect(doc.byId.query.raw).toBe("{{query (todo TODO)}}\ntine.view:: table");
    undo();
    expect(doc.byId.query.raw).toBe(originalRaw);
    expect(activeView(root)).toBe("List");

    clickView(root, "Table");
    clickView(root, "Board");
    expect(activeView(root)).toBe("Board");
    expect(blockProperty("query", "tine.view")).toBe("board");
    expect(blockProperty("query", "tine.group-by")).toBe("state");
    undo();
    expect(blockProperty("query", "tine.view")).toBe("table");
    expect(blockProperty("query", "tine.group-by")).toBeNull();

    clickView(root, "Board");
    clickView(root, "List");
    expect(activeView(root)).toBe("List");
    expect(blockProperty("query", "tine.view")).toBeNull();
    expect(blockProperty("query", "tine.group-by")).toBe("state");
    undo();
    expect(blockProperty("query", "tine.view")).toBe("board");
    expect(blockProperty("query", "tine.group-by")).toBe("state");

    dispose();
  });

  it("does not clobber an existing board grouping when switching to Board", async () => {
    loadQueryDoc("{{query (todo TODO)}}\ntine.group-by:: tags");

    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();

    clickView(root, "Board");

    expect(blockProperty("query", "tine.view")).toBe("board");
    expect(blockProperty("query", "tine.group-by")).toBe("tags");

    dispose();
  });

  it("collapses a query sheet face while keeping the query controls visible", async () => {
    loadQueryDoc("{{query (todo TODO)}}\ntine.view:: table");

    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();
    expect(root.querySelectorAll(".sheet-table")).toHaveLength(1);

    (root.querySelector(".query-collapse") as HTMLElement).click();

    expect(root.querySelector(".query-header")).not.toBeNull();
    expect(root.querySelector(".qb-bar")).not.toBeNull();
    expect(root.querySelectorAll(".sheet-table")).toHaveLength(0);

    dispose();
  });

  it("keeps identical query collapse overrides isolated by block identity", async () => {
    setDoc({
      byId: {
        q1: node("q1", "{{query (todo TODO)}}", null),
        q2: node("q2", "{{query (todo TODO)}}", null),
        todo: node("todo", "TODO From query", null),
      },
      pages: [page(["q1", "q2", "todo"])], feed: ["Sheet"], loaded: true,
    });
    vi.spyOn(backend(), "runQuery").mockResolvedValue(queryGroups(["todo"]));
    const { root, dispose } = mount(() => <><Block id="q1" /><Block id="q2" /></>);
    await settleQuery();
    const toggles = root.querySelectorAll<HTMLElement>(".query-collapse");
    toggles[0].click();
    expect(toggles[0].classList.contains("collapsed")).toBe(true);
    expect(toggles[1].classList.contains("collapsed")).toBe(false);
    dispose();
  });

  it("persists an explicit expanded override over source collapsed true", async () => {
    loadQueryDoc("{{query (todo TODO) {:collapsed? true}}}");
    vi.spyOn(backend(), "runQuery").mockResolvedValue(queryGroups(["todo"]));
    const first = mount(() => <Block id="query" />);
    await settleQuery();
    const toggle = first.root.querySelector(".query-collapse") as HTMLElement;
    expect(toggle.classList.contains("collapsed")).toBe(true);
    toggle.click();
    expect(toggle.classList.contains("collapsed")).toBe(false);
    first.dispose();
    document.body.innerHTML = "";

    const second = mount(() => <Block id="query" />);
    await settleQuery();
    expect((second.root.querySelector(".query-collapse") as HTMLElement).classList.contains("collapsed")).toBe(false);
    second.dispose();
  });

  it("keeps legacy :table-view? rendering read-only when no tine.view is set", async () => {
    loadQueryDoc("{{query (todo TODO) {:table-view? true}}}");

    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();

    expect(activeView(root)).toBe("List");
    expect(blockProperty("query", "tine.view")).toBeNull();
    expect(root.querySelectorAll(".query-table")).toHaveLength(1);
    expect(root.querySelectorAll(".sheet-table")).toHaveLength(0);

    dispose();
  });

  it("shows an enabled Simple toggle for stashed advanced queries and restores the builder", async () => {
    const simpleDsl = "(and (task TODO) (sort-by priority desc))";
    stashSimpleForm("query", simpleDsl);
    loadAdvancedQueryDoc('{{query [:find (pull ?b [*]) :where (task ?b "TODO")]}}');

    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();

    const button = [...root.querySelectorAll("button")].find(
      (el) => el.textContent?.trim() === "← Simple"
    ) as HTMLButtonElement | undefined;
    expect(button).not.toBeUndefined();
    expect(button!.disabled).toBe(false);
    expect(root.querySelector(".qb-bar")).toBeNull();

    button!.click();
    await settleQuery();

    expect(doc.byId.query.raw).toBe(`{{query ${simpleDsl}}}`);
    expect(getSimpleForm("query")).toBeUndefined();
    expect(root.querySelector(".qb-bar")).not.toBeNull();

    dispose();
  });
});
