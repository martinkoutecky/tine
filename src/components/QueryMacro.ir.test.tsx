// Ported from master src/components/QueryMacro.ir.test.tsx.
// The seam between the block and the ONE query engine (SPEC §4.3.1, §7.1;
// I-9, I-12, I-20, I-4).
//
// Three things this file is the evidence for:
//
//  B1  A `{{tine-query …}}` block RENDERS ROWS. Before this packet the macro
//      called `run_query`, which cannot read TQL, so a TQL block drew its
//      header, its count and its controls and then nothing — and
//      `query_explain_empty` was decoded and never rendered, so the one moment a
//      user most needs to know WHICH conjunct emptied the query said only "No
//      results".
//  B4  The text pane. Debounced, last-good rows stay VISIBLE AND GREYED behind
//      the parser's own message, and a late answer for text the user has since
//      retyped is dropped rather than rendered (I-20).
//  B5  The save path, and the one caller entitled to `NotApplicable` (Q3): the
//      macro name is chosen from `query_og_expressible`, an OG refusal is
//      answered by switching dialect, and any OTHER refusal writes nothing and
//      shows the printer's own message (I-9).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { backend, QueryPrintRefusedError } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { blockProperty, resetStore, setRaw } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { RefGroup } from "../types";
import type { ExplainEmptyResult, ParsedQuery, Query, QueryResult, ViewSettings } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetTabsToJournals, route } from "../router";
import { tryFreezeGraphRewrite } from "../document/graphRewriteState";
import { toasts, setToasts } from "../toasts";
import { bumpDataRev } from "../graphSession";
import { QUERY_TEXT_OPEN_KEY } from "./QueryBuilder";

beforeAll(async () => {
  await initParser();
});

// GH #619 item 4: the query text is behind an "Edit as text" toggle, closed by default. These
// tests are about the text pane, so they open the toggle the way a user who wants the text has.
beforeEach(() => {
  localStorage.setItem(QUERY_TEXT_OPEN_KEY, "1");
});

