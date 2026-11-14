import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { blockProperty, resetStore, undo, redo } from "../document";
import { setDoc } from "../document/model";
import { openCalmQuery } from "../tests/calmQueryActions";
import { loadSingle } from "../document/workingSet";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { QueryMacro } from "./Macro";
import { BeginQuery } from "./BeginQuery";
import { inspectBeginQuery } from "../render/beginQuery";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { splitTrailingMap, readEdnOptions } from "../editor/edn";
import { QueryDisplay } from "./QueryDisplay";

const saveFixture = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn() }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saveFixture.markDirty : saveFixture.actual?.[key as string],
  has: () => true,
}));
beforeAll(async () => {
  saveFixture.actual = await vi.importActual("../document/host/wiring");
  await initParser();
});
afterEach(() => { vi.restoreAllMocks(); resetStore(); resetSharedQueryResultsForTests(); document.body.innerHTML = ""; });

const query = '[:find (pull ?b [*]) :where [?b :block/marker "TODO"]]';
function engine(sorted = false, kind: "og" | "tql" | "advanced" = "advanced") {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text) => {
    const { form, opts } = splitTrailingMap(text);
    return { query: { anchor: "block", filter: { kind: "raw", text: form, diagnostic_kind: "not_applicable" },
      source: { kind, original: form, og_options: opts } }, view: sorted ? { sort: [["content", "asc"]] } : {}, legacy_table: readEdnOptions(opts)?.table ?? false };
  });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([{ page: "Tasks", kind: "page", blocks: [
    { id: "hit", raw: "TODO buy apples", children: [], collapsed: false, breadcrumb: ["Shopping parent"] },
  ] }], { ran: ["task"], ignored: [], supported: true }));
}

it.each([false, true, undefined, null])("advanced grouped list honors breadcrumb %s and keeps its page heading", async (value) => {
  engine();
  const root = document.createElement("div"); document.body.append(root);
  const option = value === undefined ? "" : ` :breadcrumb-show? ${value === null ? "nil" : value}`;
  const dispose = render(() => <QueryMacro body={`query {:query ${query}${option}}`} />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("buy apples"));
    expect(root.querySelector(".query-page")?.textContent).toBe("Tasks");
    expect(root.querySelector(".ref-breadcrumb")?.textContent ?? "").toBe(value === false ? "" : "Shopping parent");
  } finally { dispose(); }
});

it.each([false, true])("hydrated list honors breadcrumb %s", async value => {
  engine();
  loadSingle({ name: "Tasks", title: "Tasks", kind: "page", pre_block: null, blocks: [
    { id: "parent", raw: "Shopping parent", collapsed: false, children: [
      { id: "hit", raw: "TODO buy apples", collapsed: false, children: [] },
    ] },
  ] });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacro body={`query {:query ${query} :breadcrumb-show? ${value}}`} />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector(".ls-block")?.textContent).toContain("buy apples"));
    expect(root.querySelector(".ref-breadcrumb")?.textContent ?? "").toBe(value ? "Shopping parent" : "");
    expect(root.querySelector(".query-page")?.textContent).toBe("Tasks");
  } finally { dispose(); }
});

it.each([false, true])("globally sorted list honors breadcrumb %s", async value => {
  engine(true);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacro body={`query {:query ${query} :breadcrumb-show? ${value}}`} />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("buy apples"));
    expect(root.querySelector(".query-crumb")?.textContent ?? "").toBe(value ? "Tasks" : "");
    expect(root.querySelector(".ref-breadcrumb")?.textContent ?? "").toBe(value ? "Shopping parent" : "");
  } finally { dispose(); }
});

it("does not change table rows when breadcrumbs are disabled", async () => {
  engine();
  const root = document.createElement("div"); document.body.append(root);
  const match = inspectBeginQuery(`#+BEGIN_QUERY\n{:query ${query} :breadcrumb-show? false}\n#+END_QUERY`, "md")!;
  const dispose = render(() => <BeginQuery match={match} />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector(".query-table")?.textContent).toContain("buy apples"));
    expect(root.querySelector(".query-table")?.textContent).toContain("Tasks");
  } finally { dispose(); }
});

it.each([false, true])("BEGIN_QUERY list honors breadcrumb %s with page grouping disabled", async (value) => {
  engine();
  const root = document.createElement("div"); document.body.append(root);
  const match = inspectBeginQuery(`#+BEGIN_QUERY\n{:query ${query} :table-view? false :group-by-page? false :breadcrumb-show? ${value}}\n#+END_QUERY`, "md")!;
  const dispose = render(() => <BeginQuery match={match} />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("buy apples"));
    expect(root.querySelector(".query-table")).toBeNull();
    expect(root.querySelector(".query-page, .query-crumb")).toBeNull();
    expect(root.querySelector(".ref-breadcrumb")?.textContent ?? "").toBe(value ? "Shopping parent" : "");
  } finally { dispose(); }
});

