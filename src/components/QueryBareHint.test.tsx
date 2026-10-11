// GH #422, D-18: a Logseq query whose bare `(task)` / `(priority)` adds no
// condition, or which has no condition at all, says so ON the query, and its
// buttons rewrite the clause to explicit markers through the ordinary query
// save (engine print -> block bytes, one undo unit).
//
// The engine owns the detection and the rewritten queries (`wire_parse.rs`
// `og_query_hint`, pinned by `the_og_hint_names_bare_heads_and_offers_
// round_tripping_rewrites`); here the backend is the jsdom stub, and what is
// under test is the host: the hint shows beside the results without replacing
// them, each button saves exactly the engine's rewrite of THAT query, the save
// is one ordinary undo unit, and a host that cannot write offers no buttons.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { setToasts } from "../toasts";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetStore, undo } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { bumpGraphEpoch } from "../graphSession";
import type { Filter, OgQueryHint, ParsedQuery, Query } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setToasts([]);
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

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await wait(0);
}

function load(raw: string, readOnly = false): void {
  const page: FeedPage = {
    name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots: ["query"], format: "md", readOnly, guide: false,
  };
  const node: StoreNode = { id: "query", raw, collapsed: false, parent: null, page: "Sheet", children: [] };
  setDoc({ byId: { query: node }, pages: [page], feed: ["Sheet"], loaded: true });
}

const list = (attr: "task" | "priority", items: string[]): Filter => ({
  kind: "leaf",
  leaf: { kind: "attr", attr, op: "in", value: { kind: "list", items: items.map((text) => ({ kind: "text", text })) } },
});
const og = (original: string, filter: Filter): Query => ({
  anchor: "block",
  filter,
  diagnostics: [],
  source: { kind: "og", original, og_options: "" },
});

const OPEN = ["TODO", "DOING", "NOW", "LATER", "WAITING", "WAIT", "STARTED", "IN-PROGRESS"];
const ANY = ["TODO", "DOING", "DONE", "NOW", "LATER", "WAITING", "WAIT", "CANCELED", "CANCELLED", "STARTED", "IN-PROGRESS"];

/** The readings the engine gives for each macro argument this file uses. */
function reading(argument: string): ParsedQuery {
  const text = argument.trim();
  const hinted = (filter: Filter, hint: OgQueryHint): ParsedQuery => ({ query: og(text, filter), view: {}, og_hint: hint });
  switch (text) {
    case "(task)":
      return hinted(list("task", []), {
        bare: ["task"],
        no_conditions: true,
        rewrites: [
          { rewrite: "open_tasks", query: og(text, list("task", OPEN)) },
          { rewrite: "any_task", query: og(text, list("task", ANY)) },
        ],
      });
    case "(priority)":
      return hinted(list("priority", []), {
        bare: ["priority"],
        no_conditions: true,
        rewrites: [{ rewrite: "priorities", query: og(text, list("priority", ["A", "B", "C"])) }],
      });
    case "":
      return { query: { ...og(text, { kind: "true" }), anchor: "page" }, view: {}, og_hint: { bare: [], no_conditions: true, rewrites: [] } };
    default:
      return { query: og(text, list("task", text.replace(/^\(task |\)$/g, "").split(" "))), view: {} };
  }
}

/** The engine's OG print of a query: the explicit form of its one leaf. */
function printOf(query: Query): string {
  const filter = query.filter;
  if (filter.kind !== "leaf" || filter.leaf.kind !== "attr" || filter.leaf.value.kind !== "list") throw new Error("unexpected print");
  const items = filter.leaf.value.items.map((item) => (item.kind === "text" ? item.text : "?"));
  return `(${filter.leaf.attr}${items.length ? ` ${items.join(" ")}` : ""})`;
}

function arrange(): { print: ReturnType<typeof vi.fn> } {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (source: string) => reading(source));
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  const print = vi.fn(async (query: Query) => printOf(query));
  vi.spyOn(backend(), "printQuery").mockImplementation(print);
  bumpGraphEpoch();
  return { print };
}

async function hint(root: HTMLElement): Promise<HTMLElement> {
  return await vi.waitFor(() => {
    const found = root.querySelector<HTMLElement>(".query-og-hint");
    if (!found) throw new Error("the hint never appeared");
    return found;
  });
}
const buttons = (shown: HTMLElement) =>
  [...shown.querySelectorAll<HTMLButtonElement>(".query-og-hint-rewrite")].map((b) => b.textContent);
const press = (shown: HTMLElement, label: string) =>
  [...shown.querySelectorAll<HTMLButtonElement>(".query-og-hint-rewrite")].find((b) => b.textContent === label)!.click();
const raw = () => doc.byId.query.raw;

describe("GH #422: the on-query hint for Logseq's no-condition forms", () => {
  it("names a bare (task), keeps the results on screen, and rewrites to the open markers in one undo unit", async () => {
    load("{{query (task)}}");
    const { print } = arrange();
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const shown = await hint(root);
      expect(shown.textContent).toContain("(task) without markers adds no condition, as in Logseq.");
      expect(shown.textContent).toContain("On its own it matches nothing.");
      expect(buttons(shown)).toEqual(["Open tasks", "Any task"]);
      // A cue, not a replacement: the block's own answer is still drawn.
      await vi.waitFor(() => expect(root.querySelector(".query-empty")?.textContent).toContain("No results"));

      press(shown, "Open tasks");
      await vi.waitFor(() => expect(raw()).toBe(`{{query (task ${OPEN.join(" ")})}}`));
      // The bytes are the engine's print of exactly the engine's rewrite.
      expect(print).toHaveBeenCalledWith(og("(task)", list("task", OPEN)), {}, "og");
      expect(undo()).toBe(true);
      expect(raw()).toBe("{{query (task)}}");
    } finally {
      dispose();
    }
  });

  it("Any task writes every marker", async () => {
    load("{{query (task)}}");
    arrange();
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      press(await hint(root), "Any task");
      await vi.waitFor(() => expect(raw()).toBe(`{{query (task ${ANY.join(" ")})}}`));
    } finally {
      dispose();
    }
  });

  it("a bare (priority) offers A, B or C", async () => {
    load("{{query (priority)}}");
    arrange();
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const shown = await hint(root);
      expect(shown.textContent).toContain("(priority) without levels adds no condition, as in Logseq.");
      expect(buttons(shown)).toEqual(["A, B or C"]);
      press(shown, "A, B or C");
      await vi.waitFor(() => expect(raw()).toBe("{{query (priority A B C)}}"));
    } finally {
      dispose();
    }
  });

  it("an empty query says it shows nothing, with nothing to rewrite", async () => {
    load("{{query }}");
    arrange();
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const shown = await hint(root);
      expect(shown.textContent).toContain("This query has no conditions, so it shows nothing, as in Logseq.");
      expect(buttons(shown)).toEqual([]);
    } finally {
      dispose();
    }
  });

  it("a read-only page shows the cue but offers no rewrite", async () => {
    load("{{query (task)}}", true);
    arrange();
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const shown = await hint(root);
      expect(shown.textContent).toContain("adds no condition");
      expect(buttons(shown)).toEqual([]);
      expect(raw()).toBe("{{query (task)}}");
    } finally {
      dispose();
    }
  });

  it("an explicit query has no hint", async () => {
    load("{{query (task TODO)}}");
    arrange();
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".query-block")).not.toBeNull());
      await settle();
      expect(root.querySelector(".query-og-hint")).toBeNull();
    } finally {
      dispose();
    }
  });
});
