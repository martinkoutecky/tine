import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal, type JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { blockProperty, resetStore, setRaw, undo } from "../document";
import { type FeedPage, type Node as StoreNode } from "../document/model";
import { doc, setDoc } from "../document/model";
import { openJournals, openPage, route } from "../router";
import { journalTitle } from "../journal";
import type { QueryExecution, QueryHit, RefGroup } from "../types";
import { editingId, startEditing, endEdit } from "../editorController";
import type { ParsedQuery, QueryResult, QueryTextDialect, Source } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { searchFilter, taskFilter } from "../editor/queryBuilder";
import { readEdnOptions } from "../editor/edn";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { bumpDataRev, bumpGraphEpoch } from "../graphSession";
import { QueryMacro } from "./Macro";
import * as blockRender from "../render/block";
import { renderedBlocks, resetNearObserverForTests } from "../lazyObserve";
import { openSwitcher, closeSwitcher } from "../ui";
import { openCalmQuery } from "../tests/calmQueryActions";
import { beginResultsChange, installResultsPendingGate } from "../queryResultsEpoch";

// These component fixtures do not bind a page host. Their source edits update
// the working set; native coverage exercises actual publication and the epoch
// suite separately pins refusal while publication is pending.
const saveFixture = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn(), persistTransfer: vi.fn(async () => true) }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saveFixture.markDirty
    : key === "persistTransfer" ? saveFixture.persistTransfer : saveFixture.actual?.[key as string],
  has: () => true,
}));

beforeAll(async () => {
  saveFixture.actual = await vi.importActual("../document/host/wiring");
  await initParser();
});

