import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { blockProperty, doc, resetStore, setDoc, setBlockProperty, setRaw, undo, type FeedPage, type Node as StoreNode } from "../store";
import { route } from "../router";
import type { QueryExecution, QueryHit, RefGroup } from "../types";
import type { QueryReport, QueryResult } from "../editor/queryIr";
import { bumpDataRev, bumpGraphEpoch, setWorkflow } from "../ui";
import { queryMacroExtent } from "../editor/queryMacro";
import { backendReadsQueries } from "../queryReadingsTestkit";
import { searchFilter } from "../editor/queryBuilder";
import { editingId, endEdit, startEditing } from "../editorController";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setWorkflow("now");
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

/** What `query_run` answers for a block-anchored query (§7.1). Execution goes
 *  through the ONE evaluator now — `run_query` and `run_advanced_query` cannot
 *  read TQL and are no longer on the render path — so this is what every result
 *  in this file is mocked as. `report` is the advanced ran/ignored answer, which
 *  now rides on the result rather than on a second command (M5). */
function blockResult(groups: RefGroup[], report?: Partial<QueryReport>): QueryResult {
  return {
    anchor: "block",
    groups,
    diagnostics: [],
    report: { ran: [], ignored: [], supported: true, ...report },
    total: groups.reduce((sum, group) => sum + group.blocks.length, 0),
    exceeded: false,
  };
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function settleQuery(): Promise<void> {
  await tick();
  await tick();
}

/** The Display panel, opened. A builder-backed query states its view there now
 *  — the header switcher would be a second control writing the same key — so
 *  the helper opens the sheet and the panel, in that order, exactly as a user
 *  would. Hosts with no builder (an advanced query, a friendly search) keep the
 *  header switcher, which is why both branches live here. */
async function openDisplay(root: HTMLElement): Promise<HTMLElement> {
  const gear = await vi.waitFor(() => {
    const found = root.querySelector<HTMLButtonElement>(".qs-gear");
    if (!found) throw new Error("the query sentence never appeared");
    return found;
  });
  if (!document.querySelector(".qs-sheet")) gear.click();
  const trigger = await vi.waitFor(() => {
    const found = document.querySelector<HTMLButtonElement>(".qd-trigger");
    if (!found) throw new Error("the Display control never appeared");
    return found;
  });
  if (!document.querySelector(".qd-panel")) trigger.click();
  return await vi.waitFor(() => {
    const panel = document.querySelector<HTMLElement>(".qd-panel");
    if (!panel) throw new Error("the Display panel never opened");
    return panel;
  });
}

async function clickView(
  root: HTMLElement,
  label: "Search" | "List" | "Table" | "Board",
): Promise<void> {
  const legacy = [...root.querySelectorAll(".query-view-switcher button")].find(
    (el) => el.textContent?.trim() === label
  ) as HTMLButtonElement | undefined;
  if (legacy) {
    legacy.click();
    return;
  }
  const panel = await openDisplay(root);
  const button = [...panel.querySelectorAll(".qd-view")].find(
    (el) => el.textContent?.trim() === label
  ) as HTMLButtonElement | undefined;
  if (!button) throw new Error(`missing query view button ${label}`);
  button.click();
}

async function activeView(root: HTMLElement): Promise<string | undefined> {
  const legacy = root.querySelector(".query-view-switcher button.active");
  if (legacy) return legacy.textContent?.trim();
  const panel = await openDisplay(root);
  return panel.querySelector(".qd-view.active")?.textContent?.trim();
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
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockResult(queryGroups(["todo"])));
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
  // The advanced ran/ignored answer now rides on the run's own report (M5).
  vi.spyOn(backend(), "queryRun").mockResolvedValue(
    blockResult(queryGroups(["todo"]), { ran: ["task"], ignored: [], supported: true }),
  );
  // Whether a `{{query …}}` holds datalog is the ENGINE's reading, not a regex
  // over the text (§7.1) — so the test says the engine read datalog.
  const argument = queryMacroExtent(queryRaw)?.argument ?? "";
  backendReadsQueries({ [argument]: { form: argument, kind: "advanced" } });
}

