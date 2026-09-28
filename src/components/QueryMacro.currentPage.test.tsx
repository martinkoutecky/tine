// Ported from master src/components/QueryMacro.currentPage.test.tsx.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { openPage, route } from "../router";
import { resetPaneLayoutToSingle } from "../panes";
import type { QueryExecution, RefGroup } from "../types";
import { queryMacroExtent } from "../editor/queryMacro";
import { backendReadsQueries, blockRunResult } from "../tests/queryReadingsTestkit";
import type { Query, QueryResult } from "../editor/queryIr";

// GH #301 (approved): a query whose text explicitly carries `<% current page %>`
// binds that marker to the FOCUSED pane's route page and re-runs when that page
// changes. Everything else stays exactly as today: authoring text untouched,
// no rerun for marker-less queries, coalesced on same-page transitions, and a
// stale asynchronous result must never overwrite the latest page's result.

let disposeKeys: (() => void) | null = null;
beforeAll(async () => {
  await initParser();
  disposeKeys = null;
});

afterEach(() => {
  disposeKeys?.();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  localStorage.clear();
  document.body.innerHTML = "";
  document.head.querySelector('meta[name="tine-published"]')?.remove();
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(roots: string[]): FeedPage {
  return { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots, format: "md", readOnly: false, guide: false };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

/** The pages this file navigates between. The dyvar is substituted in the TEXT
 *  and the substituted text gets its OWN parse (§4.4 is about `?current-page`;
 *  `<% current page %>` is a text variable), so the engine is asked to read the
 *  substituted argument too — and a declared reading must exist for each. */
const FOCUS_PAGES = ["Focus A", "Focus B", "Later Page"];

function loadQueryDoc(queryRaw: string, kind?: "advanced") {
  // Whether a `{{query …}}` holds datalog is the ENGINE's reading of the text,
  // not a regex over it (§7.1), so a test that wants the advanced path says so.
  if (kind) {
    const argument = queryMacroExtent(queryRaw)?.argument ?? "";
    const readings: Record<string, { form: string; kind: "advanced" }> = {
      [argument]: { form: argument, kind },
    };
    for (const name of FOCUS_PAGES) {
      const substituted = argument.replace(/<%\s*current page\s*%>/gi, () => `[[${name}]]`);
      readings[substituted] = { form: substituted, kind };
    }
    backendReadsQueries(readings);
  }
  setDoc({
    byId: {
      query: node("query", queryRaw, null),
      todo: node("todo", "TODO Body", null),
      rowA: node("rowA", "RowA-Presented", null),
      rowB: node("rowB", "RowB-Presented", null),
    },
    pages: [page(["query", "todo"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

/** The text the engine was asked to run — `source.original`, which for a
 *  substituted dyvar is the substituted argument the execution re-parsed. */
function ranText(call: [Query, ...unknown[]] | undefined): string {
  const source = call?.[0].source;
  return source && source.kind !== "builder" ? source.original : "";
}

function groupsFor(...ids: string[]): RefGroup[] {
  return [{
    page: "Sheet",
    kind: "page",
    blocks: ids.map((id) => ({ id, raw: doc.byId[id]?.raw ?? id, collapsed: false, children: [] })),
  }];
}

function searchFor(id: "rowA" | "rowB"): QueryExecution {
  return {
    hits: [{
      entity: "block",
      page: "Sheet",
      kind: "page",
      block: { id, raw: doc.byId[id].raw, collapsed: false, children: [] },
      display_text: doc.byId[id].raw,
      evidence: [],
    }],
    diagnostics: [],
    explanation: { branches: [] },
    cancelled: false,
  };
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function settle(): Promise<void> {
  await tick();
  await tick();
  await tick();
}

describe("query `<% current page %>` dispatch to the focused pane (GH #301)", () => {
  it("substitutes the focused page at execution time, keeps the authored text literal, and re-runs on navigation — coalescing identical transitions", async () => {
    loadQueryDoc("{{query (and (page <% current page %>) (task TODO))}}");
    const runQuery = vi
      .spyOn(backend(), "queryRun")
      .mockImplementation(async () => blockRunResult(groupsFor("todo")));
    openPage("Focus A", "page");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settle();
      const callsA = runQuery.mock.calls.map(ranText);
      expect(callsA).toHaveLength(1);
      expect(callsA[0]).toContain("[[Focus A]]");
      expect(callsA[0]).not.toContain("<%");
      // Authoring text keeps the literal dyvar (execution-only substitution).
      expect(doc.byId.query.raw).toContain("<% current page %>");
      await vi.waitFor(() => expect(root.textContent).toContain("TODO Body")); // result presented

      openPage("Focus B", "page");
      await settle();
      const callsB = runQuery.mock.calls.map(ranText);
      expect(callsB).toHaveLength(2);
      expect(callsB[1]).toContain("[[Focus B]]");
      await vi.waitFor(() => expect(root.textContent).toContain("TODO Body"));

      // Same-page "navigation" (block anchor / reopen): no rerun.
      openPage("Focus B", "page");
      await settle();
      expect(runQuery).toHaveBeenCalledTimes(2);
    } finally {
      dispose();
    }
  });

  it("does NOT rerun a marker-less query on navigation (no global rerun)", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    const runQuery = vi
      .spyOn(backend(), "queryRun")
      .mockImplementation(async () => blockRunResult(groupsFor("plain")));
    openPage("Focus A", "page");
    const { dispose } = mount(() => <Block id="query" />);
    try {
      await settle();
      openPage("Focus B", "page");
      await settle();
      expect(runQuery).toHaveBeenCalledTimes(1);
    } finally {
      dispose();
    }
  });

  it("advanced queries substitute at execution too, while its owner-page param stays the query block's page (the :query-page binding)", async () => {
    loadQueryDoc("{{query [:find (pull ?b [*]) :in $ ?cp :where [?b :block/refs ?cp]] :inputs [<% current page %>]}}", "advanced");
    const runQuery = vi
      .spyOn(backend(), "queryRun")
      .mockImplementation(async () => blockRunResult(groupsFor("todo")));
    openPage("Focus A", "page");
    const { dispose } = mount(() => <Block id="query" />);
    try {
      await settle();
      const calls = runQuery.mock.calls;
      expect(calls).toHaveLength(1);
      expect(ranText(calls[0])).toContain("[[Focus A]]");
      expect(ranText(calls[0])).not.toContain("<%");
      // The execution context still binds the OWNER page: this form has no
      // `:inputs [:current-page]`, so it is not a focused-pane query (§4.4).
      expect(calls[0][2]).toEqual({ current_page: "Sheet" });
    } finally {
      dispose();
    }
  });

  it("binds an advanced :inputs [:current-page] query to the focused pane and reruns only for a new focused page", async () => {
    loadQueryDoc("{{query [:find (pull ?b [*]) :in $ ?current-page :where [?p :block/name ?current-page] [?b :block/refs ?p]] :inputs [:current-page]}}", "advanced");
    const runQuery = vi
      .spyOn(backend(), "queryRun")
      .mockResolvedValue(blockRunResult(groupsFor("todo"), { ran: ["current-page-ref"] }));
    openPage("Focus A", "page");
    const { dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() =>
        expect(runQuery.mock.calls.at(-1)?.[2]).toEqual({ current_page: "Focus A" })
      );
      const callsOnA = runQuery.mock.calls.length;

      openPage("Focus B", "page");
      await vi.waitFor(() =>
        expect(runQuery.mock.calls.at(-1)?.[2]).toEqual({ current_page: "Focus B" })
      );
      expect(runQuery.mock.calls.length).toBeGreaterThan(callsOnA);
      const callsOnB = runQuery.mock.calls.length;

      openPage("Focus B", "page");
      await settle();
      expect(runQuery).toHaveBeenCalledTimes(callsOnB);
    } finally {
      dispose();
    }
  });

  it("keeps an advanced query without the live keyword owner-bound and navigation-independent", async () => {
    loadQueryDoc('{{query {:query [:find (pull ?b [*]) :where (task ?b "TODO")] :inputs ["example :current-page"]}}}', "advanced");
    const runQuery = vi
      .spyOn(backend(), "queryRun")
      .mockResolvedValue(blockRunResult(groupsFor("todo"), { ran: ["task"] }));
    openPage("Focus A", "page");
    const { dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(runQuery).toHaveBeenCalled());
      expect(runQuery.mock.calls.every((call) => call[2]?.current_page === "Sheet")).toBe(true);
      const calls = runQuery.mock.calls.length;
      openPage("Focus B", "page");
      await settle();
      expect(runQuery).toHaveBeenCalledTimes(calls);
    } finally {
      dispose();
    }
  });

  it("a stale async result from the previous page never overwrites the latest page's result", async () => {
    loadQueryDoc("{{query (and (page <% current page %>) (task TODO))}}");
    const deferred = new Map<string, (groups: RefGroup[]) => void>();
    const runQuery = vi.spyOn(backend(), "queryRun").mockImplementation(
      (query: Query) =>
        new Promise<QueryResult>((resolve) => {
          deferred.set(ranText([query]), (groups) => resolve(blockRunResult(groups)));
        }),
    );
    openPage("Focus A", "page");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(runQuery).toHaveBeenCalledTimes(1), { timeout: 2000 });
      openPage("Focus B", "page");
      await vi.waitFor(() => expect(runQuery).toHaveBeenCalledTimes(2), { timeout: 2000 });
      // The latest page's fetch resolves FIRST; the stale fetch resolves after.
      deferred.get(ranText(runQuery.mock.calls[1]))!(groupsFor("rowB"));
      await settle();
      deferred.get(ranText(runQuery.mock.calls[0]))!(groupsFor("rowA"));
      await settle();
      expect(root.textContent).toContain("RowB-Presented");
      expect(root.textContent).not.toContain("RowA-Presented");
    } finally {
      dispose();
    }
  });

  it("a stale friendly-search completion cannot overwrite the latest search presentation", async () => {
    loadQueryDoc('{{query (search "<% current page %>")}}\ntine.view:: search');
    const deferred = new Map<string, (result: QueryExecution) => void>();
    const runGraphSearch = vi.spyOn(backend(), "runGraphSearch").mockImplementation(
      (query: string) => new Promise<QueryExecution>((resolve) => deferred.set(query, resolve)),
    );
    openPage("Focus A", "page");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(runGraphSearch).toHaveBeenCalledTimes(1));
      openPage("Focus B", "page");
      await vi.waitFor(() => expect(runGraphSearch).toHaveBeenCalledTimes(2));
      deferred.get(String(runGraphSearch.mock.calls[1][0]))!(searchFor("rowB"));
      await settle();
      deferred.get(String(runGraphSearch.mock.calls[0][0]))!(searchFor("rowA"));
      await settle();
      expect(root.textContent).toContain("RowB-Presented");
      expect(root.textContent).not.toContain("RowA-Presented");
    } finally {
      dispose();
    }
  });

  it("with no page in the focused route (the journals feed), the dyvar is left alone verbatim", async () => {
    loadQueryDoc("{{query (and (page <% current page %>) (task TODO))}}");
    const runQuery = vi
      .spyOn(backend(), "queryRun")
      .mockImplementation(async (query: Query) => blockRunResult(groupsFor(ranText([query]))));
    // route starts at journals: no focused page.
    expect(route().kind).toBe("journals");
    const { dispose } = mount(() => <Block id="query" />);
    try {
      await settle();
      const calls = runQuery.mock.calls.map(ranText);
      expect(calls).toHaveLength(1);
      expect(calls[0]).toContain("<% current page %>");
      // Navigating from journals to a real page NOW makes it live.
      openPage("Later Page", "page");
      await settle();
      expect(runQuery).toHaveBeenCalledTimes(2);
      expect(ranText(runQuery.mock.calls[1])).toContain("[[Later Page]]");
    } finally {
      dispose();
    }
  });


  // master's two "in a published export" tests are n/a: og has no published
  // query export (publish-query Stage 2 / ADR 0065 is master-only).

  it("outside an export, the sample note is not rendered (the builder sentence carries the sample)", async () => {
    loadQueryDoc("{{query (task TODO)}}");
    const argument = queryMacroExtent("{{query (task TODO)}}")?.argument ?? "";
    backendReadsQueries({ [argument]: { form: argument, view: { view: "list", sample: 1 } } });
    vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groupsFor("todo")));
    openPage("Focus A", "page");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("TODO Body"));
      expect(root.querySelector(".query-sample-note")).toBeNull();
    } finally {
      dispose();
    }
  });
});

// master's "a query block over a failed index (GH #594)" is X-class: og has no
// query index to fail.