afterEach(() => {
  closeSwitcher();
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
    legacy_table: readEdnOptions(og_options)?.table ?? false,
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

async function clickView(root: HTMLElement, label: "Search" | "List" | "Table" | "Board"): Promise<void> {
  const panel = await openCalmQuery(root, "View");
  const button = [...panel.querySelectorAll('[aria-label="Query view"] button')].find(
    (el) => el.textContent?.trim() === label.toLowerCase()
  ) as HTMLButtonElement | undefined;
  if (!button) throw new Error(`missing query view button ${label}`);
  button.click();
}

function activeView(root: HTMLElement): string | undefined {
  const id = root.querySelector(".ls-block")?.getAttribute("data-block-id") ?? "query";
  const view = blockProperty(id, "tine.view") ?? "list";
  return view.charAt(0).toUpperCase() + view.slice(1);
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

function loadDamagedQueryDoc(raw: string) {
  loadQueryDoc(raw);
  // State the native empty OG reading (og.rs::parse_og): pages, true filter.
  // Before the fix the missing extent is converted to exactly this empty input.
  vi.mocked(backend().parseQuery).mockImplementation(async (text, dialect, properties) => {
    const parsed = readQuery(text, dialect, properties);
    return text === "" ? { ...parsed, query: { ...parsed.query, anchor: "page", filter: { kind: "true" } } } : parsed;
  });
  vi.mocked(backend().queryRun).mockResolvedValue({
    anchor: "page", pages: [
      { path: "pages/Sheet.md", name: "Sheet", kind: "page", properties: [] },
      { path: "pages/Other.md", name: "Other", kind: "page", properties: [] },
    ], diagnostics: [], report: { ran: [], ignored: [], supported: true }, total: 2, exceeded: false,
  });
}

describe("QueryMacro sheet integration", () => {
  it.each(["agenda", "config", "sections page"])("keeps the %s absent while a replacement graph's answer is pending then empty", async (surface) => {
    loadQueryDoc("{{query (task TODO)}}");
    const { root, dispose } = mount(() => <QueryMacro body="query (task TODO)" title="Scheduled & Deadline"
      hideWhenEmpty builtIn={surface === "agenda"} configQuery={surface === "config"} />);
    let land: ((value: QueryResult) => void) | undefined;
    try {
      await settleQuery();
      expect(root.textContent).toContain("From query");
      vi.mocked(backend().queryRun).mockImplementation(() => new Promise(resolve => { land = resolve; }));
      bumpGraphEpoch(); await settleQuery();
      expect(land).toBeDefined();
      expect(root.querySelector(".query-block")).toBeNull();
      land!(blockRunResult([])); await settleQuery();
      expect(root.querySelector(".query-block")).toBeNull();
    } finally { land?.(blockRunResult([])); dispose(); }
  });

  it("reveals a journal section only when its first nonempty answer is ready and retains it during save reruns", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    let land!: (value: QueryResult) => void;
    vi.mocked(backend().queryRun).mockImplementation(() => new Promise(resolve => { land = resolve; }));
    const { root, dispose } = mount(() => <QueryMacro body="query (task TODO)" title="Scheduled & Deadline" hideWhenEmpty builtIn />);
    try {
      await settleQuery();
      expect(root.querySelector(".query-block")).toBeNull();
      land(blockRunResult(queryGroups(["todo"]))); await settleQuery();
      expect(root.textContent).toContain("From query");
      bumpDataRev(); await settleQuery();
      expect(root.textContent).toContain("From query");
      expect(root.textContent).not.toContain("Loading query results");
      land(blockRunResult(queryGroups(["todo"]))); await settleQuery();
      expect(root.textContent).toContain("From query");
    } finally { land(blockRunResult([])); dispose(); }
  });

  it("keeps the loading placeholder for ordinary authored queries", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    let land!: (value: QueryResult) => void;
    vi.mocked(backend().queryRun).mockImplementation(() => new Promise(resolve => { land = resolve; }));
    const { root, dispose } = mount(() => <QueryMacro body="query (task TODO)" />);
    try {
      await settleQuery();
      expect(root.textContent).toContain("Loading query results");
    } finally { land(blockRunResult([])); dispose(); }
  });

  it.each(["group", "aggregate"])("keeps membership visible through a %s option save", async option => {
    const raw = '{{query (task TODO) {:title "Tasks"}}}';
    loadQueryDoc(raw);
    vi.mocked(backend().parseQuery).mockImplementation(async (text, dialect, properties = []) => {
      const parsed = readQuery(text, dialect, properties);
      return { ...parsed, view: properties.length ? option === "group" ? { group_by: "owner" }
        : { aggregates: [["owner", "count"]] } : {} };
    });
    const { root, dispose } = mount(() => <Block id="query" />);
    let finish: (() => void) | undefined;
    try {
      await settleQuery();
      finish = beginResultsChange();
      setRaw("query", `${raw}\n${option === "group" ? "tine.group-field:: owner" : "tine.col-aggregates:: owner=count"}`);
      await settleQuery();
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
      expect(root.querySelector(".query-group")?.textContent).toContain("From query");
    } finally { finish?.(); dispose(); }
  });
  it("marks retained results busy until an ordinary save rerun actually lands", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    const { root, dispose } = mount(() => <Block id="query" />);
    let land: ((value: QueryResult) => void) | undefined;
    try {
      await settleQuery();
      vi.mocked(backend().queryRun).mockImplementation(() => new Promise(resolve => { land = resolve; }));
      bumpDataRev(); await settleQuery();
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
      expect(root.querySelector(".query-block")?.getAttribute("aria-busy")).toBe("true");
      land!(blockRunResult(queryGroups(["todo"])));
      await settleQuery();
      expect(root.querySelector(".query-block")?.getAttribute("aria-busy")).not.toBe("true");
    } finally { land?.(blockRunResult(queryGroups(["todo"]))); dispose(); }
  });
  it("retires the answer when an option save changes its authored dialect", async () => {
    loadQueryDoc('{{query (task TODO) {:title "Tasks"}}}');
    vi.mocked(backend().parseQuery).mockImplementation(async (text, dialect, properties) => {
      const parsed = readQuery(text, dialect, properties);
      return { ...parsed, query: { ...parsed.query, filter: taskFilter(["TODO"]) } };
    });
    const { root, dispose } = mount(() => <Block id="query" />);
    let finish: (() => void) | undefined;
    try {
      await settleQuery();
      finish = beginResultsChange();
      setRaw("query", '{{tine-query @block and task in (\'TODO\') {:title "Tasks"}}}\ntine.columns:: owner');
      await settleQuery();
      expect(root.querySelector(".query-group")).toBeNull();
    } finally { finish?.(); dispose(); }
  });
  it("retires an OG answer when identical stored IR has different empty-task semantics in TQL", async () => {
    loadQueryDoc("{{query (and (task) (task TODO))}}");
    vi.mocked(backend().parseQuery).mockImplementation(async (text, dialect, properties) => {
      const parsed = readQuery(text, dialect, properties);
      return { ...parsed, query: { ...parsed.query, filter: { kind: "and", items: [taskFilter([]), taskFilter(["TODO"])] } } };
    });
    const { root, dispose } = mount(() => <Block id="query" />);
    let finish: (() => void) | undefined;
    try {
      await settleQuery();
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
      finish = beginResultsChange();
      setRaw("query", "{{tine-query @block and task in () and task in ('TODO')}}");
      await settleQuery();
      expect(root.querySelector(".query-group")).toBeNull();
    } finally { finish?.(); dispose(); }
  });
  it.each([
    ['Notes: {{query (task TODO) {:title "Tasks"}}}', 0],
    ['{{query (task TODO) {:title "Tasks"}}} and {{query (task TODO) {:title "Tasks"}}}', 1],
  ] as const)("keeps an inline query's answer through rename in %s", async (raw, index) => {
    loadQueryDoc(raw);
    vi.spyOn(backend(), "printQuery").mockResolvedValue('(task TODO) {:title "Renamed"}');
    const { root, dispose } = mount(() => <Block id="query" />);
    let finish: (() => void) | undefined;
    try {
      await settleQuery();
      const query = () => root.querySelectorAll(".query-block")[index];
      expect(query().querySelector(".query-count")?.textContent).toBe("1");
      finish = beginResultsChange();
      query().querySelector<HTMLElement>(".query-title")!.click();
      const input = query().querySelector<HTMLInputElement>(".query-title-input")!;
      input.value = "Renamed";
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await settleQuery();
      expect(query().querySelector(".query-title")?.textContent).toBe("Renamed");
      expect(query().querySelector(".query-count")?.textContent).toBe("1");
      expect(query().textContent).toContain("From query");
      if (index === 1) expect(root.querySelector(".query-title")?.textContent).toBe("Tasks");
    } finally { finish?.(); dispose(); }
  });
  it.each([false, true])("folding the query's bullet folds its results (child block: %s, GH #619 c6)", async (withChild) => {
    // OG query.cljs custom-query* hides the results of a collapsed block, and
    // editor.cljs collapsable? lets a query block fold without children.
    loadQueryDoc("{{query (task TODO)}}");
    if (withChild) {
      setDoc("byId", "hi", node("hi", "Hi", "query"));
      setDoc("byId", "query", "children", ["hi"]);
    }
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const row = () => root.querySelector<HTMLElement>(":scope > .ls-block > .block-main")!;
      expect(row().querySelector(".query-group")).not.toBeNull();
      const arrow = row().querySelector<HTMLElement>(".collapse-toggle")!;
      expect(arrow.classList.contains("has-children")).toBe(true);
      arrow.click();
      await settleQuery();
      expect(doc.byId.query.raw).toContain("collapsed:: true");
      expect(row().querySelector(".query-group")).toBeNull();
      expect(row().textContent).not.toContain("From query");
      // The query's own arrow expands it again, and with it the bullet.
      row().querySelector<HTMLElement>(".query-collapse")!.click();
      await settleQuery();
      expect(doc.byId.query.collapsed).toBe(false);
      expect(doc.byId.query.raw).not.toContain("collapsed::");
      expect(row().querySelector(".query-group")).not.toBeNull();
    } finally { dispose(); }
  });
  it("starts a rename with the caret in the title field after one click (GH #619 c6)", async () => {
    // Chromium/WebView2 honours `autofocus` once per document and jsdom never does:
    // only an explicit focus puts the caret in the field on every rename.
    loadQueryDoc('{{query (task TODO) {:title "Tasks"}}}');
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      for (const _round of [1, 2]) {
        root.querySelector<HTMLElement>(".query-title")!.click();
        await Promise.resolve();
        const input = root.querySelector<HTMLInputElement>(".query-title-input")!;
        expect(document.activeElement).toBe(input);
        expect([input.selectionStart, input.selectionEnd]).toEqual([0, "Tasks".length]);
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        await settleQuery();
        expect(root.querySelector(".query-title-input")).toBeNull();
      }
    } finally { dispose(); }
  });
  it("keeps focus in View when saving removes the focused display control", async () => {
    const original = '(and (task TODO) (sort-by owner asc)) {:title "Tasks"}';
    loadQueryDoc(`{{query ${original}}}`);
    vi.mocked(backend().parseQuery).mockResolvedValueOnce({ ...readQuery(original, "macro_query"), view: { sort: [["owner", "asc"]] } })
      .mockResolvedValue(readQuery('(task TODO) {:title "Tasks"}', "macro_query"));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue('(task TODO) {:title "Tasks"}');
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const panel = await openCalmQuery(root, "View");
      const remove = panel.querySelector<HTMLButtonElement>('button[aria-label="Remove owner"]')!;
      expect(remove).not.toBeNull();
      remove.focus(); remove.click();
      await settleQuery();
      expect(panel.querySelector('button[aria-label="Remove owner"]')).toBeNull();
      expect(panel.contains(document.activeElement)).toBe(true);
      document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      expect(root.querySelector(".qc-panel")).toBeNull();
    } finally { dispose(); }
  });
  it("renames by Enter without losing results while its own publication delays adoption", async () => {
    const [pending, setPending] = createSignal(false);
    installResultsPendingGate(pending, "own-work");
    loadQueryDoc('{{query (task TODO)}}\nowner:: Martin');
    vi.spyOn(backend(), "printQuery").mockResolvedValue('(task TODO) {:title "Renamed"}');
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      setPending(true);
      root.querySelector<HTMLElement>(".query-title")!.click();
      const input = root.querySelector<HTMLInputElement>(".query-title-input")!;
      input.value = "Renamed";
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await settleQuery();
      expect(root.querySelector(".query-title")?.textContent).toBe("Renamed");
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
      expect(root.querySelector(".query-group")?.textContent).toContain("From query");
      expect(backend().queryRun).toHaveBeenCalledTimes(1);
      expect(root.querySelector(".query-block")?.getAttribute("aria-busy")).toBe("true");
      setPending(false);
      await vi.waitFor(() => expect(root.querySelector(".query-block")?.getAttribute("aria-busy")).not.toBe("true"));
    } finally { setPending(false); dispose(); }
  });
  it("opens an advanced query's View panel with focus and returns it on Escape", async () => {
    loadQueryDoc('{{query [:find ?b :where [?b :block/marker "TODO"]] {:title "Tasks"}}}');
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const panel = await openCalmQuery(root, "View");
      await tick();
      expect(document.activeElement).toBe(panel.querySelector(".qc-done"));
      document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      expect(root.querySelector(".qc-panel")).toBeNull();
      expect(document.activeElement).toBe(root.querySelector(".qc-actions > summary"));
    } finally { dispose(); }
  });
  it.each([
    ["md", "", '{:title "Renamed"}'],
    ["md", "\ntine.view:: table", '{:title "Renamed" :collapsed false}'],
    ["org", "\n:PROPERTIES:\n:tine.view: board\n:END:", '{:title "Renamed"}'],
    ["org", "\n:PROPERTIES:\n:tine.view: search\n:END:", '{:title "Renamed" :collapsed false}'],
  ] as const)("keeps the shown answer through %s options with %s while epochs invalidate actions", async (format, properties, options) => {
    loadQueryDoc(`{{query (task TODO) {:title "Open tasks"}}}${properties}`);
    setDoc("pages", 0, "format", format);
    const { root, dispose } = mount(() => <Block id="query" />);
    let finish: (() => void) | undefined;
    try {
      await settleQuery();
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
      finish = beginResultsChange();
      setRaw("query", `{{query (task TODO) ${options}}}${properties}`);
      await settleQuery();
      expect(root.querySelector(".query-title")?.textContent).toBe("Renamed");
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
      expect(root.querySelector(".query-block")?.textContent).toContain("From query");
      expect(root.querySelector(".query-block")?.getAttribute("aria-busy")).toBe("true");
      finish(); finish = undefined;
      await vi.waitFor(() => expect(root.querySelector(".query-block")?.getAttribute("aria-busy")).not.toBe("true"));
      finish = beginResultsChange();
      setRaw("query", `{{query (task DONE) {:title "Renamed"}}}${properties}`);
      await settleQuery();
      expect(root.querySelector(".query-count")?.textContent).toBe("0");
    } finally { finish?.(); dispose(); }
  });
  it("persists Select none from default table columns and restores defaults on Undo", async () => {
    loadQueryDoc('{{query (task TODO)}}\ntine.view:: table');
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      expect(root.querySelectorAll(".sheet-field-header").length).toBeGreaterThan(0);
      const panel = await openCalmQuery(root, "View");
      [...panel.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === "Select none")!.click();
      await vi.waitFor(() => expect(blockProperty("query", "tine.columns")).toBe(""));
      await settleQuery();
      expect(root.querySelectorAll(".sheet-field-header")).toHaveLength(0);
      expect(root.querySelector(".sheet-title-cell")).not.toBeNull();
      undo(); await settleQuery();
      expect(blockProperty("query", "tine.columns")).toBeNull();
      expect(root.querySelectorAll(".sheet-field-header").length).toBeGreaterThan(0);
    } finally { dispose(); }
  });
  it("keeps a title rename pointer gesture inside the query controls", async () => {
    loadQueryDoc('{{query (task TODO) {:title "Open tasks"}}}');
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const title = root.querySelector<HTMLElement>(".query-title")!;
      title.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      title.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      title.click();
      expect(editingId()).toBeNull();
      expect(root.querySelector(".query-title-input")).not.toBeNull();
    } finally { dispose(); }
  });
  it("collapses and expands all List result trees through saved collapsed properties", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    setDoc("byId", "todo", "children", ["child"]);
    setDoc("byId", "child", node("child", "Child of result", "todo"));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const click = async (label: string) => {
        const details = root.querySelector<HTMLDetailsElement>(".qc-actions")!;
        details.open = true; details.dispatchEvent(new Event("toggle")); await tick();
        const id = details.querySelector("summary")!.getAttribute("aria-controls")!;
        const folds = [...document.getElementById(id)!.querySelectorAll<HTMLButtonElement>("button")].filter(button => ["Collapse all", "Expand all"].includes(button.textContent ?? ""));
        expect(folds).toHaveLength(1);
        const button = [...document.getElementById(id)!.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === label);
        expect(button).toBeDefined(); button!.click();
      };
      await click("Collapse all");
      await vi.waitFor(() => expect(blockProperty("todo", "collapsed")).toBe("true"));
      expect(root.querySelector('.query-group [data-block-id="child"]')).toBeNull();
      undo();
      await vi.waitFor(() => expect(root.querySelector('.query-group [data-block-id="child"]')).not.toBeNull());
      await click("Collapse all");
      await vi.waitFor(() => expect(blockProperty("todo", "collapsed")).toBe("true"));
      await click("Expand all");
      await vi.waitFor(() => expect(doc.byId.todo.collapsed).toBe(false));
      expect(blockProperty("todo", "collapsed")).toBeNull();
      await vi.waitFor(() => expect(root.querySelector('.query-group [data-block-id="child"]')).not.toBeNull());
    } finally { dispose(); }
  });
  it("marks its results busy while a graph change has not yet re-run them", async () => {
    // Result actions refuse until the answer re-lands after a save; aria-busy
    // says so, for assistive technology and for native journeys to wait on.
    loadQueryDoc("{{query (task TODO)}}");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const block = () => root.querySelector(".query-block")!;
      expect(block().querySelector('.query-group [data-block-id="todo"]')).not.toBeNull();
      expect(block().getAttribute("aria-busy")).not.toBe("true");
      const finish = beginResultsChange();
      expect(block().getAttribute("aria-busy")).toBe("true");
      finish();
      expect(block().getAttribute("aria-busy")).toBe("true");
      await vi.waitFor(() => expect(block().getAttribute("aria-busy")).not.toBe("true"));
    } finally { dispose(); }
  });
  it("does not run an empty new query before a condition is added", async () => {
    loadQueryDoc("{{query }}");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      expect(backend().queryRun).not.toHaveBeenCalled();
      expect(root.textContent).toContain("Add a condition to see results.");
    } finally { dispose(); }
  });
  it.each(["table", "board", "search"])("offers source task checkboxes in %s results", async (view) => {
    loadQueryDoc(`{{query (task TODO)}}\ntine.view:: ${view}`);
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      const checkbox = root.querySelector<HTMLElement>('[role="checkbox"]');
      expect(checkbox).not.toBeNull();
      checkbox!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, cancelable: true }));
      await vi.waitFor(() => expect(doc.byId.todo.raw).toContain("DONE From query"));
      expect(doc.byId.query.raw).toContain("(task TODO)");
      undo();
      expect(doc.byId.todo.raw).toContain("TODO From query");
    } finally { dispose(); }
  });
  it("rests with title and count and opens its builder inline from the query menu", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      expect(root.querySelector(".query-title")?.textContent).toBe("Query");
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
      expect(root.querySelector(".qs-sentence")).toBeNull();
      expect(root.querySelector(".query-view-switcher")).toBeNull();
      const trigger = root.querySelector<HTMLElement>('[aria-label="Query actions"]');
      expect(trigger).not.toBeNull();
      trigger!.click();
      await settleQuery();
      const menuId = trigger!.getAttribute("aria-controls")!;
      const edit = [...document.getElementById(menuId)!.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === "Edit");
      expect(edit).toBeDefined();
      edit!.click();
      await settleQuery();
      expect(root.querySelector(".qc-panel .qs-sheet")).not.toBeNull();
      expect(document.querySelector(".qs-sheet-anchor")).toBeNull();
      expect(root.querySelector(".query-count")?.textContent).toBe("1");
    } finally { dispose(); }
  });
  it("renders the reported options map without a stray brace", async () => {
    const raw = '{{query (page-property tags gptpro) {:title "gptpro"}}}\ntine.group-field:: prop:anchor\ntine.view:: search';
    loadQueryDoc(raw);
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      expect(root.querySelector(".query-title")?.textContent).toBe("gptpro");
      expect(root.querySelector(".block-content")?.textContent).not.toContain("}");
      expect(doc.byId.query.raw).toBe(raw);
    } finally { dispose(); }
  });

  it("shows a source parse error instead of running the reported damaged macro as an empty query", async () => {
    const raw = '{{query (page-property tags gptpro) {:title "gptpro"}}\ntine.group-field:: prop:anchor\ntine.view:: search';
    loadDamagedQueryDoc(raw);
    vi.mocked(backend().queryRun).mockClear();
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      expect(root.textContent).toMatch(/query source.*pars/i);
      expect(backend().queryRun).not.toHaveBeenCalled();
      expect(doc.byId.query.raw).toBe(raw);
      startEditing("query");
      await settleQuery();
      expect(root.querySelector<HTMLTextAreaElement>("textarea.block-editor")?.value).toContain(raw.split("\n")[0]);
    } finally { dispose(); }
  });

  it("reports unreadable source before offering a builder edit on the damaged block", async () => {
    const raw = '{{query (page-property tags gptpro) {:title "gptpro"}}\ntine.group-field:: prop:anchor\ntine.view:: search';
    loadDamagedQueryDoc(raw);
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue('(page-property tags gptpro)');
    vi.spyOn(backend(), "queryRegistry").mockResolvedValue({ generation: 1, rows: [{
      normalized_name: "tags", cardinality: "one", observed_type: "text",
      count_blocks: 0, count_pages: 1, mismatch_count: 0,
    }] });
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      // Exercise Martin's edit if an invalid source is still allowed into the builder.
      const gear = root.querySelector<HTMLButtonElement>(".qs-gear");
      if (gear) {
        gear.click();
        await settleQuery();
        document.querySelector<HTMLButtonElement>(".qs-add")!.click();
        await settleQuery();
        const tags = [...document.querySelectorAll<HTMLElement>("[role=option]")]
          .find((option) => option.textContent?.trim() === "tags");
        expect(tags).toBeDefined();
        tags!.click();
        const value = document.querySelector<HTMLInputElement>('.qs-input[aria-label="Value"]')!;
        value.value = "gptpro";
        value.dispatchEvent(new Event("input", { bubbles: true }));
        document.querySelector<HTMLButtonElement>(".qs-commit")!.click();
        await settleQuery();
      }
      expect(root.textContent).not.toContain("The block changed while saving");
      expect(root.querySelector('[role="alert"]')?.textContent).toMatch(/query source.*pars/i);
      expect(root.querySelector(".qs-gear")).toBeNull();
      expect(doc.byId.query.raw).toBe(raw);
    } finally { dispose(); }
  });
  it("leaves background List mounts pending while the picker owns foreground input", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    renderedBlocks.add("query");
    mockRun(Array.from({ length: 100 }, (_, index) => ({
      page: `Result ${index}`, kind: "page", blocks: [{ id: `result-${index}`, raw: "TODO found", children: [], collapsed: false }],
    })));
    const frames = new Map<number, FrameRequestCallback>();
    let serial = 0;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.set(++serial, callback);
      return serial;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
    openSwitcher();
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settleQuery();
      expect(root.querySelectorAll(".query-group").length,
        "I-25: covered background List DOM must yield to foreground picker input").toBe(0);
      expect(root.querySelector(".query-pending-groups")).not.toBeNull();
      closeSwitcher();
      expect(root.querySelectorAll(".query-group").length).toBe(0);
      const queued = [...frames.values()].at(-1)!;
      expect(queued).toBeTypeOf("function");
      openSwitcher();
      queued(0);
      expect(root.querySelectorAll(".query-group").length).toBe(0);
      closeSwitcher();
      for (let turn = 0; root.querySelectorAll(".query-group").length === 0 && turn < 20; turn += 1) {
        const [id, frame] = [...frames][0];
        frames.delete(id);
        frame(0);
      }
      expect(root.querySelectorAll(".query-group").length).toBe(32);
      openSwitcher();
      expect(root.querySelectorAll(".query-group").length).toBe(32);
    } finally { dispose(); }
  });
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
      await clickView(root, "Search");
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
    expect(root.querySelector(".qs-sentence")).toBeNull();
    expect(root.querySelector(".qs-seg-value")).toBeNull();
    expect(root.querySelector(".qs-seg-advanced")).toBeNull();
    expect([...root.querySelectorAll("mark")].map((mark) => mark.textContent)).toEqual(["alpha", "beta"]);
    expect(graphSearch).toHaveBeenCalledWith("alpha beta", 500, 5_000, "inline-query:query", false);
    root.querySelector<HTMLButtonElement>(".query-search-hit")!.click();
    expect(route()).toMatchObject({ kind: "page", name: "Sheet", pageKind: "page" });

    dispose();
  });

  it("GH #659 intentional OG difference: a Table answer retains the edited row until blur", async () => {
    setDoc({
      byId: {
        query: node("query", "{{query (task TODO)}}\ntine.view:: table", null),
        "todo-1": node("todo-1", "TODO Result 1", null),
      },
      pages: [page(["query", "todo-1"])], feed: ["Sheet"], loaded: true,
    });
    const run = mockRun(queryGroups(["todo-1"]));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector('.sheet-title-cell[data-block-id="todo-1"]')).not.toBeNull());
      startEditing("todo-1"); run.mockResolvedValue(blockRunResult([])); bumpDataRev();
      await settleQuery();
      expect(root.querySelector('.sheet-title-cell[data-block-id="todo-1"]')).not.toBeNull();
      endEdit("blur");
      await vi.waitFor(() => expect(root.querySelector('.sheet-title-cell[data-block-id="todo-1"]')).toBeNull());
      expect(run).toHaveBeenCalledTimes(2);
    } finally { endEdit("blur"); dispose(); }
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
      await clickView(root, view);
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
    expect(root.querySelector(".qc-actions summary")).not.toBeNull();
    expect(root.querySelector(".qs-seg")).toBeNull();
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

    await clickView(root, "Board");
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

    await clickView(root, "Table");
    expect(activeView(root)).toBe("Table");
    expect(blockProperty("query", "tine.view")).toBe("table");
    expect(doc.byId.query.raw).toBe("{{query (todo TODO)}}\ntine.view:: table");
    undo();
    expect(doc.byId.query.raw).toBe(originalRaw);
    expect(activeView(root)).toBe("List");

    await clickView(root, "Table");
    await clickView(root, "Board");
    expect(activeView(root)).toBe("Board");
    expect(blockProperty("query", "tine.view")).toBe("board");
    expect(blockProperty("query", "tine.group-by")).toBe("state");
    undo();
    expect(blockProperty("query", "tine.view")).toBe("table");
    expect(blockProperty("query", "tine.group-by")).toBeNull();

    await clickView(root, "Board");
    await clickView(root, "List");
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

    await clickView(root, "Board");

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
    expect(root.querySelector(".qc-actions summary")).not.toBeNull();
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

  it.each([
    ["{{query (task TODO)}}\nquery-table:: true", true],
    ["{{query (task TODO) table}}", false],
  ])("reads legacy table mode without writing query-table: %s", async (raw, property) => {
    loadQueryDoc(raw);
    vi.mocked(backend().parseQuery).mockImplementation(async (text, dialect, properties) => {
      if (property) expect(properties).toContainEqual(["query-table", "true"]);
      return { ...readQuery(text, dialect, properties), legacy_table: true };
    });
    const { root, dispose } = mount(() => <Block id="query" />);
    await settleQuery();
    expect(root.querySelectorAll(".query-table")).toHaveLength(1);
    expect(doc.byId.query.raw).toBe(raw);
    await clickView(root, "Table");
    expect(blockProperty("query", "tine.view")).toBe("table");
    expect(blockProperty("query", "query-table")).toBe(property ? "true" : null);
    undo();
    expect(doc.byId.query.raw).toBe(raw);
    await clickView(root, "List");
    expect(blockProperty("query", "tine.view")).toBe("list");
    await settleQuery();
    expect(root.querySelectorAll(".query-table")).toHaveLength(0);
    expect(blockProperty("query", "query-table")).toBe(property ? "true" : null);
    dispose();
  });

  // OG query_table.cljs:61-109,163-179 (audit #9): the legacy table reads its columns
  // and initial sort from the HOST block's `query-properties`, `query-sort-by` and
  // `query-sort-desc`.
  describe("legacy :table-view? host properties", () => {
    const groupsWith = (rows: Array<[string, Array<[string, string]>]>): RefGroup[] => [
      {
        page: "Sheet",
        kind: "page",
        blocks: rows.map(([text, properties], index) => ({
          id: `row-${index}`, raw: text, collapsed: false, children: [], properties,
        })),
      },
    ];
    const rowsData: Array<[string, Array<[string, string]>]> = [
      ["TODO bravo", [["owner", "Beta"], ["rank", "10"]]],
      ["TODO alpha", [["owner", "Alpha"], ["rank", "9"]]],
      ["TODO charlie", [["owner", "Gamma"], ["rank", "100"]]],
    ];
    const headers = (root: HTMLElement) =>
      [...root.querySelectorAll(".query-table th")].map((th) => th.textContent?.replace(/[ ▲▼]/g, ""));
    const firstColumn = (root: HTMLElement) =>
      [...root.querySelectorAll(".query-table tbody tr")].map((tr) => tr.querySelector("td")!.textContent);

    async function table(raw: string) {
      loadQueryDoc(raw);
      mockRun(groupsWith(rowsData));
      const view = mount(() => <Block id="query" />);
      await settleQuery();
      return view;
    }

    it("shows only the columns query-properties names, in that order", async () => {
      const { root, dispose } = await table(
        "{{query (task TODO) {:table-view? true}}}\nquery-properties:: [:rank :block]"
      );
      expect(headers(root)).toEqual(["rank", "Content"]);
      expect(firstColumn(root)).toEqual(["10", "9", "100"]);
      dispose();
    });

    it("derives block, page and the property columns when query-properties is absent", async () => {
      const { root, dispose } = await table("{{query (task TODO) {:table-view? true}}}");
      expect(headers(root)).toEqual(["Content", "Page", "owner", "rank"]);
      dispose();
    });

    it("sorts by query-sort-by, descending unless query-sort-desc is false", async () => {
      const desc = await table("{{query (task TODO) {:table-view? true}}}\nquery-sort-by:: owner");
      expect(firstColumn(desc.root)).toEqual(["charlie", "bravo", "alpha"]);
      desc.dispose();
      document.body.innerHTML = "";
      const asc = await table(
        "{{query (task TODO) {:table-view? true}}}\nquery-sort-by:: owner\nquery-sort-desc:: false"
      );
      expect(firstColumn(asc.root)).toEqual(["alpha", "bravo", "charlie"]);
      asc.dispose();
    });

    it("compares numeric cells as numbers", async () => {
      const { root, dispose } = await table(
        "{{query (task TODO) {:table-view? true}}}\nquery-sort-by:: rank\nquery-sort-desc:: false"
      );
      expect(firstColumn(root)).toEqual(["alpha", "bravo", "charlie"]);
      dispose();
    });

    it("leaves the engine order alone when no sort property is given", async () => {
      const { root, dispose } = await table("{{query (task TODO) {:table-view? true}}}");
      expect(firstColumn(root)).toEqual(["bravo", "alpha", "charlie"]);
      dispose();
    });
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

    await vi.waitFor(() => expect(root.querySelector(".query-header .query-count")).not.toBeNull());
    // GH #502: at rest an advanced query shows its title and results only; which
    // clauses ran is editing context, shown in its View panel.
    expect(root.querySelector(".query-adv-note")).toBeNull();
    const details = root.querySelector<HTMLDetailsElement>(".qc-actions")!;
    details.open = true; details.dispatchEvent(new Event("toggle")); await tick();
    const menuId = details.querySelector("summary")!.getAttribute("aria-controls")!;
    [...document.getElementById(menuId)!.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === "View")!.click();
    await vi.waitFor(() => expect(root.querySelector(".qc-panel .query-adv-note")?.textContent ?? "").toContain("ran: task"));
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

// Ported from master QueryMacro.test.tsx, then changed on purpose (Martin 2026-10-03, GH #619 comment 2): the sheet opens
// on the empty condition list and the field chooser (og's QueryListbox `.qs-menu`) stays CLOSED until the user opens it.
it("choosing the Query slash command opens its sheet on the condition list, chooser closed", async () => {
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
    await new Promise(resolve => setTimeout(resolve, 300));
    expect(document.querySelector(".qs-menu")).toBeNull();
    expect(document.querySelectorAll(".qs-sheet .qs-row")).toHaveLength(0);
    expect(editingId()).toBeNull();
  } finally { dispose(); }
});