it.each([false, true])("advanced ungrouped list keeps breadcrumb %s independent of its page heading", async value => {
  engine();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacro body={`query {:query ${query} :group-by-page? false :breadcrumb-show? ${value}}`} />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("buy apples"));
    expect(root.querySelector(".query-page, .query-crumb")).toBeNull();
    expect(root.querySelector(".ref-breadcrumb")?.textContent ?? "").toBe(value ? "Shopping parent" : "");
  } finally { dispose(); }
});

it("ignores advanced breadcrumb syntax on simple queries", async () => {
  engine(false, "og");
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacro body="query (task TODO) {:breadcrumb-show? false}" />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("buy apples"));
    expect(root.querySelector(".ref-breadcrumb")?.textContent).toBe("Shopping parent");
  } finally { dispose(); }
});

it.each([false, true])("simple list View switch hides context, persists and undoes (sorted=%s)", async sorted => {
  engine(sorted, "og");
  setDoc({ byId: { query: { id: "query", raw: "{{query (task TODO)}}", page: "Queries", parent: null, children: [], collapsed: false } },
    pages: [{ name: "Queries", title: "Queries", kind: "page", roots: ["query"], preBlock: null, format: "md", readOnly: false, guide: false }],
    feed: ["Queries"], loaded: true });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacro body="query (task TODO)" blockId="query" />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("Shopping parent"));
    const panel = await openCalmQuery(root, "View");
    const toggle = () => panel.querySelector<HTMLInputElement>('input[aria-label="Show breadcrumbs"]')!;
    expect(toggle()).not.toBeNull();
    expect(toggle().checked).toBe(true);
    const parses = vi.mocked(backend().parseQuery).mock.calls.length;
    const runs = vi.mocked(backend().queryRun).mock.calls.length;
    saveFixture.markDirty.mockClear();
    toggle().click();
    await vi.waitFor(() => {
      expect(blockProperty("query", "tine.breadcrumbs")).toBe("false");
      expect(root.querySelector(".query-page, .query-crumb, .ref-breadcrumb")).toBeNull();
      expect(root.textContent).toContain("buy apples");
    });
    expect(saveFixture.markDirty).toHaveBeenCalledTimes(1);
    expect(backend().parseQuery).toHaveBeenCalledTimes(parses);
    expect(backend().queryRun).toHaveBeenCalledTimes(runs);
    undo();
    await vi.waitFor(() => expect(root.textContent).toContain("Shopping parent"));
    expect(blockProperty("query", "tine.breadcrumbs")).toBeNull();
    redo();
    await vi.waitFor(() => expect(toggle().checked).toBe(false));
    toggle().click();
    await vi.waitFor(() => expect(root.textContent).toContain("Shopping parent"));
    expect(blockProperty("query", "tine.breadcrumbs")).toBeNull();
  } finally { dispose(); }
});

it.each(["table", "board", "search", "page"] as const)("does not offer a breadcrumb switch for %s results", face => {
  const root = document.createElement("div"); document.body.append(root);
  const noop = () => {};
  const dispose = render(() => <QueryDisplay inline view={() => ({ view: face === "page" ? "list" : face })}
    apply={noop} rowKind={() => face === "page" ? "page" : "block"}
    breadcrumbs={{ shown: () => true, set: noop }}
    registry={{ rows: () => [], pending: () => false, failure: () => null, unavailable: () => false, request: noop, retry: noop }} />, root);
  try { expect(root.querySelector('input[aria-label="Show breadcrumbs"]')).toBeNull(); }
  finally { dispose(); }
});

for (const format of ["md", "org"] as const) {
  it.each(["og", "tql"] as const)(`restores the saved choice from a ${format} %s host without an editable query block`, async kind => {
    engine(false, kind);
    const root = document.createElement("div"); document.body.append(root);
    const body = kind === "tql" ? "tine-query @block and task = 'TODO'" : "query (task TODO)";
    const raw = `{{${body}}}\n` + (format === "md" ? "tine.breadcrumbs:: false" : ":PROPERTIES:\n:tine.breadcrumbs: false\n:END:");
    const dispose = render(() => <QueryMacro body={body} hostRaw={{ raw, format }} />, root);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("buy apples"));
      expect(root.querySelector(".query-page, .query-crumb, .ref-breadcrumb")).toBeNull();
    } finally { dispose(); }
  });
}