afterEach(() => {
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  resetTabsToJournals();
  localStorage.clear();
  setToasts([]);
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const tick = () => wait(0);
async function settle(): Promise<void> {
  await tick();
  await tick();
  await tick();
}

function page(roots: string[], readOnly = false): FeedPage {
  return {
    name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots, format: "md", readOnly, guide: false,
  };
}

function node(id: string, raw: string): StoreNode {
  return { id, raw, collapsed: false, parent: null, page: "Sheet", children: [] };
}

function groups(): RefGroup[] {
  return [{
    page: "Sheet",
    kind: "page",
    blocks: [{ id: "todo", raw: "TODO A tracked row", collapsed: false, children: [] }],
  }];
}

function load(raw: string, { readOnly = false }: { readOnly?: boolean } = {}): void {
  setDoc({
    byId: { query: node("query", raw), todo: node("todo", "TODO A tracked row") },
    pages: [page(["query", "todo"], readOnly)],
    feed: ["Sheet"],
    loaded: true,
  });
}

/** A `{{tine-query …}}` block: TQL, which ONLY the IR evaluator can read. */
const TQL_MACRO = "{{tine-query -- task TODO}}";

describe("B1: a TQL block executes through query_run", () => {
  it("renders a scoped block board without a singular tine.view property", async () => {
    load("{{tine-query -- task TODO}}\ntine.block-view:: board");
    const parse = backend().parseQuery.bind(backend());
    vi.spyOn(backend(), "parseQuery").mockImplementation(async (...args) => ({
      ...await parse(...args), block_presentation: "board",
    }));
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".sheet-board")).not.toBeNull());
    } finally { dispose(); }
  });
  it("coalesces twenty identical visible query runs after a save revision", async () => {
    const ids = Array.from({ length: 20 }, (_, index) => `query-${index}`);
    setDoc({
      byId: Object.fromEntries([...ids.map((id) => [id, node(id, TQL_MACRO)]), ["todo", node("todo", "TODO A tracked row")]]),
      pages: [page([...ids, "todo"])], feed: ["Sheet"], loaded: true,
    });
    const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    const { root, dispose } = mount(() => <>{ids.map((id) => <Block id={id} />)}</>);
    try {
      await vi.waitFor(() => expect(root.querySelectorAll(".query-count")).toHaveLength(20));
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(1));
      bumpDataRev();
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));
    } finally {
      dispose();
    }
  });

  it("shows a rejected run without claiming an empty answer", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockRejectedValue(new Error("result-too-large"));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("result-too-large"));
      expect(root.textContent).not.toContain("No results");
    } finally {
      dispose();
    }
  });

  it("shows a parser diagnostic without claiming an empty answer", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue({
      ...blockRunResult([], { ran: [], ignored: [], supported: false }),
      diagnostics: [{ kind: "syntax", message: "Invalid condition", disabled: false }],
    });
    vi.spyOn(backend(), "parseQuery").mockResolvedValue({
      query: {
        anchor: "block", filter: { kind: "raw", text: "-- task TODO", diagnostic_kind: "syntax" },
        diagnostics: [{ kind: "syntax", message: "Invalid condition", disabled: false }],
        source: { kind: "tql", original: "-- task TODO", og_options: "" },
      },
      view: {},
    });
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("Invalid condition"));
      expect(root.textContent).not.toContain("No results");
    } finally {
      dispose();
    }
  });
  it("uses the exact page count and physical path returned by the shared page reader", async () => {
    load("{{tine-query @page and journal = false}}");
    const answer: QueryResult = {
      anchor: "page",
      pages: [{
        path: "pages/nested/Twin.md",
        name: "Twin",
        kind: "page",
        properties: [["rank", "first"]],
      }],
      diagnostics: [],
      report: { ran: ["journal"], ignored: [], supported: true },
      total: 1,
      matched_total: 43,
      exceeded: true,
    };
    vi.spyOn(backend(), "queryRun").mockResolvedValue(answer);

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".query-page-link")?.textContent).toContain("Twin"));
      expect(root.querySelector(".query-count")?.textContent).toBe("43");
      (root.querySelector(".query-page-link") as HTMLButtonElement).click();
      expect(route()).toMatchObject({
        kind: "page",
        name: "Twin",
        pageKind: "page",
        path: "pages/nested/Twin.md",
      });
    } finally {
      dispose();
    }
  });

  /** **A page-anchored answer is the SAME seven pages in every view** (GH #547).
   *
   *  `{{query (page-property tags gptpro)}}` answers with pages. List rendered
   *  them; Table and Board asked `groups()` — which a page-anchored run leaves
   *  empty by construction — and printed "No results", and Search counted
   *  `searchPresentationHits()`, which was built from those same absent blocks,
   *  so the header said 0 beside a summary group that said 7. Presentation never
   *  changes membership: the ONE page renderer already draws all four faces. */
  for (const view of ["table", "board", "search"] as const) {
    it(`renders the page rows a page-anchored query matched under tine.view:: ${view}`, async () => {
      load(`{{tine-query @page and journal = false}}\ntine.view:: ${view}`);
      const answer: QueryResult = {
        anchor: "page",
        pages: [
          { path: "pages/Twin.md", name: "Twin", kind: "page", properties: [["tags", "gptpro"]] },
          { path: "pages/Other.md", name: "Other", kind: "page", properties: [["tags", "gptpro"]] },
        ],
        diagnostics: [],
        report: { ran: ["journal"], ignored: [], supported: true },
        total: 2,
        matched_total: 2,
        exceeded: false,
      };
      vi.spyOn(backend(), "queryRun").mockResolvedValue(answer);

      const { root, dispose } = mount(() => <Block id="query" />);
      try {
        await vi.waitFor(() => expect(root.querySelectorAll(".query-page-link")).toHaveLength(2));
        expect(root.textContent).toContain("Twin");
        expect(root.textContent).toContain("Other");
        // The count the header shows is the count of the rows it is showing.
        expect(root.querySelector(".query-count")?.textContent).toBe("2");
        expect(root.querySelector(".query-empty")).toBeNull();
      } finally {
        dispose();
      }
    });
  }


  // master's "automatically retries indexing…" and "retains existing query group DOM
  // during a pending refresh" are X-class: og has no index readiness (QueryNotReadyError).


  it("renders its rows, and hands the evaluator the IR rather than a printed string", async () => {
    load(TQL_MACRO);
    const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      // The rows are the assertion: the legacy evaluator cannot read TQL, so a
      // block that renders its header and count but no rows is the exact defect.
      await vi.waitFor(() => expect(root.textContent).toContain("A tracked row"));
      expect(root.querySelector(".query-count")?.textContent).toBe("1");

      const [query, view] = run.mock.calls[0];
      expect(query.source).toMatchObject({ kind: "tql", original: "-- task TODO" });
      // The effective view of the reading's anchor (§7.6, Q3). A block with no
      // display settings at all still HAS a presentation — the resolver's
      // default — and sending the resolved view rather than the raw reading is
      // what makes a scoped one reach the executor at all.
      // og: the reading's own view goes to the evaluator (the scoped-display
      // resolver that turns `{}` into `{ view: "list" }` is Q4b's).
      expect(view).toEqual({});
    } finally {
      dispose();
    }
  });

  it("renders query_explain_empty when the run comes back empty (Q14, N19)", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
    const explain: ExplainEmptyResult = {
      rows: [
        { conjunct: "task = TODO", alone: 12, without: 0 },
        { conjunct: "page = [[Nowhere]]", alone: 0, without: 12 },
      ],
      diagnostics: [],
      report: { ran: [], ignored: [], supported: true },
    };
    const explainEmpty = vi.spyOn(backend(), "queryExplainEmpty").mockResolvedValue(explain);

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      // The affordance itself is the sign the run came back empty; `.query-empty`
      // alone also hosts "Loading query results…" before any answer exists.
      await vi.waitFor(() => expect(root.querySelector(".query-why-empty")).not.toBeNull());
      // Asked only once a run has actually come back empty — an ordinary query
      // still costs one command.
      expect(explainEmpty).not.toHaveBeenCalled();

      root.querySelector<HTMLButtonElement>(".query-why-empty")!.click();
      await vi.waitFor(() => expect(root.querySelector(".query-why-empty-panel")?.textContent).toContain("page = [[Nowhere]]"));
      const text = root.querySelector(".query-why-empty-panel")!.textContent ?? "";
      // The conjunct that matches nothing ALONE is the one that emptied it.
      expect(text).toContain("page = [[Nowhere]]");
      expect(text).toContain("task = TODO");
      expect(explainEmpty).toHaveBeenCalledTimes(1);
    } finally {
      dispose();
    }
  });
});

