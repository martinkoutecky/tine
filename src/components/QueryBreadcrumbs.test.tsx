import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { QueryMacro } from "./Macro";
import { BeginQuery } from "./BeginQuery";
import { inspectBeginQuery } from "../render/beginQuery";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { splitTrailingMap, readEdnOptions } from "../editor/edn";

beforeAll(initParser);
afterEach(() => { vi.restoreAllMocks(); resetStore(); resetSharedQueryResultsForTests(); document.body.innerHTML = ""; });

const query = '[:find (pull ?b [*]) :where [?b :block/marker "TODO"]]';
function engine(sorted = false, kind: "og" | "advanced" = "advanced") {
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

it("leaves simple-query breadcrumbs enabled while their switch awaits a decision", async () => {
  engine(false, "og");
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacro body="query (task TODO) {:breadcrumb-show? false}" />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("buy apples"));
    expect(root.querySelector(".ref-breadcrumb")?.textContent).toBe("Shopping parent");
  } finally { dispose(); }
});
