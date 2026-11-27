// GH #1004: "Collapse (one level)" / "Expand (one level)" on a selected query block reach the result groups that
// were not drawn yet. Results on many pages draw one group per page as it comes near the screen; the command only
// saw the drawn groups, so the late ones drew later with their default folds. Now the list keeps the press that
// committed its result level, and a writable result root with children takes it as its one fold snapshot (shared
// by every occurrence of the id) once it has a live row.
// The groups here are held off "screen" (observeNear) after the first few.
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetStore, selectBlock } from "../document";
import { undoTopTag } from "../document/history";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { ParsedQuery, Source } from "../editor/queryIr";
import { readEdnOptions } from "../editor/edn";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetNearObserverForTests } from "../lazyObserve";
import { paletteCommands, installKeybindings } from "../keybindings";
import { setGraphMeta } from "../graphSession";
import { setGraphConfigProblem } from "../graphConfig";
import { advanceResultsEpoch } from "../queryResultsEpoch";
import { bumpDataRev } from "../dataRevision";
import { readOnlyFold } from "./block/readOnlyFold";
import { editorIn } from "../tests/editorDriver";
import { editingId } from "../editorController";
import type { BlockDto, RefGroup } from "../types";

const saveFixture = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn(), persistTransfer: vi.fn(async () => true) }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saveFixture.markDirty
    : key === "persistTransfer" ? saveFixture.persistTransfer : saveFixture.actual?.[key as string],
  has: () => true,
}));

/** Query groups come near the screen only while `near(index)` says so; the rest wait in `held` until `scroll()`.
 *  Every other lazy body renders at once, as without layout. */
const gate = vi.hoisted(() => ({ index: 0, near: ((_index: number) => true) as (index: number) => boolean, held: [] as (() => void)[] }));
vi.mock("../lazyObserve", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lazyObserve")>();
  return {
    ...actual,
    observeNear: (el: Element, cb: (entry?: IntersectionObserverEntry) => void, reach?: "around" | "above") => {
      if (el.classList.contains("query-group") && !gate.near(gate.index++)) { gate.held.push(() => cb()); return; }
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
beforeEach(() => {
  gate.index = 0;
  gate.near = (index) => index < 4;
  gate.held = [];
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text) => {
    const trimmed = text.trim();
    return {
      query: { anchor: "block", filter: { kind: "raw", text: trimmed, diagnostic_kind: "not_applicable" }, diagnostics: [],
        source: { kind: "og", original: trimmed, og_options: "" } as Source },
      view: {}, legacy_table: readEdnOptions("")?.table ?? false,
    } as ParsedQuery;
  });
});
afterEach(() => {
  resetNearObserverForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  setGraphMeta(null);
  localStorage.clear();
  document.body.innerHTML = "";
});

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 4; i++) await tick(); };
const run = (id: string) => paletteCommands().find((c) => c.id === id)!.run();
const scroll = async () => { const held = gate.held.splice(0); for (const draw of held) draw(); await settle(); };
const pageOf = (name: string, roots: string[], readOnly = false): FeedPage =>
  ({ name, kind: "page", title: name, preBlock: null, roots, format: "md", readOnly, guide: false });
const node = (id: string, raw: string, page: string, parent: string | null, children: string[] = []): StoreNode =>
  ({ id, raw, collapsed: false, parent, page, children });