describe("why empty? describes an answer, not the absence of one", () => {
  // 2026-09-11: with the projection rebuilding, `query_parse` retried forever,
  // no reading existed, nothing ran, `total()` fell to 0 — and the block said
  // "No results" with a "why empty?" that opened a panel with nothing in it.
  // og: no index readiness; the same contract holds while the parse is still
  // pending (master's "shows the readiness state…" asserted "Rebuilding the
  // query index…", which og never shows).
  it("shows no affordance and no \"No results\" until a run has come back", async () => {
    load(TQL_MACRO);
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    vi.spyOn(backend(), "parseQuery").mockImplementation(async (text: string) => {
      await gate;
      return parsedAs(text);
    });
    const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
    const explainEmpty = vi.spyOn(backend(), "queryExplainEmpty");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settle();
      expect(root.querySelector(".query-why-empty")).toBeNull();
      expect(root.textContent).not.toContain("No results");
      expect(run).not.toHaveBeenCalled();
      expect(explainEmpty).not.toHaveBeenCalled();

      release();
      await vi.waitFor(() => expect(root.querySelector(".query-why-empty")).not.toBeNull(), { timeout: 4000 });
      expect(root.textContent).toContain("No results");
    } finally {
      dispose();
    }
  });
});

/** Open the editing sheet, as a user clicking the resting sentence's ⚙ does.
 *  The sheet is portalled to <body>, so its contents are queried from the
 *  document rather than from the block's own element. */
async function openSheet(root: HTMLElement): Promise<HTMLElement> {
  const gear = await vi.waitFor(() => {
    const found = root.querySelector<HTMLButtonElement>(".qs-gear");
    if (!found) throw new Error("the query sentence never appeared");
    return found;
  });
  if (!document.querySelector(".qs-sheet")) gear.click();
  return await vi.waitFor(() => {
    const sheet = document.querySelector<HTMLElement>(".qs-sheet");
    if (!sheet) throw new Error("the sheet never opened");
    return sheet;
  });
}

describe("Q4b: Display saves through the query block", () => {
  it("saves an explicit empty grouping when None is chosen from an unset view", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task TODO");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const sheet = await openSheet(root);
      sheet.querySelector<HTMLButtonElement>(".qd-trigger")!.click();
      await vi.waitFor(() => expect(sheet.querySelector(".qd-panel")).not.toBeNull());
      [...sheet.querySelectorAll<HTMLButtonElement>(".qd-panel button")]
        .find((button) => button.textContent === "None")!.click();
      await vi.waitFor(() => expect(blockProperty("query", "tine.group-field")).toBe(""));
    } finally { dispose(); }
  });

  it("offers Page as a table column and writes it on the query block (GH #606)", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task TODO");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const sheet = await openSheet(root);
      sheet.querySelector<HTMLButtonElement>(".qd-trigger")!.click();
      await vi.waitFor(() => expect(sheet.querySelector(".qd-panel")).not.toBeNull());
      const column = [...sheet.querySelectorAll<HTMLButtonElement>(".qd-add")]
        .find((button) => button.textContent?.includes("column"));
      column!.click();
      const page = [...sheet.querySelectorAll<HTMLButtonElement>(".qs-vocab-option")]
        .find((button) => button.textContent?.trim() === "Page");
      expect(page).toBeDefined();
      page!.click();
      await vi.waitFor(() => expect(blockProperty("query", "tine.columns")).toBe("page"));
      expect(blockProperty("query", "tine.fields")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("persists a query table's Page header sort in the same query view", async () => {
    load(`${TQL_MACRO}\ntine.view:: table`);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task TODO");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const header = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>('[data-sheet-field-header][data-sheet-field="page"]');
        if (!found) throw new Error("Page column was not rendered");
        return found;
      });
      header.click();
      await vi.waitFor(() => expect(blockProperty("query", "tine.sort")).toBe("page asc"));
    } finally { dispose(); }
  });
});

