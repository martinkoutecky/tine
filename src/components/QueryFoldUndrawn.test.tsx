// GH #1004: "Collapse (one level)" on a selected query block names every result of the
// query's answer, also the result groups not drawn yet (a query draws one group per page as
// it comes near the screen). Their folds are kept with the group as the rows' own arrows
// keep them: no file write, no undo step. Expand names them the same way.
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetStore, selectBlock, undoTopTag } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { ParsedQuery, Source } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetNearObserverForTests } from "../lazyObserve";
import { paletteCommands, installKeybindings } from "../keybindings";
import { endEdit, startEditing } from "../editorController";
import type { BlockDto, PageRead, RefGroup } from "../types";
import { setGraphConfigProblem } from "../graphConfig";

const saveFixture = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn(), persistTransfer: vi.fn(async () => true) }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saveFixture.markDirty
    : key === "persistTransfer" ? saveFixture.persistTransfer : saveFixture.actual?.[key as string],
  has: () => true,
}));
// The first DRAWN query groups come near at once; the rest wait until `release()`, as
// groups below the screen do in the app.
const near = vi.hoisted(() => ({ drawn: 4, seen: 0, held: [] as (() => void)[] }));
vi.mock("../lazyObserve", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lazyObserve")>();
  return {
    ...actual,
    observeNear: (el: Element, cb: (entry?: IntersectionObserverEntry) => void, reach?: Parameters<typeof actual.observeNear>[2]) => {
      if (el.classList.contains("query-group") && near.seen++ >= near.drawn) { near.held.push(() => cb()); return; }
      actual.observeNear(el, cb, reach);
    },
  };
});

let disposeKeys: () => void;
beforeAll(async () => {
  saveFixture.actual = await vi.importActual("../document/host/wiring");
  await initParser();
  disposeKeys = installKeybindings();
});
afterAll(() => disposeKeys());
afterEach(() => {
  setGraphConfigProblem(null);
  near.drawn = 4;
  near.seen = 0;
  near.held = [];
  resetNearObserverForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});
beforeEach(() => {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text) => {
    const original = text.trim();
    return {
      query: { anchor: "block", filter: { kind: "raw", text: original, diagnostic_kind: "not_applicable" }, diagnostics: [],
        source: { kind: "og", original, og_options: "" } as Source },
      view: {}, legacy_table: false,
    } as ParsedQuery;
  });
});

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 4; i++) await tick(); };
const run = (id: string) => paletteCommands().find((c) => c.id === id)!.run();
let PAGES = 30;
const pageName = (i: number) => `Ticket page ${i}`;
const node = (id: string, raw: string, parent: string | null, page: string, children: string[] = []): StoreNode =>
  ({ id, raw, collapsed: false, parent, page, children });
const feedPage = (name: string, roots: string[], readOnly = false): FeedPage =>
  ({ name, kind: "page", title: name, preBlock: null, roots, format: "md", readOnly, guide: false });

interface LoadOptions {
  /** Hit pages not in the working set: the backend serves them when their group is drawn. */
  unloaded?: number[];
  /** Served pages are Org. */
  org?: boolean;
  /** `kid-1` is folded in its file. */
  kidFolded?: boolean;
  /** Extra hits on pages never drawn, whose DTO `id` reads are counted in `idReads`. */
  counted?: { pages: number; hits: number };
}
const idReads = { n: 0 };

/** The query `q` on page Tasks (with an ordinary sibling `note` that has a child), and
 *  `pages` pages that each hold one hit `hit-i` with a child `kid-i`; `kid-1` has a child
 *  `grand-1`. `readOnlyPage` marks one hit page read-only. */