describe("QueryMacro sheet integration", () => {
  it("keeps a completed task mounted for the two-second UI grace, then removes the departed row coherently", async () => {
    setWorkflow("todo");
    loadQueryDoc("{{query (task TODO)}}");
    let ids = ["todo"];
    const run = vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockResult(queryGroups(ids)));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector('[data-block-id="todo"]')).not.toBeNull());
      const row = root.querySelector<HTMLElement>('[data-block-id="todo"]')!;
      const checkbox = row.querySelector<HTMLElement>(".block-task-checkbox")!;

      ids = [];
      checkbox.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      expect(doc.byId.todo.raw).toMatch(/^DONE /);
      expect(row.querySelector(".block-marker")?.textContent).toBe("DONE");
      bumpDataRev();

      await tick();
      expect(run).toHaveBeenCalledTimes(1);
      expect(root.querySelector('[data-block-id="todo"]')).toBe(row);
      expect(root.querySelector(".query-count")?.textContent).toBe("1");

      await new Promise((resolve) => setTimeout(resolve, 2_050));
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));
      await vi.waitFor(() => expect(root.querySelector('[data-block-id="todo"]')).toBeNull());
      expect(root.querySelector(".query-count")?.textContent).toBe("0");

      // A later projection/change notification is a fresh demand, not part of
      // the expired hold, so the coherent row can return without another wait.
      undo();
      ids = ["todo"];
      bumpDataRev();
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(3));
      await vi.waitFor(() => expect(root.querySelector('[data-block-id="todo"]')).not.toBeNull());
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
    } finally {
      dispose();
    }
  });

  it("keeps the live DONE edit when an early committed query still returns the older TODO row", async () => {
    setWorkflow("todo");
    loadQueryDoc("{{query (task TODO)}}");
    const older = blockResult(queryGroups(["todo"]));
    let answer = older;
    const run = vi.spyOn(backend(), "queryRun").mockImplementation(async () => answer);
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const row = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>('[data-block-id="todo"]');
        expect(found).not.toBeNull();
        return found!;
      });
      row.querySelector<HTMLElement>(".block-task-checkbox")!
        .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      bumpDataRev();
      expect(doc.byId.todo.raw).toMatch(/^DONE /);
      await new Promise((resolve) => setTimeout(resolve, 2_050));
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));
      expect(root.querySelector('[data-block-id="todo"]')).toBe(row);
      expect(row.querySelector(".block-marker")?.textContent).toBe("DONE");
      expect(doc.byId.todo.raw).toMatch(/^DONE /);
      expect(root.querySelector(".query-count")?.textContent).toBe("1");

      // Later ordinary projection progress updates membership without rolling
      // back the independently live editor/source object in the meantime.
      answer = blockResult(queryGroups([]));
      bumpDataRev();
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(3));
      await vi.waitFor(() => expect(root.querySelector('[data-block-id="todo"]')).toBeNull());
      expect(root.querySelector(".query-count")?.textContent).toBe("0");
      expect(doc.byId.todo.raw).toMatch(/^DONE /);
    } finally {
      dispose();
    }
  });

  it("coalesces checkbox reversal and Undo into the latest grace refresh", async () => {
    setWorkflow("todo");
    loadQueryDoc("{{query (task TODO)}}");
    let ids = ["todo"];
    const run = vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockResult(queryGroups(ids)));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const row = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>('[data-block-id="todo"]');
        expect(found).not.toBeNull();
        return found!;
      });
      const toggle = () => row.querySelector<HTMLElement>(".block-task-checkbox")!
        .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));

      ids = [];
      toggle();
      bumpDataRev();
      expect(doc.byId.todo.raw).toMatch(/^DONE /);
      ids = ["todo"];
      toggle();
      bumpDataRev();
      expect(doc.byId.todo.raw).toMatch(/^TODO /);
      ids = [];
      toggle();
      bumpDataRev();
      expect(doc.byId.todo.raw).toMatch(/^DONE /);
      undo();
      ids = ["todo"];
      bumpDataRev();
      expect(doc.byId.todo.raw).toMatch(/^TODO /);

      await tick();
      expect(run).toHaveBeenCalledTimes(1);
      expect(root.querySelector('[data-block-id="todo"]')).toBe(row);
      await new Promise((resolve) => setTimeout(resolve, 2_050));
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));
      expect(root.querySelector('[data-block-id="todo"]')).toBe(row);
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
    } finally {
      dispose();
    }
  });

  it("keeps a focused result editor and its coherent snapshot until editing ends", async () => {
    const form = "(task TODO) (sort-by page asc)";
    setDoc({
      byId: {
        query: node("query", `{{query ${form}}}`, null),
        "hit-a": node("hit-a", "TODO First\nMultiline detail", null),
        "hit-b": node("hit-b", "TODO Second", null),
      },
      pages: [page(["query", "hit-a", "hit-b"])],
      feed: ["Sheet"],
      loaded: true,
    });
    backendReadsQueries({ [form]: { form, view: { sort: [["page", "asc"]] } } });
    let finishRefresh!: (value: QueryResult) => void;
    const run = vi.spyOn(backend(), "queryRun")
      .mockResolvedValueOnce(blockResult(queryGroups(["hit-a", "hit-b"])))
      .mockImplementationOnce(() => new Promise<QueryResult>((resolve) => { finishRefresh = resolve; }));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const row = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>('[data-block-id="hit-a"]');
        expect(found).not.toBeNull();
        return found!;
      });
      bumpDataRev();
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));

      const content = row.querySelector<HTMLElement>(".block-content")!;
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      const editor = await vi.waitFor(() => {
        const found = row.querySelector<HTMLTextAreaElement>("textarea.block-editor");
        expect(found).not.toBeNull();
        return found!;
      });
      editor.focus();
      editor.setSelectionRange(5, 5);
      setRaw("hit-a", "TODO First edited\nMultiline detail");
      expect(doc.byId["hit-a"].raw).toContain("First edited");
      expect(editingId()).toBe("hit-a");

      finishRefresh(blockResult(queryGroups(["hit-b"])));
      await tick();
      expect(row.isConnected).toBe(true);
      expect(row.querySelector("textarea.block-editor")).toBe(editor);
      expect(root.querySelector(".query-count")?.textContent).toBe("2");

      endEdit("blur");
      await vi.waitFor(() => expect(root.querySelector('[data-block-id="hit-a"]')).toBeNull());
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
    } finally {
      dispose();
    }
  });

  it("keeps sorted result nodes mounted when another result disappears", async () => {
    const form = "(task TODO) (sort-by page asc)";
    const hitIds = Array.from({ length: 30 }, (_, index) => `hit-${index + 1}`);
    setDoc({
      byId: {
        query: node("query", `{{query ${form}}}`, null),
        ...Object.fromEntries(hitIds.map((id, index) => [
          id,
          node(id, `TODO Result ${index + 1}\nMultiline detail ${index + 1}`, null),
        ])),
      },
      pages: [page(["query", ...hitIds])],
      feed: ["Sheet"], loaded: true,
    });
    backendReadsQueries({ [form]: { form, view: { sort: [["page", "asc"]] } } });
    let ids = [...hitIds];
    const run = vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockResult(queryGroups(ids)));
    const { root, dispose } = mount(() => <Block id="query" />);
    const result = (id: string) => root.querySelector(`.query-group [data-block-id="${id}"]`);
    try {
      await vi.waitFor(() => expect(result("hit-30")).not.toBeNull());
      const first = result("hit-1")!;
      const last = result("hit-30")!;
      expect(root.querySelectorAll(".query-crumb")).toHaveLength(1);
      ids = ids.filter((id) => id !== "hit-2");
      bumpDataRev();
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));
      await vi.waitFor(() => expect(result("hit-2")).toBeNull());
      expect(first.isConnected).toBe(true);
      expect(result("hit-1")).toBe(first);
      expect(result("hit-30")).toBe(last);
      // Removing the first member must not just transfer the unstable group
      // key to the next member and recreate the rest of the page again.
      ids = ids.filter((id) => id !== "hit-1");
      bumpDataRev();
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(3));
      await vi.waitFor(() => expect(result("hit-1")).toBeNull());
      expect(result("hit-30")).toBe(last);
      expect(last.isConnected).toBe(true);
      expect(root.querySelectorAll(".query-crumb")).toHaveLength(1);
    } finally { dispose(); }
  });

  it("shows bounded ancestor context for list-query hits", async () => {
    setDoc({
      byId: {
        query: node("query", "{{query (task TODO)}}", null),
        projects: node("projects", "Projects", null, ["tine"]),
        tine: node("tine", "Tine", "projects", ["todo"]),
        todo: node("todo", "TODO From query\nowner:: Martin", "tine"),
      },
      pages: [page(["query", "projects"])],
      feed: ["Sheet"],
      loaded: true,
    });
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockResult([
      {
        page: "Sheet",
        kind: "page",
        blocks: [{
          id: "todo",
          raw: doc.byId.todo.raw,
          collapsed: false,
          children: [],
        }],
      },
    ]));

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
    const runQuery = vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockResult(freshResult()));

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
    // What a `(search …)` form MEANS is the engine's answer, not a regex here:
    // the chip is friendly because the IR carries a `content match` leaf.
    backendReadsQueries({
      '(search "alpha beta")': { form: '(search "alpha beta")', filter: searchFilter("alpha beta") },
    });
    const execution: QueryExecution = {
      hits: [{
        entity: "block",
        page: "Sheet",
        kind: "page",
        block: {
          id: "todo",
          raw: "TODO From query\nid:: todo-authored",
          collapsed: false,
          children: [],
          breadcrumb: [],
          properties: [["id", "todo-authored"]],
        },
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

    expect(await activeView(root)).toBe("Search");
    // The resting SENTENCE says it, and says it as words plus one soft value —
    // not as the DSL text the block happens to hold.
    expect(root.querySelector(".qs-sentence")?.textContent).toBe("Blocks where search: alpha beta");
    expect(root.querySelector(".qs-seg-value")?.textContent).toBe("alpha beta");
    expect(root.querySelector(".qs-seg-advanced")).toBeNull();
    expect([...root.querySelectorAll("mark")].map((mark) => mark.textContent)).toEqual(["alpha", "beta"]);
    // An inline Friendly search is a whole-graph question, so it sends NO
    // physical page scope — and it carries both families' resolved Display
    // settings beside it (§7.6, Q3).
    expect(graphSearch).toHaveBeenCalledWith(
      "alpha beta", 500, 5_000, "inline-query:query", false, undefined,
      { pageView: { view: "search" }, blockView: { view: "search" } },
    );
    root.querySelector<HTMLButtonElement>(".query-search-hit")!.click();
    expect(route()).toMatchObject({ kind: "page", name: "Sheet", pageKind: "page" });

    dispose();
  });

  it("keeps friendly-search hits and count coherent while their live block is being edited", async () => {
    setDoc({
      byId: {
        query: node("query", '{{query (search "Result")}}\ntine.view:: search', null),
        "hit-a": node("hit-a", "TODO Result A\nMultiline edit", null),
        "hit-b": node("hit-b", "TODO Result B", null),
      },
      pages: [page(["query", "hit-a", "hit-b"])],
      feed: ["Sheet"],
      loaded: true,
    });
    backendReadsQueries({
      '(search "Result")': { form: '(search "Result")', filter: searchFilter("Result") },
    });
    const hit = (id: "hit-a" | "hit-b"): QueryHit => ({
      entity: "block",
      page: "Sheet",
      kind: "page",
      block: {
        id,
        raw: doc.byId[id].raw,
        collapsed: false,
        children: [],
        breadcrumb: [],
        properties: [],
      },
      display_text: doc.byId[id].raw,
      evidence: [],
    });
    const execution = (ids: ("hit-a" | "hit-b")[]): QueryExecution => ({
      hits: ids.map(hit),
      diagnostics: [],
      explanation: { branches: [] },
      cancelled: false,
    });
    let finishRefresh!: (value: QueryExecution) => void;
    const graphSearch = vi.spyOn(backend(), "runGraphSearch")
      .mockResolvedValueOnce(execution(["hit-a", "hit-b"]))
      .mockImplementationOnce(() => new Promise((resolve) => { finishRefresh = resolve; }));

    const { root, dispose } = mount(() => <><Block id="query" /><Block id="hit-a" /></>);
    try {
      await vi.waitFor(() => expect(root.querySelectorAll(".query-search-hit")).toHaveLength(2));
      const sourceRow = root.querySelector<HTMLElement>('.ls-block[data-block-id="hit-a"]')!;
      sourceRow.querySelector<HTMLElement>(".block-content")!
        .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      await vi.waitFor(() => expect(editingId()).toBe("hit-a"));

      bumpDataRev();
      await vi.waitFor(() => expect(graphSearch).toHaveBeenCalledTimes(2));
      finishRefresh(execution(["hit-b"]));
      await tick();

      expect(root.querySelectorAll(".query-search-hit")).toHaveLength(2);
      expect(root.querySelector(".query-count")?.textContent).toBe("2");
      expect(root.querySelector(".query-search-results")?.textContent).toContain("Result A");
      expect(sourceRow.querySelector("textarea.block-editor")).not.toBeNull();

      endEdit("blur");
      await vi.waitFor(() => expect(root.querySelectorAll(".query-search-hit")).toHaveLength(1));
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
      expect(root.querySelector(".query-search-results")?.textContent).not.toContain("Result A");
    } finally {
      dispose();
    }
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
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockResult(queryGroups(ids)));
    const graphSearch = vi.spyOn(backend(), "runGraphSearch");

    const { root, dispose } = mount(() => <Block id="query" />);
    await settleQuery();

    expect(await activeView(root)).toBe("Search");
    expect(root.querySelector(".query-count")?.textContent).toBe("9");
    expect(root.querySelectorAll(".query-search-results .query-search-hit")).toHaveLength(9);
    expect(root.querySelector(".query-search-hit")?.textContent).toContain("Result 1");
    expect(presentedResultNumbers(root, "Search")).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(graphSearch).not.toHaveBeenCalled();

    for (const view of ["List", "Table", "Board", "Search"] as const) {
      await clickView(root, view);
      await settleQuery();
      expect(await activeView(root)).toBe(view);
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
    expect(root.querySelector(".qs-line")).not.toBeNull();
    // This block's text is mocked as unparsed, so the sentence reads it back as
    // the retained leaf it is — still one line, still not a chip bar.
    expect(root.querySelector(".qs-sentence")?.textContent).toBe("Blocks where (todo TODO)");
    // Exactly once: one sentence, one gear, one count — not one per face.
    expect(root.querySelectorAll(".qs-sentence")).toHaveLength(1);
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
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockResult(queryGroups(["low", "high"])));

    const { root, dispose } = mount(() => <Block id="query" />);
    await settleQuery();

    expect(await activeView(root)).toBe("Table");
    expect(
      [...root.querySelectorAll(".sheet-title-cell .sheet-cell-body")].map((cell) => cell.textContent?.trim())
    ).toEqual(["High score"]);

    await clickView(root, "Board");
    await settleQuery();
    expect(await activeView(root)).toBe("Board");
    expect([...root.querySelectorAll(".sheet-board-card-title")].map((card) => card.textContent?.trim())).toEqual([
      "High score",
    ]);
    // View switching must retain the coarse query and the formula refinement.
    expect(blockProperty("query", "tine.filter")).toBe("points > 2");
    // Execution goes through the ONE evaluator, carrying the query the engine
    // read — not a string the frontend re-printed (I-12).
    expect(vi.mocked(backend().queryRun).mock.calls[0][0].source).toMatchObject({
      kind: "og",
      original: '(and (todo TODO) "score")',
    });

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

    expect(await activeView(root)).toBe("List");

    await clickView(root, "Table");
    expect(await activeView(root)).toBe("Table");
    expect(blockProperty("query", "tine.view")).toBe("table");
    expect(doc.byId.query.raw).toBe("{{query (todo TODO)}}\ntine.view:: table");
    undo();
    expect(doc.byId.query.raw).toBe(originalRaw);
    expect(await activeView(root)).toBe("List");

    await clickView(root, "Table");
    await clickView(root, "Board");
    expect(await activeView(root)).toBe("Board");
    expect(blockProperty("query", "tine.view")).toBe("board");
    // The Board's default grouping is written under the QUERY-owned key, whose
    // value is a canonical field id — so `state` here is the task marker and
    // could not be mistaken for an ordinary property of the same name (P5B).
    expect(blockProperty("query", "tine.group-field")).toBe("state");
    expect(blockProperty("query", "tine.group-by")).toBeNull();
    undo();
    expect(blockProperty("query", "tine.view")).toBe("table");
    expect(blockProperty("query", "tine.group-field")).toBeNull();

    await clickView(root, "Board");
    await clickView(root, "List");
    expect(await activeView(root)).toBe("List");
    expect(blockProperty("query", "tine.view")).toBeNull();
    expect(blockProperty("query", "tine.group-field")).toBe("state");
    undo();
    expect(blockProperty("query", "tine.view")).toBe("board");
    expect(blockProperty("query", "tine.group-field")).toBe("state");

    dispose();
  });

  // Found by `scripts/e2e-query-display.mjs` on real WebKit, where a press is a
  // POINTER sequence and not a bare `click()`: the panel is portalled to <body>,
  // the sheet's outside-pointer check looked for an open popover UNDER its own
  // element, and so every press in the panel read as a press outside the sheet.
  // The sheet closed, the panel went with it, and the control's own click never
  // landed — the whole panel was unusable with a real pointer while every
  // `click()`-driven test passed.
  it("holds the sheet still under a press inside the portalled Display panel", async () => {
    loadQueryDoc("{{query (todo TODO)}}");
    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();
    const panel = await openDisplay(root);

    const board = [...panel.querySelectorAll<HTMLButtonElement>(".qd-view")].find(
      (el) => el.textContent?.trim() === "Board",
    )!;
    // The press first, exactly as a pointer delivers it, and only then the
    // click: the bug was that nothing survived in between.
    for (const type of ["pointerdown", "mousedown"] as const) {
      board.dispatchEvent(new MouseEvent(type, { bubbles: true, composed: true }));
    }
    expect(document.querySelector(".qs-sheet")).not.toBeNull();
    expect(document.querySelector(".qd-panel")).not.toBeNull();

    board.click();
    await vi.waitFor(() => expect(blockProperty("query", "tine.view")).toBe("board"));
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

    await clickView(root, "Board");

    expect(blockProperty("query", "tine.view")).toBe("board");
    // A legacy key that already answers is a STATEMENT, so the Board default
    // does not speak over it — `state` is nowhere here.
    //
    // What the switch DOES do is pin the meaning the block had. A bare
    // `tine.group-by:: tags` on a LIST is the ordinary property `tags`, which is
    // what the list grouper has always read; the same token on a Board would be
    // the tags facet. So the switch writes the list reading canonically and
    // retires the ambiguous key, rather than letting the new view silently
    // reinterpret it.
    expect(blockProperty("query", "tine.group-field")).toBe("prop:tags");
    expect(blockProperty("query", "tine.group-by")).toBeNull();

    dispose();
  });

  it("switching to Board leaves an explicit no-grouping alone", async () => {
    // A PRESENT but empty `tine.group-field` is the user saying "no grouping".
    // The Board default only fills the silence, so it must not speak over this.
    loadQueryDoc("{{query (todo TODO)}}\ntine.group-field:: ");

    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();

    await clickView(root, "Board");

    expect(blockProperty("query", "tine.view")).toBe("board");
    expect(blockProperty("query", "tine.group-field")).toBe("");
    dispose();
  });

  it("keeps the task-marker default on a board whose grouping nothing states", async () => {
    // ADR 0030, kept alive across the P5B grouping split. A note authored as
    // `tine.view:: board` with no grouping ANYWHERE has always shown a
    // task-marker board — the default fills the silence. `unset` and an explicit
    // clear are two different answers, and only the clear is one ungrouped
    // column; collapsing them would silently un-group every existing board.
    setDoc({
      byId: {
        query: node("query", "{{query (todo TODO)}}\ntine.view:: board", null),
        todo: node("todo", "TODO From query\nowner:: Martin", null),
      },
      pages: [page(["query", "todo"])],
      feed: ["Sheet"],
      loaded: true,
    });
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockResult(queryGroups(["todo"])));

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();

      const headings = await vi.waitFor(() => {
        const found = [...root.querySelectorAll(".sheet-board-header span:first-child")].map((el) =>
          el.textContent?.trim(),
        );
        if (!found.length) throw new Error("the board never rendered");
        return found;
      });
      expect(headings).toContain("TODO");
      expect(headings).not.toContain("All results");
      // Reading is not writing: the default is applied by the renderer, and the
      // note gains no property from being looked at (I-4).
      expect(blockProperty("query", "tine.group-field")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("opens Display without the filter sheet and reads its registry only on demand", async () => {
    bumpGraphEpoch();
    loadQueryDoc("{{query (todo TODO)}}");
    const registry = vi.spyOn(backend(), "queryRegistry");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      expect(document.querySelector('.qs-sheet[aria-label="Query filter"]')).toBeNull();
      expect(registry).not.toHaveBeenCalled();
      const trigger = root.querySelector<HTMLButtonElement>(".qd-trigger");
      expect(trigger).not.toBeNull();
      trigger!.click();
      await vi.waitFor(() => expect(document.querySelector(".qd-panel")).not.toBeNull());
      await vi.waitFor(() => expect(registry).toHaveBeenCalledTimes(1));
      expect(document.querySelector('.qs-sheet[aria-label="Query filter"]')).toBeNull();
    } finally { dispose(); }
  });

  it("does not undo a display edit with the next click made before the re-parse", async () => {
    // FAIL-BEFORE (I-20): every display surface renders from the ENGINE's last
    // reading, and the engine re-reads asynchronously. Two clicks inside one
    // parse round-trip therefore both start from the reading that predates the
    // first — and a write set computed against the block's properties from that
    // stale reading restates the fact the first click just changed, undoing it.
    //
    // Here: clear the grouping, then switch to Board without waiting. The
    // switch's untouched grouping is the pre-clear one, and the save baseline
    // called that a disagreement with the empty `tine.group-field` and wrote the
    // old grouping straight back.
    loadQueryDoc("{{query (todo TODO)}}\ntine.group-by:: state");

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const panel = await openDisplay(root);

      const clear = [...panel.querySelectorAll<HTMLButtonElement>(".qd-row-btn")].find(
        (button) => button.textContent?.trim() === "None",
      )!;
      const board = [...panel.querySelectorAll<HTMLButtonElement>(".qd-view")].find(
        (button) => button.textContent?.trim() === "Board",
      )!;
      // Two clicks, no await between them — the parse cannot have answered.
      clear.click();
      board.click();

      expect(blockProperty("query", "tine.view")).toBe("board");
      // The explicit clear survives, and the ambiguous legacy key stays retired.
      expect(blockProperty("query", "tine.group-field")).toBe("");
      expect(blockProperty("query", "tine.group-by")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("keeps both aggregate additions made before the re-parse", async () => {
    loadQueryDoc("{{query (todo TODO)}}");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const panel = await openDisplay(root);
      const add = [...panel.querySelectorAll<HTMLButtonElement>("button")].find(
        (button) => button.textContent?.trim() === "+ count",
      )!;
      add.click();
      add.click();
      expect(blockProperty("query", "tine.col-aggregates")).toBe("count;count");
    } finally { dispose(); }
  });

  it("shows the sheet-only aggregate segments it retains, and keeps them on an edit", async () => {
    // FAIL-BEFORE: `tine.col-aggregates` is shared ground (contract §5). The
    // save merges rather than rewrites, so a table-only `estimate=median`
    // survived — but no surface said so, and the panel that DOES list the
    // aggregates listed only the three the query reader owns. Retention the
    // author cannot see is indistinguishable from loss.
    loadQueryDoc("{{query (todo TODO)}}\ntine.col-aggregates:: estimate=median");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const panel = await openDisplay(root);
      // Read from the block's OWN bytes: the query reader never returns this
      // segment, so no reading of the view could have produced it.
      expect(panel.querySelector(".qd-retained")?.textContent).toContain("estimate=median");

      const add = [...panel.querySelectorAll<HTMLButtonElement>("button")].find(
        (button) => button.textContent?.trim() === "+ count",
      )!;
      add.click();
      // The unrelated edit appends; the retained segment keeps its text and its
      // place, and the panel still says it is there.
      expect(blockProperty("query", "tine.col-aggregates")).toBe("estimate=median;count");
      await vi.waitFor(() =>
        expect(document.querySelector(".qd-retained")?.textContent).toContain("estimate=median"),
      );
    } finally { dispose(); }
  });

  it("preserves a grouping clear when a sample save starts before the re-parse", async () => {
    loadQueryDoc("{{query (todo TODO)}}\ntine.group-by:: state");
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(todo TODO)");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const panel = await openDisplay(root);
      const clear = [...panel.querySelectorAll<HTMLButtonElement>(".qd-row-btn")].find(
        (button) => button.textContent?.trim() === "None",
      )!;
      const sample = panel.querySelector<HTMLInputElement>(".qd-sample")!;
      clear.click();
      sample.value = "2";
      sample.dispatchEvent(new Event("input", { bubbles: true }));
      sample.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await vi.waitFor(() => expect(blockProperty("query", "tine.sample")).toBe("2"));
      expect(blockProperty("query", "tine.group-field")).toBe("");
      expect(blockProperty("query", "tine.group-by")).toBeNull();
    } finally { dispose(); }
  });

  it.each(["property", "graph"])("does not overwrite a %s change while the printer is pending", async (change) => {
    loadQueryDoc("{{query (todo TODO)}}");
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    let finish!: (text: string) => void;
    const printed = new Promise<string>((resolve) => { finish = resolve; });
    const printer = vi.spyOn(backend(), "printQuery").mockReturnValue(printed);
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const panel = await openDisplay(root);
      const sample = panel.querySelector<HTMLInputElement>(".qd-sample")!;
      sample.value = "2";
      sample.dispatchEvent(new Event("input", { bubbles: true }));
      sample.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await vi.waitFor(() => expect(printer).toHaveBeenCalled());
      if (change === "property") setBlockProperty("query", "owner", "Later edit");
      else bumpGraphEpoch();
      finish("(todo TODO)");
      await vi.waitFor(() => expect(root.querySelector(".query-print-refused")?.textContent).toContain("changed while saving"));
      if (change === "property") expect(blockProperty("query", "owner")).toBe("Later edit");
      expect(blockProperty("query", "tine.sample")).toBeNull();
    } finally { finish("(todo TODO)"); dispose(); }
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
    expect(root.querySelector(".qs-line")).not.toBeNull();
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
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockResult(queryGroups(["todo"])));
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
    backendReadsQueries({ "(todo TODO) {:collapsed? true}": { form: "(todo TODO)", opts: "{:collapsed? true}" } });
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockResult(queryGroups(["todo"])));
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
    backendReadsQueries({ "(todo TODO) {:table-view? true}": { form: "(todo TODO)", opts: "{:table-view? true}" } });

    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();

    expect(await activeView(root)).toBe("List");
    expect(blockProperty("query", "tine.view")).toBeNull();
    expect(root.querySelectorAll(".query-table")).toHaveLength(1);
    expect(root.querySelectorAll(".sheet-table")).toHaveLength(0);

    dispose();
  });

  // The `⚙ advanced` / `← Simple` pair is gone with the frontend's datalog
  // converters (§9 P0-ts). What must still hold is that an authored advanced
  // query renders its ran/ignored note and does NOT show the chip bar — the
  // builder edits a filter, and converting authored datalog into one is out of
  // scope (§4.3.1, Q13).
  it("renders an advanced query's report without offering the filter builder", async () => {
    loadAdvancedQueryDoc('{{query [:find (pull ?b [*]) :where (task ?b "TODO")]}}');

    const { root, dispose } = mount(() => (
      <>
        <Block id="query" />
        <ContextMenu />
      </>
    ));
    await settleQuery();

    await vi.waitFor(() => expect(root.querySelector(".query-adv-note")?.textContent ?? "").toContain("ran: task"));
    // An advanced (datalog) query has no sentence and no sheet to offer.
    expect(root.querySelector(".qs-line")).toBeNull();
    expect(root.querySelector(".qs-sheet")).toBeNull();
    // …and the count stays in the header, where it has always been.
    expect(root.querySelector(".query-header .query-count")).not.toBeNull();
    expect([...root.querySelectorAll("button")].some((el) => el.textContent?.trim() === "← Simple")).toBe(false);

    dispose();
  });
});

