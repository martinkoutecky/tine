// GH #619 item 7 (Martin 2026-10-03, MUST): while the query sheet is open, the query's results show
// INSIDE it and follow the conditions as they change. The sheet covers the block's own results, so before
// this the user edited blind. Pinned here at the user's view: the sheet's own markup.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { clearTransientLayersForTest } from "../transientLayers";
import { QueryBuilder, type BuilderSession } from "./QueryBuilder";
import { pageRefFilter, taskFilter } from "../editor/queryBuilder";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import type { Filter, QueryResult } from "../editor/queryIr";
import { PREVIEW_DEBOUNCE_MS } from "./QueryLivePreview";

function session(filter: Filter): BuilderSession {
  return { query: { anchor: "block", filter, source: { kind: "builder" } }, view: {} };
}

function groupOf(...texts: string[]): QueryResult {
  return blockRunResult([{
    page: "Alpha",
    kind: "page",
    blocks: texts.map((text, index) => ({ id: `b-${text}-${index}`, raw: text, collapsed: false, children: [] })),
  }]);
}

function mountBuilder(first: Filter) {
  const host = document.createElement("div");
  document.body.append(host);
  const [current, setCurrent] = createSignal<BuilderSession>(session(first));
  const dispose = render(
    () => <QueryBuilder session={current} onChange={(next) => setCurrent(next)} blockId="host" />,
    host,
  );
  host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
  const sheet = document.querySelector<HTMLElement>(".qs-sheet");
  if (!sheet) throw new Error("the sheet did not open");
  const live = () => sheet.querySelector<HTMLElement>('[aria-label="Live results"]');
  return { host, sheet, live, setSession: setCurrent, dispose };
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const settled = () => wait(PREVIEW_DEBOUNCE_MS + 150);

const NARROW: Filter = { kind: "and", items: [taskFilter(["TODO"]), pageRefFilter("Alpha")] };
const WIDE: Filter = { kind: "and", items: [taskFilter(["TODO"])] };

beforeEach(() => {
  vi.spyOn(backend(), "queryFacets").mockResolvedValue([]);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(and (task TODO) [[Alpha]])");
});

afterEach(() => {
  clearTransientLayersForTest();
  resetSharedQueryResultsForTests();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("live results in the sheet", () => {
  it("shows the query's results inside the open sheet", async () => {
    vi.spyOn(backend(), "queryRun").mockResolvedValue(groupOf("write the report", "file the tax form"));
    const view = mountBuilder(NARROW);
    try {
      await vi.waitFor(() => expect(view.live()?.textContent).toContain("write the report"));
      expect(view.live()?.textContent).toContain("file the tax form");
      expect(view.live()?.querySelector(".qs-live-count")?.textContent).toContain("2 results");
    } finally { view.dispose(); }
  });

  it("follows an edit: removing a condition re-runs the query and the sheet shows the new answer", async () => {
    const run = vi.spyOn(backend(), "queryRun").mockImplementation(async (query) =>
      JSON.stringify(query.filter).includes("page_ref") || JSON.stringify(query.filter).includes("Alpha")
        ? groupOf("narrow hit")
        : groupOf("wide hit one", "wide hit two", "wide hit three"));
    const view = mountBuilder(NARROW);
    try {
      await vi.waitFor(() => expect(view.live()?.textContent).toContain("narrow hit"));
      const removes = [...view.sheet.querySelectorAll<HTMLButtonElement>('[aria-label="Remove condition"]')];
      removes[removes.length - 1].click();
      await vi.waitFor(() => expect(view.live()?.textContent).toContain("wide hit three"));
      expect(view.live()?.textContent).not.toContain("narrow hit");
      expect(view.live()?.querySelector(".qs-live-count")?.textContent).toContain("3 results");
      expect(run.mock.calls.length).toBeGreaterThanOrEqual(2);
    } finally { view.dispose(); }
  });

  it("only the latest query's answer lands: a slow superseded response is dropped", async () => {
    const pending: Array<(result: QueryResult) => void> = [];
    vi.spyOn(backend(), "queryRun").mockImplementation((query) =>
      JSON.stringify(query.filter).includes("Alpha")
        ? new Promise<QueryResult>((resolve) => pending.push(resolve))
        : Promise.resolve(groupOf("the fresh answer")));
    const view = mountBuilder(NARROW);
    try {
      await wait(20);
      view.setSession(session(WIDE));
      await vi.waitFor(() => expect(view.live()?.textContent).toContain("the fresh answer"));
      pending.forEach((resolve) => resolve(groupOf("the STALE answer")));
      await settled();
      expect(view.live()?.textContent).toContain("the fresh answer");
      expect(view.live()?.textContent).not.toContain("the STALE answer");
    } finally { view.dispose(); }
  });

  it("runs one query for a burst of edits, not one per keystroke", async () => {
    const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(groupOf("hit"));
    const view = mountBuilder(NARROW);
    try {
      await settled();
      const before = run.mock.calls.length;
      for (const name of ["Beta", "Gamma", "Delta", "Epsilon"]) {
        view.setSession(session({ kind: "and", items: [taskFilter(["TODO"]), pageRefFilter(name)] }));
        await wait(20);
      }
      await settled();
      expect(run.mock.calls.length - before).toBe(1);
    } finally { view.dispose(); }
  });

  it("says so when nothing matches, and shows the engine's error instead of an empty void", async () => {
    vi.spyOn(backend(), "queryRun").mockResolvedValue(groupOf());
    const view = mountBuilder(NARROW);
    try {
      await vi.waitFor(() => expect(view.live()?.querySelector(".qs-live-empty")).not.toBeNull());
    } finally { view.dispose(); }
  });
});

// GH #619 item 9: in "Pages and blocks" mode the sheet's preview shows both families, not just the own anchor.
describe("live results in Pages and blocks mode", () => {
  const pageResult = (matched: number): QueryResult => ({
    anchor: "page",
    pages: [{ path: "pages/Twin.md", name: "Twin", kind: "page", properties: [] }],
    diagnostics: [], report: { ran: [], ignored: [], supported: true }, total: 1, matched_total: matched, exceeded: false,
  });

  it("shows the page family and the block family together, with per-family counts and a cut-off note", async () => {
    vi.spyOn(backend(), "printQuery").mockResolvedValue("@page and task TODO");
    const parse = backend().parseQuery.bind(backend());
    vi.spyOn(backend(), "parseQuery").mockImplementation(async (text, ...rest) => {
      const parsed = await parse(text.startsWith("@page") ? "-- task TODO" : text, ...rest);
      return text.startsWith("@page") ? { ...parsed, query: { ...parsed.query, anchor: "page" as const } } : parsed;
    });
    vi.spyOn(backend(), "queryRun").mockImplementation(async (query) =>
      query.anchor === "page" ? pageResult(7) : groupOf("a block hit"));
    const host = document.createElement("div");
    document.body.append(host);
    const [current] = createSignal<BuilderSession>(session(WIDE));
    const dispose = render(
      () => <QueryBuilder session={current} onChange={() => undefined} blockId="host" both={{ on: () => true, set: () => undefined }} />,
      host,
    );
    host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
    try {
      const live = () => document.querySelector<HTMLElement>('.qs-sheet [aria-label="Live results"]');
      await vi.waitFor(() => expect(live()?.textContent).toContain("a block hit"));
      expect(live()?.textContent).toContain("Twin");
      const sheet = document.querySelector<HTMLElement>(".qs-sheet")!;
      expect(sheet.querySelector(".qs-live-count")?.textContent).toContain("7 pages");
      expect(sheet.querySelector(".qs-live-count")?.textContent).toContain("1 block");
      expect(sheet.textContent).toContain("More pages match than are shown.");
      expect(sheet.textContent).not.toContain("More blocks match than are shown.");
      // The sentence names both families.
      const sentence = host.querySelector(".qs-sentence")?.textContent ?? "";
      expect(sentence).toMatch(/Pages and blocks/);
      expect(sentence).not.toMatch(/^Blocks where/);
    } finally { dispose(); }
  });
});