function load(readOnlyPage?: number, pages = 30, options: LoadOptions = {}) {
  PAGES = pages;
  const unloaded = new Set(options.unloaded ?? []);
  const byId: Record<string, StoreNode> = {
    q: node("q", "{{query (and [[Ticket]])}}", null, "Tasks"),
    note: node("note", "Note", null, "Tasks", ["note-kid"]),
    "note-kid": node("note-kid", "Note kid", "note", "Tasks"),
  };
  const feed = [feedPage("Tasks", ["q", "note"])];
  const groups: RefGroup[] = [];
  const served = new Map<string, PageRead>();
  for (let i = 1; i <= PAGES; i++) {
    if (unloaded.has(i)) {
      const format = options.org ? "org" : "md";
      served.set(pageName(i), { id: `pages/${pageName(i)}.${format}`, name: pageName(i), kind: "page", title: pageName(i), pre_block: null,
        format, read_only: i === readOnlyPage, guide: false, blocks: [{ id: `hit-${i}`, raw: `Ticket ${i} [[Ticket]]`, collapsed: false,
          children: [{ id: `kid-${i}`, raw: `Detail ${i}`, collapsed: false, children: [] }] }] });
    } else {
      byId[`hit-${i}`] = node(`hit-${i}`, `Ticket ${i} [[Ticket]]`, null, pageName(i), [`kid-${i}`]);
      byId[`kid-${i}`] = { ...node(`kid-${i}`, `Detail ${i}`, `hit-${i}`, pageName(i), i === 1 ? ["grand-1"] : []), collapsed: i === 1 && !!options.kidFolded };
      feed.push(feedPage(pageName(i), [`hit-${i}`], i === readOnlyPage));
    }
    groups.push({ page: pageName(i), kind: "page", ...(unloaded.has(i) && options.org ? { format: "org" as const } : {}), blocks: [{
      id: `hit-${i}`, raw: `Ticket ${i} [[Ticket]]`, collapsed: false,
      children: [{ id: `kid-${i}`, raw: `Detail ${i}`, collapsed: false, children: [] }],
    }] });
  }
  for (let p = 0; p < (options.counted?.pages ?? 0); p++) {
    const blocks = Array.from({ length: options.counted!.hits / options.counted!.pages }, (_, k) => {
      const dto = { raw: `Counted ${p}.${k} [[Ticket]]`, collapsed: false, children: [] } as unknown as BlockDto;
      const id = `counted-${p}-${k}`;
      Object.defineProperty(dto, "id", { enumerable: true, get: () => { idReads.n += 1; return id; } });
      return dto;
    });
    groups.push({ page: `Counted page ${p}`, kind: "page", blocks });
  }
  byId["grand-1"] = node("grand-1", "Grand 1", "kid-1", pageName(1));
  setDoc({ byId, pages: feed, feed: ["Tasks"], loaded: true });
  vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => served.get(name) ?? null);
}
function mount() {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <><Block id="q" /><Block id="note" /></>, root);
  return { root, dispose };
}
const shown = (root: HTMLElement, id: string) => root.querySelector(`.query-group [data-block-id="${id}"]`) !== null;
const raws = () => Object.fromEntries(Object.entries(doc.byId).map(([id, n]) => [id, [n.raw, n.collapsed]]));
const release = async () => {
  for (const cb of near.held.splice(0)) cb();
  await settle();
};
const all = (f: (i: number) => boolean) => Array.from({ length: PAGES }, (_, k) => k + 1).filter((i) => !f(i));

