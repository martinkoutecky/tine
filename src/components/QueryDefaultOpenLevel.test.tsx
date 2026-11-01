// GH #793: `:ref/default-open-blocks-level` also governs query results (OG
// `block-default-collapsed?` tests `:ref?` OR `:custom-query?`). The default 2 is
// pinned by QueryExpandChildren.test.tsx; here the key moves it.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { ParsedQuery, Source } from "../editor/queryIr";
import { readEdnOptions } from "../editor/edn";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetNearObserverForTests } from "../lazyObserve";
import { setGraphMeta } from "../graphSession";
import type { RefGroup } from "../types";

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
  setGraphMeta(null);
  resetNearObserverForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});
beforeEach(() => {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text, _dialect, properties) => {
    const trimmed = text.trim();
    const split = /^([\s\S]*?[)\]"])\s*(\{:[\s\S]*\})$/.exec(trimmed);
    const [original, og_options] = split && !trimmed.startsWith("{") ? [split[1], split[2]] : [trimmed, ""];
    const view = Object.fromEntries((properties ?? []).filter(([key]) => key === "tine.view").map(([, value]) => ["view", value]));
    return {
      query: { anchor: "block", filter: { kind: "raw", text: original, diagnostic_kind: "not_applicable" }, diagnostics: [],
        source: { kind: "og", original, og_options } as Source },
      view, legacy_table: readEdnOptions(og_options)?.table ?? false,
    } as ParsedQuery;
  });
});

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const settle = async () => { await tick(); await tick(); await tick(); };
const node = (id: string, raw: string, parent: string | null, children: string[] = []): StoreNode => ({ id, raw, collapsed: false, parent, page: "Tasks", children });
const page = (roots: string[]): FeedPage => ({ name: "Tasks", kind: "page", title: "Tasks", preBlock: null, roots, format: "md", readOnly: false, guide: false });

/** A query host (`query`) and its one result `todo`, which has a child `kid` that has a child `grand`. In a
 *  query's results the rows below the hit start folded (OG). */
function load(queryRaw: string) {
  setDoc({
    byId: {
      query: node("query", queryRaw, null),
      src: node("src", "Source page block", null),
      todo: node("todo", "TODO Result row", "src", ["kid"]),
      kid: node("kid", "Kid row", "todo", ["grand"]),
      grand: node("grand", "Grand row", "kid"),
    },
    pages: [page(["query", "src"])], feed: ["Tasks"], loaded: true,
  });
  setDoc("byId", "src", "children", ["todo"]);
  const groups: RefGroup[] = [{ page: "Tasks", kind: "page", blocks: [{
    id: "todo", raw: "TODO Result row", marker: "TODO", collapsed: false,
    children: [{ id: "kid", raw: "Kid row", collapsed: false, children: [{ id: "grand", raw: "Grand row", collapsed: false, children: [] }] }],
  }] }];
  vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));
}
function mount() {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(() => <Block id="query" />, root) };
}
const shown = (root: HTMLElement, id: string) => root.querySelector(`.query-group [data-block-id="${id}"]`) !== null;
/** A result row drawn with its children hidden. */
const folded = (root: HTMLElement, id: string) => root.querySelector(`.query-group [data-block-id="${id}"]`)?.classList.contains("collapsed") ?? false;


describe("query results honor the default open level (GH #793)", () => {
  // The hit (todo) is level 1, kid level 2, grand level 3 (a leaf).
  it.each([
    [undefined, { todo: false, kid: true }, false],
    [2, { todo: false, kid: true }, false],
    [3, { todo: false, kid: false }, true],
    [10, { todo: false, kid: false }, true],
    [1, { todo: true }, false], // level 1 folds the hit itself
  ])("level %s", async (level, foldedRows, grandShown) => {
    setGraphMeta({ root: "/graphs/A", ref_default_open_blocks_level: level } as never);
    load("{{query (task TODO)}}");
    const { root, dispose } = mount();
    try {
      await settle();
      expect(shown(root, "todo")).toBe(true);
      for (const [id, isFolded] of Object.entries(foldedRows)) expect(folded(root, id), `${id} folded`).toBe(isFolded);
      expect(shown(root, "grand")).toBe(grandShown);
      expect(doc.byId.todo.collapsed || doc.byId.kid.collapsed).toBe(false); // the file is never told
    } finally { dispose(); }
  });
});
