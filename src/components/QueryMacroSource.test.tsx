// The macro's SOURCE bytes, end to end (SPEC §4.3.1, §7.1, §7.9; I-4, I-12).
//
// Three properties this packet is responsible for, each asserted through the
// component rather than at a helper:
//
//  1. The argument the engine is asked to read is the exact slice of the block's
//     raw source — not the document parser's `args`, which comma-split it and
//     stop before the first `}`.
//  2. A title edit re-emits the AUTHORED form (`preserveForm`), so renaming a
//     query cannot rewrite its filter.
//  3. A refused print writes nothing and says so. No catch-all turns a refusal
//     into a silent no-op.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { backend, QueryPrintRefusedError } from "../backend";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { RefGroup } from "../types";
import { backendReadsQueries } from "../tests/queryReadingsTestkit";
import { blockRunResult } from "../tests/queryReadingsTestkit";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}
async function settle(): Promise<void> {
  await tick();
  await tick();
}

function page(roots: string[]): FeedPage {
  return {
    name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots, format: "md", readOnly: false, guide: false,
  };
}
function node(id: string, raw: string): StoreNode {
  return { id, raw, collapsed: false, parent: null, page: "Sheet", children: [] };
}
function groups(): RefGroup[] {
  return [{
    page: "Sheet",
    kind: "page",
    blocks: [{ id: "hit", raw: "TODO A row", collapsed: false, children: [] }],
  }];
}

function load(raw: string): void {
  setDoc({
    byId: { query: node("query", raw), hit: node("hit", "TODO A row") },
    pages: [page(["query", "hit"])],
    feed: ["Sheet"],
    loaded: true,
  });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
}

// The argument that survives only if the RAW source is read: a literal comma
// inside a string (the document parser splits arguments on commas) and an
// options map (it stops before the first `}`).
const HOSTILE_ARGUMENT = '(and (task TODO) "a, b") {:title "Open, work"}';

describe("a query macro is read from the block's raw source", () => {
  it("hands the engine the exact bytes, options map and literal comma included", async () => {
    load(`Tasks: {{query ${HOSTILE_ARGUMENT}}} — see above`);
    backendReadsQueries({
      [HOSTILE_ARGUMENT]: { form: '(and (task TODO) "a, b")', opts: '{:title "Open, work"}' },
    });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settle();
      // The stub throws for any argument it was not told about, so reaching the
      // title at all is the assertion: a `name,args` rejoin would have asked for
      // `(and (task TODO) "a` + `b") {:title "Open` + `work"` instead.
      expect(backend().parseQuery).toHaveBeenCalledWith(HOSTILE_ARGUMENT, "macro_query", []);
      await vi.waitFor(() =>
        expect(root.querySelector(".query-title")?.textContent).toBe("Open, work"),
      );
      // …and nothing of the macro leaks into the rendered text as literal source.
      expect(root.textContent).not.toContain("{{query");
      expect(root.textContent).not.toContain(":title");
    } finally {
      dispose();
    }
  });

  it("reads the SECOND macro of a block as itself, not as the first", async () => {
    load('{{query (task TODO)}} and {{query (task DONE) {:title "Done"}}}');
    backendReadsQueries({
      "(task TODO)": { form: "(task TODO)" },
      '(task DONE) {:title "Done"}': { form: "(task DONE)", opts: '{:title "Done"}' },
    });

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await settle();
      const titles = [...root.querySelectorAll(".query-title")].map((el) => el.textContent);
      expect(titles).toEqual(["Query", "Done"]);
    } finally {
      dispose();
    }
  });
});

describe("editing a query's title preserves its authored form (§4.3.1)", () => {
  const ARGUMENT = '(and (task TODO) "a, b") {:title "Open, work"}';

  async function openTitleEditor(): Promise<{ root: HTMLDivElement; dispose: () => void }> {
    load(`{{query ${ARGUMENT}}}`);
    backendReadsQueries({
      [ARGUMENT]: { form: '(and (task TODO) "a, b")', opts: '{:title "Open, work"}' },
      // A successful rename rewrites the macro, so the engine is asked to read
      // the new text too.
      '(and (task TODO) "a, b") {:title "Renamed"}': {
        form: '(and (task TODO) "a, b")',
        opts: '{:title "Renamed"}',
      },
    });
    const mounted = mount(() => <Block id="query" />);
    await settle();
    (mounted.root.querySelector(".query-title") as HTMLElement).click();
    await settle();
    return mounted;
  }

  it("prints with preserveForm and writes back exactly what the printer returned", async () => {
    const printQuery = vi
      .spyOn(backend(), "printQuery")
      .mockResolvedValue('(and (task TODO) "a, b") {:title "Renamed"}');
    const { root, dispose } = await openTitleEditor();
    try {
      const input = root.querySelector(".query-title-input") as HTMLInputElement;
      input.value = "Renamed";
      input.dispatchEvent(new FocusEvent("blur"));
      await settle();

      const [query, , dialect, preserveForm] = printQuery.mock.calls[0];
      expect(preserveForm).toBe(true);
      expect(dialect).toBe("og");
      // Only the OPTIONS map changed; `source.original` — the author's own form —
      // went back to the printer untouched, which is what makes a rename
      // incapable of rewriting a filter the engine only partly understood.
      expect(query.source).toEqual({
        kind: "og",
        original: '(and (task TODO) "a, b")',
        og_options: '{:title "Renamed"}',
      });
      expect(doc.byId.query.raw).toBe('{{query (and (task TODO) "a, b") {:title "Renamed"}}}');
    } finally {
      dispose();
    }
  });

  it("writes NOTHING and says why when the printer refuses (I-4)", async () => {
    vi.spyOn(backend(), "printQuery").mockRejectedValue(
      new QueryPrintRefusedError("syntax", {
        kind: "syntax",
        message: "the title would not survive the macro boundary",
        suggestions: [],
        disabled: false,
      }),
    );
    const { root, dispose } = await openTitleEditor();
    try {
      const before = doc.byId.query.raw;
      const input = root.querySelector(".query-title-input") as HTMLInputElement;
      input.value = "Renamed";
      input.dispatchEvent(new FocusEvent("blur"));
      await settle();

      expect(doc.byId.query.raw).toBe(before);
      await vi.waitFor(() =>
        expect(root.querySelector(".query-print-refused")?.textContent)
          .toContain("the title would not survive the macro boundary"),
      );
    } finally {
      dispose();
    }
  });
});