/** Reach the text pane in the sheet's footer. **P4 retired the disclosure**:
 *  inside an open sheet the pane is visible and editable, so opening the sheet
 *  IS opening the pane (§7.5). The resting sentence still mounts none of it. */
async function openPane(root: HTMLElement): Promise<HTMLTextAreaElement> {
  const sheet = await openSheet(root);
  return await vi.waitFor(() => {
    const input = sheet.querySelector<HTMLTextAreaElement>(".query-text-pane-input");
    if (!input) throw new Error("the pane has no input");
    return input;
  });
}

function type(input: HTMLTextAreaElement, text: string): void {
  input.value = text;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

/** A parse answer for arbitrary pane text: a `raw` capsule, which is what a
 *  reader that retained text without interpreting it honestly returns. */
function parsedAs(text: string): ParsedQuery {
  return {
    query: {
      anchor: "block",
      filter: { kind: "raw", text, diagnostic_kind: "not_applicable" },
      diagnostics: [],
      source: { kind: "tql", original: text, og_options: "" },
    },
    view: {},
  };
}

describe("B4: the query text pane", () => {
  it("debounces: a burst of keystrokes asks the engine once, for the last text", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task TODO");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const input = await openPane(root);
      const parse = vi.spyOn(backend(), "parseQuery").mockImplementation(async (text: string) =>
        parsedAs(text),
      );

      type(input, "-- task T");
      type(input, "-- task TO");
      type(input, "-- task TOD");
      type(input, "-- task DONE");
      expect(parse).not.toHaveBeenCalled();

      await wait(250);
      const paneCalls = parse.mock.calls.filter(([, dialect]) => dialect === "tql");
      expect(paneCalls).toHaveLength(1);
      expect(paneCalls[0][0]).toBe("-- task DONE");
    } finally {
      dispose();
    }
  });

  it("drops a late answer for text the user has since retyped (I-20)", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task TODO");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const input = await openPane(root);
      const settlers = new Map<string, { ok: (parsed: ParsedQuery) => void; fail: (e: Error) => void }>();
      vi.spyOn(backend(), "parseQuery").mockImplementation(
        (text: string) =>
          new Promise<ParsedQuery>((ok, fail) => settlers.set(text, { ok, fail })),
      );

      type(input, "-- task OLD");
      await wait(200);
      type(input, "-- task NEW");
      await wait(200);
      expect([...settlers.keys()]).toEqual(["-- task OLD", "-- task NEW"]);

      // The newest revision answers first; the stale one then fails. Rendering
      // that failure would read as "my correction was rejected".
      settlers.get("-- task NEW")!.ok(parsedAs("-- task NEW"));
      await settle();
      settlers.get("-- task OLD")!.fail(new Error("unbalanced parenthesis at 1:9"));
      await settle();

      expect(document.querySelector(".query-text-pane-error")).toBeNull();
      expect(root.querySelector(".query-block")?.classList.contains("query-stale")).toBe(false);
      expect(document.querySelector<HTMLButtonElement>(".query-text-pane-save")!.disabled).toBe(false);
    } finally {
      dispose();
    }
  });

  it("keeps the last-good rows visible and greyed under the parser's own message", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task TODO");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("A tracked row"));
      const input = await openPane(root);
      vi.spyOn(backend(), "parseQuery").mockRejectedValue(
        new Error("expected a comparison after `where`"),
      );

      type(input, "-- task ");
      await vi.waitFor(() =>
        expect(document.querySelector(".query-text-pane-error")?.textContent)
          .toBe("expected a comparison after `where`"),
      );

      // Not blanked: these rows are the last reading that RAN.
      expect(root.textContent).toContain("A tracked row");
      expect(root.querySelector(".query-block")?.classList.contains("query-stale")).toBe(true);
      // …and no spinner outliving the response.
      expect(document.querySelector(".query-text-pane-pending")).toBeNull();
      expect(document.querySelector<HTMLButtonElement>(".query-text-pane-save")!.disabled).toBe(true);
      expect(document.querySelector(".query-text-pane-error")?.getAttribute("role")).toBe("alert");
    } finally {
      dispose();
    }
  });

  it("says the options map is edited elsewhere instead of asking the engine to parse one", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task TODO");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const input = await openPane(root);
      const parse = vi.spyOn(backend(), "parseQuery").mockImplementation(async (text: string) =>
        parsedAs(text),
      );

      type(input, '-- task TODO {:title "Open"}');
      await wait(250);

      expect(document.querySelector(".query-text-pane-error")?.textContent).toContain("options map");
      // Splitting a macro argument is Rust's job and only Rust's — the pane makes
      // no claim about where the map starts, and asks nothing.
      expect(parse.mock.calls.filter(([, dialect]) => dialect === "tql")).toHaveLength(0);
    } finally {
      dispose();
    }
  });
});

