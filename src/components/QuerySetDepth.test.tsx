// GH #794: Set depth N on the surfaces whose rows do not fold through the file: a
// query block's result rows (local to the result view, like their arrows) and a
// block embed's root (the embedding block's own `collapsed::`, GH #360).
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetStore, selectBlock, undo } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { ParsedQuery, Source } from "../editor/queryIr";
import { readEdnOptions } from "../editor/edn";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetNearObserverForTests } from "../lazyObserve";
import { paletteCommands, installKeybindings } from "../keybindings";
import type { RefGroup } from "../types";

const saveFixture = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn(), persistTransfer: vi.fn(async () => true) }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saveFixture.markDirty
    : key === "persistTransfer" ? saveFixture.persistTransfer : saveFixture.actual?.[key as string],
  has: () => true,
}));

let disposeKeys: () => void;
beforeAll(async () => {
  saveFixture.actual = await vi.importActual("../document/host/wiring");
  await initParser();
  disposeKeys = installKeybindings();
});
afterAll(() => disposeKeys());
afterEach(() => {
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
const run = (id: string) => paletteCommands().find((c) => c.id === id)!.run();
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


describe("Set depth on a query block (GH #794)", () => {
  // Levels: query 0, todo (the hit) 1, kid 2, grand 3 (a leaf). Rows below the hit start folded.
  it("opens and folds the result rows by level, writing only the host's own fold", async () => {
    load("{{query (task TODO)}}");
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("query");
      run("editor/set-depth-3");
      await settle();
      expect(shown(root, "grand")).toBe(true);
      expect(folded(root, "kid")).toBe(false);
      run("editor/set-depth-2"); // query and hit open, kid folded
      await settle();
      expect(shown(root, "todo")).toBe(true);
      expect(folded(root, "kid")).toBe(true);
      expect(shown(root, "grand")).toBe(false);
      run("editor/set-depth-1"); // the hit itself folds
      await settle();
      expect(folded(root, "todo")).toBe(true);
      expect(shown(root, "kid")).toBe(false);
      expect(doc.byId.query.raw).not.toContain("collapsed::");
      expect(doc.byId.kid.collapsed).toBe(false); // result folds never reach the source blocks
      expect(doc.byId.todo.collapsed).toBe(false);
      run("editor/set-depth-0");
      await settle();
      expect(doc.byId.query.raw).toContain("collapsed:: true");
      expect(root.querySelector(".query-group")).toBeNull();
      undo();
      await settle();
      expect(root.querySelector(".query-group")).not.toBeNull();
    } finally { dispose(); }
  });

  it("opens a query block that only its authored :collapsed? true folds", async () => {
    load("{{query (task TODO) {:collapsed? true}}}");
    const { root, dispose } = mount();
    try {
      await settle();
      expect(root.querySelector(".query-group")).toBeNull();
      selectBlock("query");
      run("editor/set-depth-2");
      await settle();
      expect(root.querySelector(".query-group")).not.toBeNull();
      expect(doc.byId.query.raw).toContain("collapsed:: false");
      expect(folded(root, "kid")).toBe(true);
    } finally { dispose(); }
  });
});
