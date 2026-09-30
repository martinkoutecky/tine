import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { blockProperty, resetStore, undo } from "../document";
import { type FeedPage, type Node as StoreNode } from "../document/model";
import { doc, setDoc } from "../document/model";
import { openJournals, openPage, route } from "../router";
import { journalTitle } from "../journal";
import type { QueryExecution, QueryHit, RefGroup } from "../types";
import { editingId, startEditing } from "../editorController";
import type { ParsedQuery, QueryResult, QueryTextDialect, Source } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { searchFilter } from "../editor/queryBuilder";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { bumpDataRev } from "../graphSession";
import * as blockRender from "../render/block";
import { renderedBlocks, resetNearObserverForTests } from "../lazyObserve";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetNearObserverForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});

/** What the ONE engine reads for a macro argument, stated for these tests (the
 *  jsdom mock has no parser): an advanced vector/map is `advanced`, a trailing
 *  `{…}` after an OG form or vector is the opaque options map, and the host's
 *  `tine.view` reaches the view. The filter is the IR's honest `raw`. */
function readQuery(text: string, dialect: QueryTextDialect, properties: [string, string][] = []): ParsedQuery {
  const trimmed = text.trim();
  const split = /^([\s\S]*?[)\]"])\s*(\{:[\s\S]*\})$/.exec(trimmed);
  const [original, og_options] = split && !trimmed.startsWith("{") ? [split[1], split[2]] : [trimmed, ""];
  const kind: Source["kind"] = /^[[{]/.test(trimmed) ? "advanced" : dialect === "macro_tql" ? "tql" : "og";
  const view = Object.fromEntries(properties.filter(([key]) => key === "tine.view").map(([, value]) => ["view", value]));
  return {
    query: { anchor: "block", filter: { kind: "raw", text: original, diagnostic_kind: "not_applicable" }, diagnostics: [], source: { kind, original, og_options } as Source },
    view,
  } as ParsedQuery;
}

/** State what `query_run` answers: these block groups. */
function mockRun(groups: RefGroup[] | (() => RefGroup[]), report?: Parameters<typeof blockRunResult>[1]) {
  return vi.spyOn(backend(), "queryRun").mockImplementation(async () =>
    blockRunResult(typeof groups === "function" ? groups() : groups, report));
}

beforeEach(() => {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text, dialect, properties) => readQuery(text, dialect, properties));
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
  mockRun(queryGroups(["todo"]));
}


describe("QueryMacro sheet integration", () => {
  it("mounts broad List results in bounded frames while retaining keyed groups and cancelling retired work", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    renderedBlocks.add("query");
    let results: RefGroup[] = Array.from({ length: 100 }, (_, index) => ({
      page: `Result ${index}`, kind: "page", blocks: [{ id: `result-${index}`, raw: "TODO found", children: [], collapsed: false }],
    }));
    mockRun(() => results);
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrame = 0;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.set(++nextFrame, callback);
      return nextFrame;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    vi.stubGlobal("IntersectionObserver", class {
      observe() {} unobserve() {} disconnect() {}
    });
    const { root, dispose } = mount(() => <Block id="query" />);
    let retiredFrame: FrameRequestCallback | undefined;
    try {
      await settleQuery();
      expect(root.querySelectorAll(".query-group").length,
        "I-25: a broad List must mount at most 32 new group shells per frame").toBe(32);
      const first = root.querySelector(".query-group");
      expect(root.querySelector(".query-pending-groups")).not.toBeNull();
      const advanceFrame = () => {
        const before = root.querySelectorAll(".query-group").length;
        const [id, frame] = [...frames][0];
        frames.delete(id);
        frame(0);
        expect(root.querySelectorAll(".query-group").length - before).toBeLessThanOrEqual(32);
      };
      for (let turn = 0; root.querySelectorAll(".query-group").length < 64 && turn < 20; turn += 1) advanceFrame();
      expect(root.querySelectorAll(".query-group").length).toBe(64);
      expect(root.querySelector(".query-group")).toBe(first);
      for (let turn = 0; frames.size && turn < 20; turn += 1) advanceFrame();
      expect(root.querySelectorAll(".query-group").length).toBe(100);
      expect(root.querySelector(".query-pending-groups")).toBeNull();
      results = results.map((group) => ({ ...group, page: `New ${group.page}` }));
      bumpDataRev();
      await settleQuery();
      expect(root.querySelectorAll(".query-group").length).toBe(32);
      expect(frames.size).toBeGreaterThan(0);
      retiredFrame = [...frames.values()].at(-1);
    } finally { dispose(); }
    expect(frames.size).toBe(0);
    retiredFrame?.(0);
    expect(root.children.length).toBe(0);
  });
  it("defers offscreen List group headers and mounts them on viewport approach", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    renderedBlocks.add("query");
    const observations = new Map<Element, IntersectionObserverCallback>();
    vi.stubGlobal("IntersectionObserver", class {
      constructor(private callback: IntersectionObserverCallback) {}
      observe(element: Element) { observations.set(element, this.callback); }
      unobserve(element: Element) { observations.delete(element); }
      disconnect() { observations.clear(); }
    });
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await expect.poll(() => root.querySelectorAll(".query-group").length).toBe(1);
      expect(root.querySelector(".query-page"), "I-25: offscreen List groups reserve height without mounting headers and row subtrees").toBeNull();
      const group = root.querySelector(".query-group")!;
      const intersect = observations.get(group)!;
      expect(intersect).toBeTypeOf("function");
      intersect([{ target: group, isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
      expect(root.querySelector(".query-page")?.textContent).toBe("Sheet");
    } finally { dispose(); }
  });
  it("does not build search excerpts for a collapsed List query, and builds them when Search is chosen", async () => {
    loadQueryDoc('{{query (task TODO) {:collapsed? true}}}');
    const visible = vi.spyOn(blockRender, "visibleBody");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".query-count")?.textContent).toContain("1"));
      const excerptCalls = () => visible.mock.calls.filter(([raw]) => raw === doc.byId.todo.raw);
      expect(excerptCalls(), "List counts need no per-result Search projection").toHaveLength(0);
      clickView(root, "Search");
      await vi.waitFor(() => expect(excerptCalls().length).toBeGreaterThan(0));
      (root.querySelector(".query-collapse") as HTMLButtonElement).click();
      await vi.waitFor(() => expect(root.querySelector(".query-search-hit")?.textContent).toContain("From query"));
    } finally { dispose(); }
  });

  it("keeps a newer friendly-search result when an older request finishes last", async () => {
    setDoc({
      byId: {
        query: node("query", '{{query (search "alpha")}}\ntine.view:: search', null),
        old: node("old", "Old result", null),
        fresh: node("fresh", "Fresh result", null),
      },
      pages: [page(["query", "old", "fresh"])], feed: ["Sheet"], loaded: true,
    });
    const execution = (id: "old" | "fresh"): QueryExecution => ({
      hits: [{ entity: "block", page: "Sheet", kind: "page", block: {
        id, raw: doc.byId[id].raw, collapsed: false, children: [],
      }, display_text: doc.byId[id].raw, evidence: [] }],
      diagnostics: [], explanation: { branches: [] }, cancelled: false,
    });
    let finishOld!: (value: QueryExecution) => void;
    const search = vi.spyOn(backend(), "runGraphSearch")
      .mockImplementationOnce(() => new Promise((resolve) => { finishOld = resolve; }))
      .mockResolvedValue(execution("fresh"));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(search).toHaveBeenCalledTimes(1));
      bumpDataRev();
      await vi.waitFor(() => expect(search).toHaveBeenCalledTimes(2));
      await vi.waitFor(() => expect(root.textContent).toContain("Fresh result"));
      finishOld(execution("old"));
      await settleQuery();
      expect(root.textContent).toContain("Fresh result");
      expect(root.textContent).not.toContain("Old result");
    } finally {
      finishOld?.(execution("old"));
      dispose();
    }
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
    mockRun([
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
    const runQuery = mockRun(freshResult);

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
    vi.mocked(backend().parseQuery).mockImplementation(async (text, dialect, properties) => {
      const read = readQuery(text, dialect, properties);
      return { ...read, query: { ...read.query, filter: searchFilter("alpha beta") } };
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

    expect(activeView(root)).toBe("Search");
    // Master's assertion (QueryMacro.test.tsx "reopens a materialized friendly search").
    expect(root.querySelector(".qs-sentence")?.textContent).toBe("Blocks where search: alpha beta");
    expect(root.querySelector(".qs-seg-value")?.textContent).toBe("alpha beta");
    expect(root.querySelector(".qs-seg-advanced")).toBeNull();
    expect([...root.querySelectorAll("mark")].map((mark) => mark.textContent)).toEqual(["alpha", "beta"]);
    expect(graphSearch).toHaveBeenCalledWith("alpha beta", 500, 5_000, "inline-query:query", false);
    root.querySelector<HTMLButtonElement>(".query-search-hit")!.click();
    expect(route()).toMatchObject({ kind: "page", name: "Sheet", pageKind: "page" });

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
    mockRun(queryGroups(ids));
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
    expect(root.querySelector(".qs-sentence")).not.toBeNull();
    expect(root.querySelector(".qs-seg")).not.toBeNull();
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
    const run = mockRun(queryGroups(["low", "high"]));

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
    expect(run.mock.calls.every(([query]) => query.source.kind === "og" && query.source.original === '(and (todo TODO) "score")')).toBe(true);

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
    expect(root.querySelector(".qs-sentence")).not.toBeNull();
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
    mockRun(queryGroups(["todo"]));
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
    mockRun(queryGroups(["todo"]));
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
    // A wide result table scrolls inside .md-table-wrap instead of cram-wrapping
    // its nowrap cells past the block (master ee7730b48).
    expect(root.querySelector(".query-table")!.parentElement!.classList.contains("md-table-wrap")).toBe(true);
    expect(root.querySelectorAll(".sheet-table")).toHaveLength(0);

    dispose();
  });

  // og's "shows an enabled Simple toggle for stashed advanced queries…" is retired
  // with the frontend datalog/DSL converters, as on master (§9 P0-ts: the
  // `⚙ advanced` / `← Simple` pair is gone; Macro.tsx has no stash).
  // Ported from master QueryMacro.test.tsx.
  it("renders an advanced query's report without offering the filter builder", async () => {
    loadQueryDoc('{{query [:find (pull ?b [*]) :where (task ?b "TODO")]}}');
    mockRun(queryGroups(["todo"]), { ran: ["task"] });

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

describe("QueryMacro through query_parse + query_run", () => {
  const emptyRun = (extra: Partial<QueryResult> = {}): QueryResult => ({ ...blockRunResult([]), ...extra } as QueryResult);

  it("renders a page-anchored answer as page rows, sending the host block's tine.* view properties", async () => {
    loadQueryDoc("{{query (page-property type book)}}\ntine.sample:: 5\nowner:: Martin");
    const parse = vi.mocked(backend().parseQuery);
    const run = vi.spyOn(backend(), "queryRun").mockResolvedValue({
      anchor: "page",
      pages: [
        { path: "pages/Dune.md", name: "Dune", kind: "page", properties: [["type", "book"]] },
        { path: "pages/Emma.md", name: "Emma", kind: "page", properties: [["type", "book"]] },
      ],
      diagnostics: [],
      report: { ran: [], ignored: [], supported: true },
      total: 2,
      exceeded: false,
    });
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelectorAll(".query-page-link").length).toBe(2));
      // Only `tine.*` keys travel; the engine merges them into the view
      // (master semantics: `tine.sample::` on the host block samples).
      expect(parse).toHaveBeenCalledWith("(page-property type book)", "macro_query", [["tine.sample", "5"]]);
      // The run is bound to the page the query block is on (master semantics).
      expect(run.mock.calls[0][2]).toEqual({ current_page: "Sheet" });
      expect([...root.querySelectorAll(".query-page-link")].map((el) => el.textContent)).toEqual(["Dune", "Emma"]);
      expect(root.querySelector(".query-count")?.textContent).toBe("2");
    } finally {
      dispose();
    }
  });

  it("shows an invalid query's diagnostics instead of a bare \"No results\" (I-9)", async () => {
    loadQueryDoc("{{query (frobnicate x)}}");
    vi.spyOn(backend(), "queryRun").mockResolvedValue(emptyRun({
      diagnostics: [
        { kind: "unknown_head", message: "unknown query head `frobnicate`", suggestions: [], disabled: false },
        { kind: "syntax", message: "greyed out", suggestions: [], disabled: true },
      ],
    }));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".query-diagnostics")).not.toBeNull());
      const text = root.querySelector(".query-diagnostics")!.textContent ?? "";
      expect(text).toContain("didn't understand part of this query");
      expect(text).toContain("unknown query head `frobnicate`");
      expect(text).not.toContain("greyed out");
      expect(root.querySelector(".query-why-empty")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("binds :current-page to OG's current page — the focused route, else today — not the rendering page", async () => {
    const source = "{{query {:query [:find (pull ?b [*]) :in $ ?current-page :where [?p :block/name ?current-page] [?b :block/refs ?p]] :inputs [:current-page]}}}";
    loadQueryDoc(source);
    const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(emptyRun());
    openJournals();
    const { dispose } = mount(() => <Block id="query" />);
    try {
      // No routed page and no configured home: OG falls back to today.
      await vi.waitFor(() => expect(run).toHaveBeenCalled());
      expect(run.mock.calls.at(-1)![2]?.current_page).toBe(journalTitle(new Date()));
      openPage("Elsewhere");
      await vi.waitFor(() => expect(run.mock.calls.at(-1)![2]?.current_page).toBe("Elsewhere"));
      expect(run.mock.calls.map((call) => call[2]?.current_page)).not.toContain("Sheet");
    } finally {
      dispose();
    }
  });
});

// Ported from master QueryMacro.test.tsx.
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
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(queryGroups(["query", "todo"])));

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
    vi.spyOn(backend(), "queryRun").mockResolvedValue(
      blockRunResult(queryGroups(["query", "todo"]), { ran: ["content"] }),
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

// Ported from master QueryMacro.test.tsx (the field chooser is og's QueryListbox `.qs-menu`).
it("choosing the Query slash command opens its sheet and field chooser", async () => {
  vi.mocked(backend().parseQuery).mockImplementation(async (text, dialect, properties) => {
    const read = readQuery(text, dialect, properties);
    return { ...read, query: { ...read.query, filter: { kind: "and", items: [] } } };
  });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
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
    await vi.waitFor(() => expect(document.querySelector(".qs-menu")).not.toBeNull());
    expect(editingId()).toBeNull();
  } finally { dispose(); }
});