/** The dialects the SAVE path asked for. The pane prints too — that is how it
 *  shows the query's text — so the macro dialects are what identify a save. */
function savePrintDialects(print: { mock: { calls: unknown[][] } }): string[] {
  return print.mock.calls
    .map((call) => String(call[2]))
    .filter((dialect) => dialect === "og" || dialect === "tql_macro" || dialect === "advanced_macro");
}

/** Drive one save through the pane: type valid text, wait for its parse, click
 *  "Save query text". */
async function saveThroughPane(root: HTMLElement, text: string): Promise<void> {
  const input = await openPane(root);
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (source: string) => parsedAs(source));
  type(input, text);
  const save = await vi.waitFor(() => {
    const button = document.querySelector<HTMLButtonElement>(".query-text-pane-save");
    if (!button || button.disabled) throw new Error("save is not enabled yet");
    return button;
  });
  save.click();
  await settle();
}

describe("B5: the save path chooses the name and answers NotApplicable", () => {
  it("reports a frozen rewrite refusal without changing the block or showing a crossing notice", async () => {
    load("{{query (task TODO)}}");
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task DONE");
    const { root, dispose } = mount(() => <Block id="query" />);
    let release: (() => void) | null = null;
    try {
      const input = await openPane(root);
      vi.spyOn(backend(), "parseQuery").mockImplementation(async (source: string) => parsedAs(source));
      type(input, "-- task DONE");
      const save = await vi.waitFor(() => {
        const found = document.querySelector<HTMLButtonElement>(".query-text-pane-save");
        if (!found || found.disabled) throw new Error("save unavailable");
        return found;
      });
      release = tryFreezeGraphRewrite();
      expect(release).not.toBeNull();
      save.click();
      await vi.waitFor(() => expect(root.textContent).toContain("not changed"));
      expect(doc.byId.query.raw).toBe("{{query (task TODO)}}");
      expect(root.querySelector(".query-crossing-notice")).toBeNull();
    } finally {
      release?.();
      dispose();
    }
  });
  it("edits the second of two identical macros without changing the first", async () => {
    load("{{query (task TODO)}} and {{query (task TODO)}}");
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task DONE)");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const gears = await vi.waitFor(() => {
        const found = root.querySelectorAll<HTMLButtonElement>(".qs-gear");
        if (found.length !== 2) throw new Error(`expected two query sheets, got ${found.length}`);
        return found;
      });
      gears[1].click();
      const input = await vi.waitFor(() => {
        const found = document.querySelector<HTMLTextAreaElement>(".query-text-pane-input");
        if (!found) throw new Error("second query pane is absent");
        return found;
      });
      vi.spyOn(backend(), "parseQuery").mockImplementation(async (source: string) => parsedAs(source));
      type(input, "-- task DONE");
      const save = await vi.waitFor(() => {
        const found = document.querySelector<HTMLButtonElement>(".query-text-pane-save");
        if (!found || found.disabled) throw new Error("save is not enabled");
        return found;
      });
      save.click();
      await vi.waitFor(() => expect(doc.byId.query.raw).toBe("{{query (task TODO)}} and {{query (task DONE)}}"));
    } finally {
      dispose();
    }
  });
  it("refuses an in-flight edit when the authored macro's raw extent moves", async () => {
    const original = "{{query (task TODO)}} and {{query (task TODO)}}";
    load(original);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    let finishPrint: ((text: string) => void) | undefined;
    vi.spyOn(backend(), "printQuery").mockImplementation(async (_query, _view, dialect) =>
      dialect === "og" ? new Promise<string>((resolve) => { finishPrint = resolve; }) : "-- task TODO");
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const gears = await vi.waitFor(() => {
        const found = root.querySelectorAll<HTMLButtonElement>(".qs-gear");
        if (found.length !== 2) throw new Error("two macros not rendered");
        return found;
      });
      gears[1].click();
      const input = await vi.waitFor(() => {
        const found = document.querySelector<HTMLTextAreaElement>(".query-text-pane-input");
        if (!found) throw new Error("pane not open");
        return found;
      });
      vi.spyOn(backend(), "parseQuery").mockImplementation(async (source: string) => parsedAs(source));
      type(input, "-- task DONE");
      const save = await vi.waitFor(() => {
        const found = document.querySelector<HTMLButtonElement>(".query-text-pane-save");
        if (!found || found.disabled) throw new Error("save not ready");
        return found;
      });
      save.click();
      await vi.waitFor(() => expect(finishPrint).toBeDefined());
      setRaw("query", `prefix ${original}`);
      finishPrint!("(task DONE)");
      await settle();
      expect(doc.byId.query.raw).toBe(`prefix ${original}`);
      expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("changed"))).toBe(true);
    } finally {
      dispose();
    }
  });
  it("keeps {{query}} for an OG-expressible edit, and an empty view adds no property line", async () => {
    load('{{query (task TODO)}}');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    const print = vi.spyOn(backend(), "printQuery").mockResolvedValue("(task DONE)");

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      expect(doc.byId.query.raw).toBe("{{query (task DONE)}}");
      expect(savePrintDialects(print)).toEqual(["og"]);
      // Every save now writes the six §7.6 view properties (T4) — but clearing a
      // property a block never had is the identity, so a view with nothing in it
      // still leaves the block's bytes as the macro line and nothing else.
      expect(doc.byId.query.raw).not.toContain("tine.");
    } finally {
      dispose();
    }
  });

  it("crosses a {{query}} block to {{tine-query}} when OG cannot express it, carrying the view (Y2)", async () => {
    load('{{query (task TODO)}}');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
    const print = vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task DONE");
    // The pane's parse answers with the session's own view, so state one worth
    // carrying: TQL text has nowhere to put it (§4.3 Y2).
    vi.spyOn(backend(), "parseQuery").mockImplementation(async (text: string) => ({
      query: parsedAs(text).query,
      view: { sort: [["updated", "desc"]], sample: 20, group_by: "page", aggregates: [["", "count"]] },
    }));

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      await vi.waitFor(() => expect(doc.byId.query.raw).toContain("{{tine-query -- task DONE}}"));
      expect(savePrintDialects(print)).toEqual(["tql_macro"]);
      expect(blockProperty("query", "tine.sort")).toBe("updated desc");
      expect(blockProperty("query", "tine.sample")).toBe("20");
      expect(blockProperty("query", "tine.group-field")).toBe("page");
      // X3/W5: the whole-result count is a bare `count` segment, with no `=`.
      expect(blockProperty("query", "tine.col-aggregates")).toBe("count");
    } finally {
      dispose();
    }
  });

  it("answers a NotApplicable refusal by switching dialect, not by showing it", async () => {
    load('{{query (task TODO)}}');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    // `og_expressible` said yes; the printer refuses anyway. This is the ONE
    // caller entitled to see `NotApplicable`.
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    const print = vi.spyOn(backend(), "printQuery").mockImplementation(async (
      _query: Query,
      _view,
      dialect: string,
    ) => {
      if (dialect === "og") {
        throw new QueryPrintRefusedError("not_applicable", {
          kind: "not_applicable",
          message: "the OG DSL cannot express a regex comparison",
          suggestions: [],
          disabled: false,
        });
      }
      return "-- content ~ /x/"; // `tql_macro` for the save, `tql` for the pane
    });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- content ~ /x/");
      await vi.waitFor(() => expect(doc.byId.query.raw).toBe("{{tine-query -- content ~ /x/}}"));
      expect(savePrintDialects(print)).toEqual(["og", "tql_macro"]);
      // The user is never shown a refusal for a query Tine can perfectly well
      // store.
      expect(document.querySelector(".query-print-refused")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("writes nothing and renders the printer's own message for any other refusal (I-9, I-4)", async () => {
    load('{{query (task TODO)}}');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    // Only the MACRO print refuses. The pane keeps showing the query's text, or
    // there would be nothing to type into and the refusal would be untestable.
    vi.spyOn(backend(), "printQuery").mockImplementation(async (_query, _view, dialect: string) => {
      if (dialect === "tql") return "-- task TODO";
      throw new QueryPrintRefusedError("syntax", {
        kind: "syntax",
        message: "a `}}` in the query would end the macro",
        suggestions: [],
        disabled: false,
      });
    });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const before = doc.byId.query.raw;
      await saveThroughPane(root, "-- content = '}}'");
      expect(doc.byId.query.raw).toBe(before);
      const refusal = await vi.waitFor(() => {
        const found = document.querySelector(".query-print-refused");
        if (!found) throw new Error("no refusal was shown");
        return found;
      });
      expect(refusal.getAttribute("role")).toBe("alert");
      expect(refusal.textContent).toContain("a `}}` in the query would end the macro");
    } finally {
      dispose();
    }
  });
});