interface Fixture { pages: number; /** hits without a child */ leaves?: number[]; readOnly?: number[] }
/** `pages` pages P0…, each with one hit `h<i>` ("TODO") that has one child `c<i>`; the query block is on "Tasks". */
function load(fixture: Fixture, queryRaw = "{{query (task TODO)}}") {
  const byId: Record<string, StoreNode> = { query: node("query", queryRaw, "Tasks", null) };
  const pages: FeedPage[] = [pageOf("Tasks", ["query"])];
  const groups: RefGroup[] = [];
  for (let i = 0; i < fixture.pages; i++) {
    const name = `P${i}`;
    const leaf = fixture.leaves?.includes(i);
    byId[`h${i}`] = node(`h${i}`, `TODO hit ${i}`, name, null, leaf ? [] : [`c${i}`]);
    if (!leaf) byId[`c${i}`] = node(`c${i}`, `child ${i}`, name, `h${i}`);
    pages.push(pageOf(name, [`h${i}`], fixture.readOnly?.includes(i)));
    const dto: BlockDto = { id: `h${i}`, raw: `TODO hit ${i}`, marker: "TODO", collapsed: false,
      children: leaf ? [] : [{ id: `c${i}`, raw: `child ${i}`, collapsed: false, children: [] }] };
    groups.push({ page: name, kind: "page", blocks: [dto] });
  }
  setDoc({ byId, pages, feed: ["Tasks"], loaded: true });
  vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));
  return groups;
}
function mount(surface = "main") {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(() => <SurfaceContext.Provider value={surface}><Block id="query" /></SurfaceContext.Provider>, root) };
}
const childShown = (root: HTMLElement, i: number) => root.querySelector(`.query-group [data-block-id="c${i}"]`) !== null;
const hitShown = (root: HTMLElement, i: number) => root.querySelector(`.query-group [data-block-id="h${i}"]`) !== null;
const shownChildren = (root: HTMLElement, n: number) => Array.from({ length: n }, (_, i) => i).filter((i) => childShown(root, i));
const fileBytes = () => JSON.stringify(Object.values(doc.byId).map((n) => [n.id, n.raw, n.collapsed]));