// GH #469. `{{query "xyz"}}` matched its own block, because the block's own text
// contains `xyz` — so the query listed the page it lives on, which renders the
// query again, which lists the page again. OG removes exactly the host block
// from every result set for this reason, and says so at
// frontend/components/query/result.cljs (6e7afa8e): "exclude the current one,
// otherwise it'll loop forever".
describe("a query never returns its own block (GH #469)", () => {
  function loadSelfMatching(queryRaw: string) {
    setDoc({
      byId: {
        query: node("query", queryRaw, null),
        todo: node("todo", "TODO From query\nowner:: Martin", null),
      },
      pages: [page(["query", "todo"])],
      feed: ["Sheet"],
      loaded: true,
    });
  }

  it("drops the host block from a simple DSL query's results", async () => {
    loadSelfMatching('{{query "From query"}}\ntine.view:: list');
    // The backend answers honestly: the host block's own text matches too.
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockResult(queryGroups(["query", "todo"])));

    const { root, dispose } = mount(() => <Block id="query" />);
    await settleQuery();

    const listed = [...root.querySelectorAll(".query-group [data-block-id]")]
      .map((el) => el.getAttribute("data-block-id"));
    expect(listed).toContain("todo");
    expect(listed).not.toContain("query");
    // The count the user reads must agree with what is shown.
    expect(root.querySelector(".query-count")?.textContent).toBe("1");
    dispose();
  });

  it("drops the host block from an advanced query's results", async () => {
    loadSelfMatching('{{query {:query [:find (pull ?b [*]) :where [?b :block/content "x"]]}}}\ntine.view:: list');
    // A form that is ITSELF one map stays whole: `split_trailing_map` only splits
    // a map that FOLLOWS a nonempty form (§4.3.1).
    backendReadsQueries({
      '{:query [:find (pull ?b [*]) :where [?b :block/content "x"]]}': {
        form: '{:query [:find (pull ?b [*]) :where [?b :block/content "x"]]}',
        kind: "advanced",
      },
    });
    vi.spyOn(backend(), "queryRun").mockResolvedValue(
      blockResult(queryGroups(["query", "todo"]), { ran: ["content"] }),
    );

    const { root, dispose } = mount(() => <Block id="query" />);
    await settleQuery();

    const listed = [...root.querySelectorAll(".query-group [data-block-id]")]
      .map((el) => el.getAttribute("data-block-id"));
    expect(listed).toContain("todo");
    expect(listed).not.toContain("query");
    dispose();
  });

  it("drops the host block from a full-text search query's hits", async () => {
    loadSelfMatching('{{query (search "From query")}}\ntine.view:: search');
    const hit = (id: string, raw: string): QueryHit => ({
      entity: "block" as const,
      page: "Sheet",
      kind: "page" as const,
      block: { id, raw, collapsed: false, children: [], breadcrumb: [], properties: [] },
      display_text: raw,
      evidence: [{ clause_id: 1, field: "visible_content" as const, mode: "contains" as const, spans: [{ start: 0, end: 4 }] }],
    });
    const execution: QueryExecution = {
      hits: [hit("query", '{{query (search "From query")}}'), hit("todo", "TODO From query")],
      diagnostics: [],
      explanation: { branches: [] },
      cancelled: false,
    };
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue(execution);

    const { root, dispose } = mount(() => <Block id="query" />);
    await settleQuery();

    const hits = [...root.querySelectorAll(".query-search-hit")].map((el) => el.textContent ?? "");
    expect(hits).toHaveLength(1);
    expect(hits[0]).toContain("TODO From query");
    dispose();
  });
});