/** §4.3 "Directive migration" (Q15, T4).
 *
 *  A block that STAYS `{{query}}` used to lose every view edit that OG cannot
 *  say and silently duplicate the ones it can: `writeViewProperties` fired only
 *  on the crossing to `{{tine-query}}`, and the OG printer re-emitted
 *  `(aggregate …)`/`(group-by …)` into the text. Now every save writes all six
 *  §7.6 properties for both macro names, aggregates and grouping leave the DSL
 *  text, and `sort-by`/`sample` stay in the text as well as in the properties
 *  because OG itself reads those.
 */
describe("B6: directive migration for blocks that stay {{query}}", () => {
  /** A parse whose view is `view`, so a save has something to persist. */
  function parseWithView(view: ViewSettings): void {
    vi.spyOn(backend(), "parseQuery").mockImplementation(async (text: string) => ({
      query: parsedAs(text).query,
      view,
    }));
  }

  it("writes tine.col-aggregates on a NON-crossing save and leaves (aggregate …) out of the text", async () => {
    load('{{query (task TODO)}}');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task DONE)");
    // The engine's merged `group_by` is a canonical field id (P5B): a bare
    // `status` is the ORDINARY PROPERTY, spelled `prop:status`.
    parseWithView({ aggregates: [["", "count"], ["hours", "sum"]], group_by: "prop:status" });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      await vi.waitFor(() =>
        expect(blockProperty("query", "tine.col-aggregates")).toBe("count;hours=sum"),
      );
      expect(blockProperty("query", "tine.group-field")).toBe("prop:status");
      // The block stayed `{{query}}` — the migration is about WHERE the view
      // lives, not about the macro name.
      expect(doc.byId.query.raw).toContain("{{query (task DONE)}}");
      expect(doc.byId.query.raw).not.toContain("(aggregate");
      expect(doc.byId.query.raw).not.toContain("(group-by");
    } finally {
      dispose();
    }
  });

  // Not og's design (og E, family 3, verified 2026-09-29): master's save writes the
  // full six-field `queryViewPropertyPatch` against the persisted block; og's save
  // (`materializeView`) writes only the facts its dialect's reprint would drop, and a
  // Display edit writes its own complete patch (`displayPropertyPatch`, tested in
  // QueryDisplay/QueryWorkspace tests). The observable outcomes differ only where the
  // test's mocked parse invents a view no real block can carry (columns/view kind live
  // only in properties, so a filter save cannot lose them) or where og writes no
  // redundant `tine.sort::` beside a `(sort-by …)` OG spelling. Left skipped on purpose.
  it.skip("keeps (sort-by a desc) in the OG text AND gains tine.sort:: a desc", async () => {
    load('{{query (task TODO) (sort-by a desc)}}');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO) (sort-by a desc)");
    parseWithView({ sort: [["a", "desc"]] });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task TODO");
      await vi.waitFor(() => expect(blockProperty("query", "tine.sort")).toBe("a desc"));
      // Q15: OG reads `sort-by`, so it is re-emitted as well as persisted.
      expect(doc.byId.query.raw).toContain("(sort-by a desc)");
    } finally {
      dispose();
    }
  });

  // Not og's design (og E, family 3, verified 2026-09-29): master's save writes the
  // full six-field `queryViewPropertyPatch` against the persisted block; og's save
  // (`materializeView`) writes only the facts its dialect's reprint would drop, and a
  // Display edit writes its own complete patch (`displayPropertyPatch`, tested in
  // QueryDisplay/QueryWorkspace tests). The observable outcomes differ only where the
  // test's mocked parse invents a view no real block can carry (columns/view kind live
  // only in properties, so a filter save cannot lose them) or where og writes no
  // redundant `tine.sort::` beside a `(sort-by …)` OG spelling. Left skipped on purpose.
  it.skip("drops tine.sort when the sort is removed, so the removal survives a reparse", async () => {
    load('{{query (task TODO)}}\ntine.sort:: a desc');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task DONE)");
    // The builder removed the sort: the session's view no longer carries one.
    parseWithView({});

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      expect(blockProperty("query", "tine.sort")).toBe("a desc");
      await saveThroughPane(root, "-- task DONE");
      // `tine.*` has ABSOLUTE precedence over the DSL text, so a stale property
      // line would put the removed sort straight back on the next parse.
      await vi.waitFor(() => expect(blockProperty("query", "tine.sort")).toBeNull());
      expect(doc.byId.query.raw).not.toContain("tine.sort");
    } finally {
      dispose();
    }
  });

  // Not og's design (og E, family 3, verified 2026-09-29): master's save writes the
  // full six-field `queryViewPropertyPatch` against the persisted block; og's save
  // (`materializeView`) writes only the facts its dialect's reprint would drop, and a
  // Display edit writes its own complete patch (`displayPropertyPatch`, tested in
  // QueryDisplay/QueryWorkspace tests). The observable outcomes differ only where the
  // test's mocked parse invents a view no real block can carry (columns/view kind live
  // only in properties, so a filter save cannot lose them) or where og writes no
  // redundant `tine.sort::` beside a `(sort-by …)` OG spelling. Left skipped on purpose.
  it.skip("persists all six §7.6 fields, so the block re-parses to the saved view", async () => {
    load('{{query (task TODO)}}');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task DONE)");
    const saved: ViewSettings = {
      view: "table",
      sort: [["a", "desc"]],
      group_by: "prop:status",
      columns: ["a", "b"],
      aggregates: [["hours", "sum"]],
      sample: 20,
    };
    parseWithView(saved);

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      await vi.waitFor(() => expect(blockProperty("query", "tine.view")).toBe("table"));
      // `tine.columns::` and `tine.view::` are the two the reader consumes and
      // nobody wrote: before T4 a saved column set or view kind was simply lost.
      // P5A moved the column set off `tine.fields`, which is the TYPED SCHEMA
      // and is never written by a query save.
      expect(blockProperty("query", "tine.columns")).toBe("a;b");
      expect(blockProperty("query", "tine.fields")).toBeNull();
      expect(blockProperty("query", "tine.sort")).toBe("a desc");
      expect(blockProperty("query", "tine.group-field")).toBe("prop:status");
      expect(blockProperty("query", "tine.col-aggregates")).toBe("hours=sum");
      expect(blockProperty("query", "tine.sample")).toBe("20");
      // The properties the block now carries are exactly the ones the engine is
      // handed back on the next parse (§4.1 precedence merge).
      const properties = vi.mocked(backend().parseQuery).mock.calls.at(-1)![2] ?? [];
      const asMap = Object.fromEntries(properties);
      expect(asMap["tine.view"]).toBe("table");
      expect(asMap["tine.columns"]).toBe("a;b");
    } finally {
      dispose();
    }
  });

  it("leaves an untouched block byte-identical (no save, no property lines)", async () => {
    load('{{query (task TODO)}}');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
    parseWithView({ sort: [["a", "desc"]], aggregates: [["", "count"]] });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const before = doc.byId.query.raw;
      await vi.waitFor(() => expect(root.querySelector(".query-block")).not.toBeNull());
      await settle();
      // I-4: rendering a query is not editing it. Nothing is written until the
      // user saves.
      expect(doc.byId.query.raw).toBe(before);
    } finally {
      dispose();
    }
  });

  it("writes nothing at all when the page is not writable", async () => {
    load('{{query (task TODO)}}', { readOnly: true });
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task DONE)");
    parseWithView({ sort: [["a", "desc"]] });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const before = doc.byId.query.raw;
      await saveThroughPane(root, "-- task DONE").catch(() => undefined);
      await settle();
      // The whole save is one undo unit now, and `withUndoUnit` returns without
      // running its body on a non-writable page — so the macro rewrite and the
      // property writes are refused TOGETHER rather than half-applied.
      expect(doc.byId.query.raw).toBe(before);
      expect(blockProperty("query", "tine.sort")).toBeNull();
    } finally {
      dispose();
    }
  });
});