describe("one-level fold commands reach query result groups not drawn yet (GH #1004)", () => {
  it("Collapse folds every result, drawn or not; the query stays open and no file or Undo changes", async () => {
    load({ pages: 30 });
    const { root, dispose } = mount();
    try {
      await settle();
      expect(shownChildren(root, 30)).toEqual([0, 1, 2, 3]); // the drawn groups, hits open (default level 2)
      const bytes = fileBytes(), top = undoTopTag();
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      await scroll();
      expect(Array.from({ length: 30 }, (_, i) => hitShown(root, i)).every(Boolean)).toBe(true);
      expect(shownChildren(root, 30)).toEqual([]);
      expect(root.querySelector(".query-group")).not.toBeNull(); // Q open
      expect(fileBytes()).toBe(bytes);
      expect(undoTopTag()).toBe(top);
    } finally { dispose(); }
  });

  it("Expand opens every result, drawn or not, when hits fold by default (level 1)", async () => {
    setGraphMeta({ root: "/graphs/A", ref_default_open_blocks_level: 1 } as never);
    load({ pages: 30 });
    const { root, dispose } = mount();
    try {
      await settle();
      expect(shownChildren(root, 30)).toEqual([]);
      selectBlock("query");
      run("editor/expand-children");
      await settle();
      await scroll();
      expect(shownChildren(root, 30)).toEqual(Array.from({ length: 30 }, (_, i) => i));
    } finally { dispose(); }
  });

  it("the last press wins: Collapse then Expand before the late groups draw leaves them open", async () => {
    load({ pages: 12 });
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      run("editor/expand-children");
      await settle();
      await scroll();
      expect(shownChildren(root, 12)).toEqual(Array.from({ length: 12 }, (_, i) => i));
    } finally { dispose(); }
  });

  it.each([["live in the other pane", true], ["cold in both panes", false]] as const)("reaches every open list of the query: late result %s", async (_label, liveInB) => {
    load({ pages: 12 });
    // Mount A holds groups 4.. ; mount B (indices 12..) holds them too unless `liveInB`.
    gate.near = (index) => index < 12 ? index < 4 : liveInB || index - 12 < 4;
    const a = mount("main"), b = mount("sidebar");
    try {
      await settle();
      expect(childShown(a.root, 8)).toBe(false);
      expect(hitShown(b.root, 8)).toBe(liveInB);
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      await scroll();
      expect(hitShown(a.root, 8) && hitShown(b.root, 8)).toBe(true);
      expect(childShown(a.root, 8)).toBe(false);
      expect(childShown(b.root, 8)).toBe(false);
    } finally { a.dispose(); b.dispose(); }
  });

  it("a late hit without children keeps the default: a child added after it draws shows", async () => {
    load({ pages: 8, leaves: [6] });
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      await scroll();
      expect(hitShown(root, 6)).toBe(true);
      setDoc("byId", "c6", node("c6", "child 6", "P6", "h6"));
      setDoc("byId", "h6", "children", ["c6"]);
      await settle();
      expect(childShown(root, 6)).toBe(true);
    } finally { dispose(); }
  });

  it("a committing Set depth forgets the press: late results take their default", async () => {
    load({ pages: 8 });
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      run("editor/set-depth-2");
      await settle();
      await scroll();
      expect(childShown(root, 6)).toBe(true);
    } finally { dispose(); }
  });

  it("the press dies with the list: Collapse, Collapse again (Q folds), reopen Q gives the defaults", async () => {
    load({ pages: 8 });
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      run("editor/collapse-children");
      await settle();
      expect(root.querySelector(".query-group")).toBeNull();
      gate.index = 0;
      run("editor/expand-children");
      await settle();
      await scroll();
      expect(shownChildren(root, 8)).toEqual(Array.from({ length: 8 }, (_, i) => i));
    } finally { dispose(); }
  });

  /** Pages from `spec`: name → [roots, readOnly?]; nodes from `nodes` (id → [page, parent, children]). Groups list
   *  each page's `hits` as DTOs (with their subtrees). */
  function loadTree(nodes: Record<string, [page: string, parent: string | null, children: string[]]>,
    groupHits: [page: string, hits: string[]][], readOnly: string[] = [], extraQueryChildren: string[] = []) {
    const byId: Record<string, StoreNode> = { query: node("query", "{{query (task TODO)}}", "Tasks", null, extraQueryChildren) };
    for (const [id, [pg, parent, children]] of Object.entries(nodes)) byId[id] = node(id, `TODO ${id}`, pg, parent, children);
    const names = [...new Set(Object.values(nodes).map(([pg]) => pg))];
    const pages = [pageOf("Tasks", ["query"], readOnly.includes("Tasks")),
      ...names.filter((n) => n !== "Tasks").map((n) => pageOf(n, Object.entries(nodes).filter(([, [pg, parent]]) => pg === n && parent === null).map(([id]) => id), readOnly.includes(n)))];
    const dto = (id: string): BlockDto => ({ id, raw: `TODO ${id}`, marker: "TODO", collapsed: false, children: byId[id].children.map(dto) });
    const groups: RefGroup[] = groupHits.map(([pg, hits]) => ({ page: pg, kind: "page", blocks: hits.map(dto) }));
    setDoc({ byId, pages, feed: ["Tasks"], loaded: true });
    vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));
  }
  const rowOf = (root: HTMLElement, id: string) => root.querySelector<HTMLElement>(`.query-group [data-block-id="${id}"]`)!;
  const inRow = (row: HTMLElement, id: string) => row.querySelector(`[data-block-id="${id}"]`) !== null;

  /** Check finding 1's tree: A has foldable descendants B (also a hit) and C. `drawn` draws the group before the
   *  press, which is develop's answer for the late group. */
  const borderTree = async (drawn: boolean) => {
    setGraphMeta({ root: "/graphs/A", ref_default_open_blocks_level: 10 } as never);
    loadTree({
      L: ["P0", null, ["l1"]], l1: ["P0", "L", []],
      A: ["PX", null, ["B", "C"]], B: ["PX", "A", ["b1"]], b1: ["PX", "B", []], C: ["PX", "A", ["c1"]], c1: ["PX", "C", []],
    }, [["P0", ["L"]], ["PX", ["A", "B"]]]);
    gate.near = (index) => drawn || index < 1;
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      await scroll();
      const before = inRow(rowOf(root, "A"), "B");
      rowOf(root, "A").querySelector<HTMLElement>(".collapse-toggle")!.click(); // reopen A by its arrow
      await settle();
      const border = rowOf(root, "A").querySelector<HTMLButtonElement>(".block-children-left-border")!;
      const choice = border.getAttribute("aria-expanded");
      border.click();
      await settle();
      const a = rowOf(root, "A");
      return { before, choice, after: ["B", "b1", "C", "c1"].map((id) => inRow(a, id)), c: doc.byId.C.collapsed, b: doc.byId.B.collapsed };
    } finally { dispose(); }
  };
  it("the descendant border over a hit that is also another hit's descendant chooses as when the group was drawn at the press (check finding 1)", async () => {
    const late = await borderTree(false);
    resetStore(); resetSharedQueryResultsForTests(); resetNearObserverForTests(); document.body.innerHTML = "";
    gate.index = 0; gate.held = [];
    const drawn = await borderTree(true);
    expect(late.before).toBe(false); // A drew folded, with the press
    expect(late).toEqual(drawn);
  });

  /** The review's tree: a late group with hits A and its child B; B has a child b1. */
  const nestedHit = async (level: number, drawn = false) => {
    setGraphMeta({ root: "/graphs/A", ref_default_open_blocks_level: level } as never);
    loadTree({
      L: ["P0", null, ["l1"]], l1: ["P0", "L", []],
      A: ["PX", null, ["B", "C"]], B: ["PX", "A", ["b1"]], b1: ["PX", "B", []], C: ["PX", "A", []],
    }, [["P0", ["L"]], ["PX", ["A", "B"]]]);
    gate.near = (index) => drawn || index < 1;
    return mount();
  };
  /** Put the caret in the occurrence of B nested under A, so the one-level commands act on it. */
  const editNested = async (root: HTMLElement) => {
    const nested = rowOf(root, "A").querySelector<HTMLElement>('.ls-block[data-block-id="B"]')!;
    nested.querySelector<HTMLElement>(".block-content-wrapper")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 0, clientY: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, clientX: 0, clientY: 0 }));
    await settle();
    return editorIn(rowOf(root, "A"), "B");
  };
  /** Whether each drawn occurrence of B shows b1, in document order. */
  const bOccurrences = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>('.query-group .ls-block[data-block-id="B"]')]
    .map((row) => row.querySelector('[data-block-id="b1"]') !== null);

  it("every occurrence of a late hit shares one fold: Collapse, then reopen its parent hit", async () => {
    const { root, dispose } = await nestedHit(10);
    try {
      await settle();
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      await scroll();
      rowOf(root, "A").querySelector<HTMLElement>(".collapse-toggle")!.click();
      await settle();
      expect(bOccurrences(root)).toEqual([false, false]); // nested under A, and the standalone hit
      // Expand (one level) on the nested B opens both; Collapse folds both.
      await editNested(root);
      run("editor/expand-children");
      await settle();
      expect(bOccurrences(root)).toEqual([true, true]);
      run("editor/collapse-children");
      await settle();
      expect(bOccurrences(root)).toEqual([false, false]);
    } finally { dispose(); }
  });

  it("every occurrence of a late hit shares one fold: Expand with hits folded by default (level 1)", async () => {
    const { root, dispose } = await nestedHit(1);
    try {
      await settle();
      selectBlock("query");
      run("editor/expand-children");
      await settle();
      await scroll();
      expect(bOccurrences(root)).toEqual([true, true]);
      await editNested(root);
      run("editor/collapse-children");
      await settle();
      expect(bOccurrences(root)).toEqual([false, false]);
    } finally { dispose(); }
  });

  it.each([["Collapse", "editor/collapse-children", 10], ["Expand", "editor/expand-children", 1]] as const)("after %s, Down from the nested late hit goes to the row drawn next", async (_label, command, level) => {
    const { root, dispose } = await nestedHit(level);
    try {
      await settle();
      selectBlock("query");
      run(command);
      await settle();
      await scroll();
      if (!inRow(rowOf(root, "A"), "B")) { rowOf(root, "A").querySelector<HTMLElement>(".collapse-toggle")!.click(); await settle(); }
      const nested = rowOf(root, "A").querySelector<HTMLElement>('.ls-block[data-block-id="B"]')!;
      const rows = [...root.querySelectorAll<HTMLElement>(".query-group .ls-block[data-block-id]")];
      const expected = rows[rows.indexOf(nested) + 1].dataset.blockId;
      const editor = await editNested(root);
      editor.select(editor.value().length, editor.value().length);
      editor.keydown({ key: "ArrowDown", bubbles: true, cancelable: true });
      await settle();
      expect(editingId()).toBe(expected);
    } finally { dispose(); }
  });

  it("a layout read while the late group still stands in leaves no snapshot: its first live draw folds (check finding 2)", async () => {
    const nodes: Record<string, [string, string | null, string[]]> = { L: ["P0", null, ["l1"]], l1: ["P0", "L", []] };
    const hits: string[] = [];
    for (let i = 0; i < 50; i++) { nodes[`w${i}`] = ["PW", null, [`k${i}`]]; nodes[`k${i}`] = ["PW", `w${i}`, []]; hits.push(`w${i}`); }
    loadTree(nodes, [["P0", ["L"]], ["PW", hits]]);
    gate.near = (index) => index < 1;
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      advanceResultsEpoch(); // the old answer's tickets go stale: the late group draws its stand-ins first
      await scroll();
      expect(rowOf(root, "L")).not.toBeNull();
      bumpDataRev(); // a fresh answer for the same page and ids
      await settle();
      await vi.waitFor(() => expect(root.querySelector('.query-group .ls-block[data-block-id="w0"]')).not.toBeNull());
      expect(Array.from({ length: 50 }, (_, i) => i).filter((i) => root.querySelector(`.query-group [data-block-id="k${i}"]`))).toEqual([]);
    } finally { dispose(); }
  });

  it.each([["writable", false], ["config unreadable", true]] as const)("a late read-only result never takes the press (check finding 3, %s)", async (_label, unreadable) => {
    // L (writable unless the config is unreadable) has a child k (folded by default) with a grandchild g; R is a
    // late hit on a read-only Org page.
    loadTree({
      L: ["P0", null, ["k"]], k: ["P0", "L", ["g"]], g: ["P0", "k", []],
      R: ["PR", null, ["r1"]], r1: ["PR", "R", []],
    }, [["P0", ["L"]], ["PR", ["R"]]], ["PR"]);
    if (unreadable) setGraphConfigProblem({ kind: "parse", message: "unreadable" } as never);
    gate.near = (index) => index < 1;
    const { root, dispose } = mount();
    try {
      await settle();
      const rSource = JSON.stringify(doc.byId.R), rSession = readOnlyFold.collapsed("R", false);
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      await scroll();
      rowOf(root, "L").querySelector<HTMLElement>(".collapse-toggle")!.click();
      await settle();
      run("editor/expand-children");
      await settle();
      // Writable L: Expand reaches k and shows g. Unreadable config: L's arrow addresses its session view, so the
      // raw fold stays and Expand stops at L (develop's outcome); g stays hidden.
      expect(root.querySelector('.query-group [data-block-id="g"]') !== null).toBe(!unreadable);
      expect(JSON.stringify(doc.byId.R)).toBe(rSource);
      expect(readOnlyFold.collapsed("R", false)).toBe(rSession);
    } finally { setGraphConfigProblem(null); dispose(); }
  });

  it("a refused mixed commit changes nothing: late results keep their default (host on a read-only page)", async () => {
    loadTree({
      K: ["Tasks", "query", ["k1"]], k1: ["Tasks", "K", []],
      L: ["P0", null, ["l1"]], l1: ["P0", "L", []], R: ["P1", null, ["r1"]], r1: ["P1", "R", []],
    }, [["P0", ["L"]], ["P1", ["R"]]], ["Tasks"], ["K"]);
    gate.near = (index) => index < 1;
    const { root, dispose } = mount();
    try {
      await settle();
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      await scroll();
      expect(root.querySelector('.query-group [data-block-id="l1"]')).not.toBeNull();
      expect(root.querySelector('.query-group [data-block-id="r1"]')).not.toBeNull();
    } finally { dispose(); }
  });

  it.each([["Collapse, then the menu's Expand all", true], ["the menu's Expand all, then Collapse", false]] as const)("%s with the late group cold: the late result opens (develop)", async (_label, pressFirst) => {
    // The late hit's source is folded, so the menu offers Expand all; it saves the source open.
    const groups = load({ pages: 8 });
    setDoc("byId", "h6", "collapsed", true);
    groups[6].blocks[0].collapsed = true;
    const { root, dispose } = mount();
    try {
      await settle();
      const menu = async (label: string) => {
        const details = root.querySelector<HTMLDetailsElement>(".qc-actions")!;
        details.open = true; details.dispatchEvent(new Event("toggle")); await tick();
        const id = details.querySelector("summary")!.getAttribute("aria-controls")!;
        [...document.getElementById(id)!.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label)!.click();
        await settle();
      };
      const press = async () => { selectBlock("query"); run("editor/collapse-children"); await settle(); };
      if (pressFirst) { await press(); await menu("Expand all"); } else { await menu("Expand all"); await press(); }
      await scroll();
      expect(hitShown(root, 6)).toBe(true);
      expect(childShown(root, 6)).toBe(true);
    } finally { dispose(); }
  });
});