// **Scoped Display settings on an inline query** (SPEC §7.6, Q3).
//
// A Friendly search answers two questions at once, and its two answers are
// controlled independently: `tine.page-*` for the Pages section, `tine.block-*`
// for the Blocks section, `tine.page-match-scope` for which pages are members
// at all. Rust reads all of it and hands it to the frontend flattened beside
// `{query, view}`; these cases are the evidence that the frontend consumes it
// as WRITTEN — including the difference between a draft that is absent and one
// that is present and empty, which is the difference between "inherit" and
// "clear".
describe("q3: scoped display settings on an inline query", () => {
  const pageHit = (name: string, path: string): QueryHit => ({
    entity: "page",
    page: { name, kind: "page", date_key: null, path },
    display_text: name,
    evidence: [{ clause_id: 1, field: "page_name", mode: "contains", spans: [{ start: 0, end: 5 }] }],
    score: 10,
    row: { name, kind: "page", path, properties: [["status", "open"]] },
  });
  const blockHit = (id: string, raw: string): QueryHit => ({
    entity: "block",
    page: "Sheet",
    kind: "page",
    block: { id, raw, collapsed: false, children: [], breadcrumb: [], properties: [] },
    display_text: raw,
    evidence: [{ clause_id: 1, field: "visible_content", mode: "contains", spans: [{ start: 0, end: 5 }] }],
  });
  const mixed = (): QueryExecution => ({
    hits: [pageHit("Alpha notes", "pages/alpha.md"), blockHit("todo", "TODO alpha work")],
    diagnostics: [],
    explanation: { branches: [] },
    cancelled: false,
    has_more: { pages: true, blocks: false },
  });

  function loadFriendly(raw: string, reading: Parameters<typeof backendReadsQueries>[0][string]) {
    setDoc({
      byId: {
        query: node("query", raw, null),
        todo: node("todo", "TODO alpha work", null),
      },
      pages: [page(["query", "todo"])],
      feed: ["Sheet"],
      loaded: true,
    });
    backendReadsQueries({ '(search "alpha")': reading });
  }

  it("q3_macro_consumes_flattened_scopes", async () => {
    // FAIL-BEFORE: Macro read the singular `view` only, so both families
    // rendered under one presentation and the two sections did not exist. The
    // Pages half of the answer was dropped entirely outside the Search face.
    loadFriendly('{{query (search "alpha")}}\ntine.view:: list', {
      form: '(search "alpha")',
      filter: searchFilter("alpha"),
      view: { view: "list" },
      // Pages are a TABLE with a column of the page's own property; blocks stay
      // on the inherited list. Two families, two presentations, one query.
      page_presentation: "table",
      page_display: { columns: ["status"] },
      page_match_scope: "both",
    });
    const search = vi.spyOn(backend(), "runGraphSearch").mockResolvedValue(mixed());
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      // Both resolved views ride the ONE request, with the membership scope.
      expect(search).toHaveBeenCalledWith(
        "alpha", 500, 5_000, "inline-query:query", false, undefined,
        {
          pageMatchScope: "both",
          pageView: { view: "table", columns: ["status"] },
          blockView: { view: "list" },
        },
      );

      const pages = root.querySelector<HTMLElement>('[data-query-result-kind="page"]')!;
      const blocks = root.querySelector<HTMLElement>('[data-query-result-kind="block"]')!;
      expect(pages.querySelector("h3")?.textContent).toBe("Pages");
      expect(blocks.querySelector("h3")?.textContent).toBe("Blocks");
      // The Pages section took its OWN presentation…
      expect(pages.querySelector("table.query-results-table")).not.toBeNull();
      expect([...pages.querySelectorAll("thead th")].map((th) => th.textContent)).toEqual(["Page", "status"]);
      expect(pages.querySelector("tbody tr")?.textContent).toContain("open");
      // …and the Blocks section kept the inherited one.
      expect(blocks.querySelector("table")).toBeNull();
      expect(blocks.textContent).toContain("alpha work");
      // The backend's own truncation flag, beside the family it describes.
      expect(pages.textContent).toContain("More pages match than are shown.");
      expect(blocks.textContent).not.toContain("More blocks match");

      // Each section names its own control and its own dialog.
      expect(pages.querySelector('[aria-label="Display pages"]')).not.toBeNull();
      expect(blocks.querySelector('[aria-label="Display blocks"]')).not.toBeNull();
      // The membership control belongs to neither namespace's display state.
      const scope = pages.querySelector<HTMLSelectElement>(".query-page-match select")!;
      expect(scope.value).toBe("both");
      expect([...scope.options].map((option) => option.textContent))
        .toEqual(["Names and aliases", "Page content", "Both"]);
      expect([...scope.options].map((option) => option.value)).toEqual(["names", "content", "both"]);
    } finally {
      dispose();
    }
  });

  it("q3_scoped_block_presentation_reaches_the_sheet_face", async () => {
    // FAIL-BEFORE: the scoped presentation was inert for the two faces that are
    // drawn by a sheet. `sheetFaceFor` asked the SINGULAR `tine.view` whether it
    // was already a sheet face before it would honour a scoped `board`, so a
    // note whose Blocks section said `tine.block-view:: board` fell through to
    // the grouped renderer: the panel said Board and the section kept showing a
    // list, with nothing on screen or in the file to explain the disagreement.
    // The presentation is the scoped namespace's own authority (§15.1); what a
    // sheet face needs from the block is a schema owner, not the other
    // namespace's opinion.
    loadFriendly('{{query (search "alpha")}}\ntine.view:: list\ntine.block-display:: 1\ntine.block-view:: board', {
      form: '(search "alpha")',
      filter: searchFilter("alpha"),
      view: { view: "list" },
      block_presentation: "board",
      block_display: {},
    });
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue(mixed());
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const blocks = root.querySelector<HTMLElement>('[data-query-result-kind="block"]')!;
      expect(blocks.querySelector(".sheet-board")).not.toBeNull();
      // The other family is untouched by it: nothing about the Pages section
      // asked for a sheet, so it keeps the presentation it inherited.
      const pages = root.querySelector<HTMLElement>('[data-query-result-kind="page"]')!;
      expect(pages.querySelector(".sheet-board")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("q3_absent_scope_inherits_and_present_empty_clears", async () => {
    // FAIL-BEFORE: nothing distinguished the two, because nothing read either.
    // `{}` is falsy-shaped in every way that matters, so a truthiness copy of
    // the draft turns "clear" into "inherit" silently.
    const search = vi.spyOn(backend(), "runGraphSearch").mockResolvedValue(mixed());

    // No page draft at all: the Pages section inherits the singular sort.
    loadFriendly('{{query (search "alpha")}}\ntine.view:: list', {
      form: '(search "alpha")',
      filter: searchFilter("alpha"),
      view: { view: "list", sort: [["page", "asc"]] },
    });
    let mounted = mount(() => <Block id="query" />);
    await settleQuery();
    expect(search.mock.calls.at(-1)?.[6]).toEqual({
      pageView: { view: "list", sort: [["page", "asc"]] },
      blockView: { view: "list", sort: [["page", "asc"]] },
    });
    mounted.dispose();
    resetStore();

    // A PRESENT, empty page draft: the marker is there and states nothing, so
    // the Pages section shows nothing extra. The Blocks section is untouched.
    loadFriendly('{{query (search "alpha")}}\ntine.view:: list', {
      form: '(search "alpha")',
      filter: searchFilter("alpha"),
      view: { view: "list", sort: [["page", "asc"]] },
      page_display: {},
    });
    mounted = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      expect(search.mock.calls.at(-1)?.[6]).toEqual({
        pageView: { view: "list" },
        blockView: { view: "list", sort: [["page", "asc"]] },
      });
    } finally {
      mounted.dispose();
    }
  });

  it("q3_scoped_edit_preserves_sibling_and_source", async () => {
    // FAIL-BEFORE: there was no scoped writer on this path at all, and the one
    // singular writer rewrote `tine.sort`/`tine.columns` for the whole block.
    loadFriendly(
      '{{query (search "alpha")}}\ntine.view:: list\ntine.sort:: page asc\n'
      + 'tine.block-display:: 1\ntine.block-columns:: owner\nunknown.property:: keep me',
      {
        form: '(search "alpha")',
        filter: searchFilter("alpha"),
        view: { view: "list", sort: [["page", "asc"]] },
        block_display: { columns: ["owner"] },
      },
    );
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue(mixed());
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const before = doc.byId.query.raw;

      // Switch the PAGES section to a board through its own panel.
      const pages = root.querySelector<HTMLElement>('[data-query-result-kind="page"]')!;
      pages.querySelector<HTMLButtonElement>('[aria-label="Display pages"]')!.click();
      const panel = await vi.waitFor(() => {
        const found = document.querySelector<HTMLElement>('[aria-label="Page display"]');
        if (!found) throw new Error("the Page display panel never opened");
        return found;
      });
      [...panel.querySelectorAll<HTMLButtonElement>(".qd-view")]
        .find((button) => button.textContent?.trim() === "Board")!.click();

      // The page namespace gained its own complete draft…
      expect(blockProperty("query", "tine.page-view")).toBe("board");
      expect(blockProperty("query", "tine.page-display")).toBe("1");
      // …cloned from what the section was ALREADY showing, so the edit did not
      // silently clear the inherited sort.
      expect(blockProperty("query", "tine.page-sort")).toBe("page asc");
      // The sibling namespace, the singular settings, the unknown authored
      // property and the query TEXT are all exactly as they were.
      expect(blockProperty("query", "tine.block-columns")).toBe("owner");
      expect(blockProperty("query", "tine.block-display")).toBe("1");
      expect(blockProperty("query", "tine.sort")).toBe("page asc");
      expect(blockProperty("query", "tine.view")).toBe("list");
      expect(blockProperty("query", "unknown.property")).toBe("keep me");
      expect(doc.byId.query.raw.split("\n")[0]).toBe(before.split("\n")[0]);

      // One undo unit: the whole scoped change comes back in one step.
      undo();
      expect(doc.byId.query.raw).toBe(before);
    } finally {
      dispose();
    }
  });

  it("q3_mixed_operation_rejects_stale_completion", async () => {
    // FAIL-BEFORE (I-20): request identity carried the query and the singular
    // view only, so a read started under one section's settings could land on
    // a screen showing another's — and both families would then describe
    // different graph states.
    loadFriendly('{{query (search "alpha")}}\ntine.view:: list', {
      form: '(search "alpha")',
      filter: searchFilter("alpha"),
      view: { view: "list" },
    });
    // The reading the ENGINE would give: it reads `tine.*` off the block every
    // time, so the membership scope the user just wrote is in the next parse.
    vi.spyOn(backend(), "parseQuery").mockImplementation(async () => {
      const written = blockProperty("query", "tine.page-match-scope");
      return {
        query: {
          anchor: "block" as const,
          filter: searchFilter("alpha"),
          diagnostics: [],
          source: { kind: "og" as const, original: '(search "alpha")', og_options: "" },
        },
        view: { view: "list" as const },
        ...(written ? { page_match_scope: written as "names" | "content" | "both" } : {}),
      };
    });
    let releaseFirst!: (value: QueryExecution) => void;
    const first = new Promise<QueryExecution>((resolve) => { releaseFirst = resolve; });
    const search = vi.spyOn(backend(), "runGraphSearch")
      .mockImplementationOnce(() => first)
      .mockResolvedValue({
        hits: [pageHit("Beta notes", "pages/beta.md")],
        diagnostics: [],
        explanation: { branches: [] },
        cancelled: false,
      });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      expect(search).toHaveBeenCalledTimes(1);

      // The membership scope changes while the first read is still in flight.
      // That is a DIFFERENT question, so it starts its own read. The engine
      // re-reads the block's properties on every parse, so the stub does too —
      // a stub that kept answering "no scope" would be testing a backend that
      // cannot see the property the user just wrote.
      const scope = root.querySelector<HTMLSelectElement>(".query-page-match select")!;
      scope.value = "content";
      scope.dispatchEvent(new Event("change", { bubbles: true }));
      await settleQuery();
      expect(search.mock.calls.length).toBeGreaterThan(1);
      expect(search.mock.calls.at(-1)?.[6]).toMatchObject({ pageMatchScope: "content" });
      await vi.waitFor(() => expect(
        root.querySelector('[data-query-result-kind="page"]')?.textContent,
      ).toContain("Beta notes"));

      // The superseded answer lands late. It describes a query that is no
      // longer on screen, so nothing about it may reach the sections.
      releaseFirst(mixed());
      await settleQuery();
      const pages = root.querySelector<HTMLElement>('[data-query-result-kind="page"]')!;
      expect(pages.textContent).toContain("Beta notes");
      expect(pages.textContent).not.toContain("Alpha notes");
      expect(root.querySelector('[data-query-result-kind="block"]')?.textContent)
        .toContain("No matching blocks.");
    } finally {
      dispose();
    }
  });
});

// Exercise the actual editor completion and mounted builder ownership handoff.
it("choosing the Query slash command opens its sheet and field chooser", async () => {
  backendReadsQueries({ "": { form: "", filter: { kind: "and", items: [] } } });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockResult([]));
  setDoc({
    byId: { query: node("query", "/query", null) },
    pages: [page(["query"])], feed: ["Sheet"], loaded: true,
  });
  startEditing("query", 6);
  const { root, dispose } = mount(() => <Block id="query" />);
  try {
    const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
    editor.focus();
    editor.value = "/query";
    editor.setSelectionRange(6, 6);
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "y" }));
    const command = await vi.waitFor(() => {
      const found = [...document.querySelectorAll<HTMLElement>(".autocomplete .ac-item")]
        .filter(item => item.querySelector(".ac-label")?.textContent === "Query");
      expect(found).toHaveLength(1);
      return found[0];
    });
    command.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(doc.byId.query.raw.trim()).toBe("{{query }}"));
    await vi.waitFor(() => expect(document.querySelector(".qs-sheet")).not.toBeNull());
    await vi.waitFor(() => expect(document.querySelector(".qs-vocab")).not.toBeNull());
    expect(editingId()).toBeNull();
  } finally { dispose(); }
});