describe("fold commands over query groups not drawn yet (GH #1004)", () => {
  it("Collapse folds the results of groups that are drawn later", async () => {
    load();
    const { root, dispose } = mount();
    try {
      await settle();
      expect(shown(root, "kid-1")).toBe(true);
      expect(shown(root, "hit-30")).toBe(false); // not drawn yet
      const before = raws();
      const undoBefore = undoTopTag();
      selectBlock("q");
      run("editor/collapse-children");
      await settle();
      await release();
      expect(all((i) => shown(root, `hit-${i}`))).toEqual([]); // every group is drawn now
      expect(all((i) => !shown(root, `kid-${i}`))).toEqual([]); // and no hit shows its child
      expect(root.querySelector(".query-group")).not.toBeNull(); // the query itself stays open
      expect(raws()).toEqual(before);
      expect(undoTopTag()).toBe(undoBefore);
    } finally { dispose(); }
  });

  it("Expand after Collapse opens every group again, drawn or not", async () => {
    load();
    const { root, dispose } = mount();
    try {
      await settle();
      const before = raws();
      selectBlock("q");
      run("editor/collapse-children");
      await settle();
      run("editor/expand-children");
      await settle();
      await release();
      expect(all((i) => shown(root, `kid-${i}`))).toEqual([]);
      expect(raws()).toEqual(before);
    } finally { dispose(); }
  });

  it("folds the writable hits and leaves a read-only hit open", async () => {
    load(30);
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("q");
      run("editor/collapse-children");
      await settle();
      await release();
      expect(shown(root, "kid-29")).toBe(false);
      expect(shown(root, "kid-30")).toBe(true); // read-only rows fold only in the session's view map
    } finally { dispose(); }
  });

  it("a read-only hit not drawn at Collapse does not stop a later Expand at the results", async () => {
    near.drawn = 1;
    load(2, 2);
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("q");
      run("editor/collapse-children");
      await settle();
      await release();
      expect(shown(root, "kid-1")).toBe(false);
      // Open the writable hit with its own arrow: every result is open now.
      (root.querySelector('.query-group [data-block-id="hit-1"] .collapse-toggle') as HTMLElement).click();
      await settle();
      expect(shown(root, "kid-1")).toBe(true);
      expect(shown(root, "grand-1")).toBe(false); // below the hit, folded by default
      selectBlock("q");
      run("editor/expand-children");
      await settle();
      expect(shown(root, "grand-1")).toBe(true);
    } finally { dispose(); near.drawn = 4; }
  });

  it("Set depth on the query keeps naming only the drawn groups", async () => {
    load();
    const { root, dispose } = mount();
    try {
      await settle();
      const before = raws();
      selectBlock("q");
      run("editor/set-depth-1");
      await settle();
      expect(shown(root, "kid-1")).toBe(false);
      await release();
      expect(shown(root, "kid-30")).toBe(true);
      expect(raws()).toEqual(before);
    } finally { dispose(); }
  });

  it("with the caret in an ordinary block beside the query, the press folds that block's children only", async () => {
    load();
    const { root, dispose } = mount();
    try {
      await settle();
      startEditing("note", 0);
      await settle();
      run("editor/collapse-children");
      await settle();
      endEdit("blur");
      await release();
      expect(doc.byId["note"].collapsed).toBe(true);
      expect(all((i) => shown(root, `kid-${i}`))).toEqual([]);
    } finally { dispose(); }
  });
  // A result whose page loads read-only only after the press folds in the session's view map,
  // as its arrow does; the press's entry for it must not make it count as folded.
  it.each([
    ["an Org page that loads read-only", false],
    ["a graph whose config makes pages read-only", true],
  ])("a result on %s does not stop a later Expand at the results", async (_name, configReadOnly) => {
    near.drawn = 1;
    if (configReadOnly) setGraphConfigProblem({ kind: "config-read", message: "unreadable" });
    load(configReadOnly ? undefined : 2, 2, { unloaded: [2], org: !configReadOnly, kidFolded: true });
    const { root, dispose } = mount();
    try {
      await settle();
      expect(shown(root, "hit-2")).toBe(false);
      selectBlock("q");
      run("editor/collapse-children");
      await settle();
      expect(shown(root, "kid-1")).toBe(false);
      await release();
      await vi.waitFor(() => expect(doc.byId["hit-2"]).toBeDefined());
      await settle();
      expect(shown(root, "kid-2")).toBe(true); // read-only: shown open, whatever the press wrote
      (root.querySelector('.query-group [data-block-id="hit-1"] .collapse-toggle') as HTMLElement).click();
      await settle();
      expect(shown(root, "kid-1")).toBe(true);
      expect(shown(root, "grand-1")).toBe(false);
      selectBlock("q");
      run("editor/expand-children");
      await settle();
      expect(shown(root, "grand-1")).toBe(true);
    } finally { dispose(); }
  });

  // Cost: the answer's ids of groups not drawn are read only by a press that writes them.
  it("Set depth reads no id of a result not drawn; Collapse reads each once", async () => {
    near.drawn = 8;
    load(undefined, 8, { counted: { pages: 4, hits: 1992 } });
    const { root, dispose } = mount();
    try {
      await settle();
      expect(shown(root, "kid-8")).toBe(true);
      selectBlock("q");
      idReads.n = 0;
      run("editor/set-depth-1");
      expect(idReads.n).toBe(0);
      await settle();
      expect(shown(root, "kid-1")).toBe(false);
      run("editor/expand-children");
      await settle();
      expect(shown(root, "kid-1")).toBe(true);
      idReads.n = 0;
      run("editor/collapse-children");
      expect(idReads.n).toBe(1992);
      await settle();
      expect(shown(root, "kid-8")).toBe(false);
    } finally { dispose(); }
  });
});