// B7 (P4). What the block SAYS when the engine understood only part of the
// query. `REG-P0-QUERY-UNKNOWN-HEAD-001` fixed the Rust half — an unknown head
// is a `Raw` leaf with a diagnostic and the query returns nothing rather than a
// silently narrower answer. The frontend half is the sentence above the empty
// result: it must say the query returned NO results, and it must not say the
// part was "ignored", which is exactly the reading ("the rest still ran, and
// these are its rows") that the Rust fix exists to prevent.
describe("B7: an unknown head reads as 'returned no results', never as 'ignored'", () => {
  it("leads the run's diagnostics with what actually happened", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue({
      ...blockRunResult([]),
      diagnostics: [
        { message: "`frobnicate` isn't something Tine can query", kind: "unknown_head", span: { start: 6, end: 16 } },
      ],
    });
    vi.spyOn(backend(), "queryExplainEmpty").mockResolvedValue({
      rows: [],
      diagnostics: [],
      report: { ran: [], ignored: [], supported: true },
    });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const note = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>(".query-diagnostics");
        if (!found) throw new Error("the diagnostics note never appeared");
        return found;
      });
      expect(note.getAttribute("role")).toBe("alert");
      expect(note.textContent).toContain("returned no results");
      expect(note.querySelector(".query-diagnostics-lead")?.textContent?.trim()).toBe(
        "Tine didn't understand part of this query, so it returned no results:",
      );
      expect(note.textContent).toContain("`frobnicate` isn't something Tine can query");
      // The word this test exists for.
      expect(note.textContent?.toLowerCase()).not.toContain("ignored");
    } finally {
      dispose();
    }
  });

  it("stays silent for a diagnostic that is inside an `off` subtree (§3.5)", async () => {
    load(TQL_MACRO);
    vi.spyOn(backend(), "queryRun").mockResolvedValue({
      ...blockRunResult(groups()),
      diagnostics: [
        { message: "a disabled condition never ran", kind: "syntax", disabled: true },
      ],
    });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("A tracked row"));
      // The rows are real: a disabled diagnostic does not invalidate, so
      // announcing "returned no results" over a result that HAS rows would be
      // the opposite lie.
      expect(root.querySelector(".query-diagnostics")).toBeNull();
    } finally {
      dispose();
    }
  });
});
