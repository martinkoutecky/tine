import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetStore } from "../document";
import { bumpDataRev } from "../graphSession";
import { setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { ParsedQuery, Source } from "../editor/queryIr";
import { readEdnOptions } from "../editor/edn";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetNearObserverForTests } from "../lazyObserve";
import type { RefGroup } from "../types";
import * as scrollContainer from "../scrollContainer";

beforeAll(async () => { await initParser(); });
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
    children: [],
  }] }];
  vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(structuredClone(groups)));
}
function mount() {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(() => <Block id="query" />, root) };
}


const runs = () => (backend().queryRun as unknown as { mock: { calls: unknown[] } }).mock.calls.length;

// GH #944 (Jioqls): on the Journals page the agenda and the default queries sit above the block being ticked. When
// their rerun changes their height, everything below moves unless the reader is held. The held restore needs the
// geometry snapshot taken before the new rows commit.
describe("GH #944: a rerun that replaces an answer holds the reader", () => {
  it("snapshots the reader before the second answer commits, and not for the first", async () => {
    load("{{query (task TODO)}}");
    const real = scrollContainer.preserveAboveViewport;
    // The snapshot call carries an empty change; the row of the old answer is still in the DOM when it is taken.
    const snapshots: boolean[] = [];
    vi.spyOn(scrollContainer, "preserveAboveViewport").mockImplementation((el, change, ...rest) => {
      if (String(change).replace(/\s+/g, "") === "()=>{}") snapshots.push(document.querySelector('[data-block-id="todo"]') !== null);
      real(el, change, ...rest);
    });
    const { dispose } = mount();
    try {
      await settle();
      expect(runs()).toBeGreaterThan(0);
      expect(snapshots).toEqual([]);
      vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult([{ page: "Tasks", kind: "page", blocks: [] }]));
      bumpDataRev(true);
      await vi.waitFor(() => expect(snapshots).toEqual([true]), { timeout: 4000 });
      await settle();
      expect(snapshots).toEqual([true]);
    } finally { dispose(); }
  });
});
