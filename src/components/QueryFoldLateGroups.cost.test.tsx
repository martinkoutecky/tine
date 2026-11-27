// GH #1004 cost counters (design-1004-smallest §5 row 6, its check's follow-up 5). 8 result groups are drawn and
// 1,992 are not. A one-level press reads no DTO of an undrawn group, asks each drawn group for its roots once per
// level it walks, and reruns no drawn row's fold; a late root's first draw takes one snapshot and reads its fold as
// often as a root drawn with no press does.
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetStore, selectBlock } from "../document";
import { setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { ParsedQuery, Source } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetNearObserverForTests } from "../lazyObserve";
import { paletteCommands, installKeybindings } from "../keybindings";
import { __livRefGroupInternals } from "./LiveRefGroup";
import type { BlockDto, RefGroup } from "../types";

const saveFixture = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn(), persistTransfer: vi.fn(async () => true) }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saveFixture.markDirty
    : key === "persistTransfer" ? saveFixture.persistTransfer : saveFixture.actual?.[key as string],
  has: () => true,
}));
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
/** `roots()` calls of the registered (drawn) groups. */
const counts = vi.hoisted(() => ({ roots: 0, folds: new Map<string, number>() }));
vi.mock("../collapseCommandSurface", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../collapseCommandSurface")>();
  return { ...actual, registerQueryResultFold: (group: Parameters<typeof actual.registerQueryResultFold>[0]) =>
    actual.registerQueryResultFold({ ...group, roots: () => { counts.roots += 1; return group.roots(); } }) };
});
/** Fold reads per row: every drawn row reads its stored fold once per run of its fold (macroFold.hostFolded). */
vi.mock("../document/edits/queryFoldOption", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../document/edits/queryFoldOption")>();
  return { ...actual, storedFold: (id: string) => { counts.folds.set(id, (counts.folds.get(id) ?? 0) + 1); return actual.storedFold(id); } };
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
  gate.held = [];
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text) => ({
    query: { anchor: "block", filter: { kind: "raw", text: text.trim(), diagnostic_kind: "not_applicable" }, diagnostics: [],
      source: { kind: "og", original: text.trim(), og_options: "" } as Source },
    view: {}, legacy_table: false,
  } as ParsedQuery));
});
afterEach(() => {
  resetNearObserverForTests();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.innerHTML = "";
});

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 4; i++) await tick(); };
const run = (id: string) => paletteCommands().find((c) => c.id === id)!.run();
const node = (id: string, page: string, parent: string | null, children: string[] = []): StoreNode =>
  ({ id, raw: `TODO ${id}`, collapsed: false, parent, page, children });
const pageOf = (name: string, roots: string[]): FeedPage =>
  ({ name, kind: "page", title: name, preBlock: null, roots, format: "md", readOnly: false, guide: false });

const LIVE = 8, TOTAL = 2000;
/** One hit with one child per page; hit 0 is a leaf, so it keeps no entry after a press. Undrawn DTOs count
 *  the reads of their id outside Macro's group keys. */
function load() {
  const byId: Record<string, StoreNode> = { query: node("query", "Tasks", null) };
  byId.query.raw = "{{query (task TODO)}}";
  const pages: FeedPage[] = [pageOf("Tasks", ["query"])];
  const groups: RefGroup[] = [];
  let dtoReads = 0;
  for (let i = 0; i < TOTAL; i++) {
    const leaf = i === 0;
    byId[`h${i}`] = node(`h${i}`, `P${i}`, null, leaf ? [] : [`c${i}`]);
    if (!leaf) byId[`c${i}`] = node(`c${i}`, `P${i}`, `h${i}`);
    pages.push(pageOf(`P${i}`, [`h${i}`]));
    const dto: BlockDto = { id: `h${i}`, raw: `TODO h${i}`, marker: "TODO", collapsed: false,
      children: leaf ? [] : [{ id: `c${i}`, raw: `TODO c${i}`, collapsed: false, children: [] }] };
    groups.push({ page: `P${i}`, kind: "page", blocks: [dto] });
  }
  setDoc({ byId, pages, feed: ["Tasks"], loaded: true });
  vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));
  return {
    groups,
    /** From now on, count id reads of the DTOs of groups `from`.. */
    watch(from: number) {
      for (const group of groups.slice(from)) for (const dto of group.blocks) {
        const id = dto.id;
        Object.defineProperty(dto, "id", { get: () => {
          // Develop's Macro reruns its group-key memo at any press (one id read per DTO per list, follow-up):
          // count every other read.
          if (!new Error().stack?.includes("queryGroupKey")) dtoReads += 1;
          return id;
        }, configurable: true });
      }
    },
    dtoReads: () => dtoReads,
  };
}
function mount(surface: string) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return render(() => <SurfaceContext.Provider value={surface}><Block id="query" /></SurfaceContext.Provider>, root);
}

describe("GH #1004 press and first-draw costs", () => {
  it("reads no undrawn DTO, asks each drawn group for its roots once per walked level and reruns no drawn fold", async () => {
    const fixture = load();
    // Pane A draws groups 0..7; pane B draws the same 8 (their rows are claimed by A's groups, first wins).
    gate.index = 0;
    gate.near = (index) => index % TOTAL < LIVE;
    const a = mount("main"), b = mount("sidebar");
    try {
      await settle();
      const drawnGroups = 2 * LIVE;
      expect(gate.held).toHaveLength(2 * (TOTAL - LIVE));
      fixture.watch(LIVE);
      selectBlock("query");
      const press = (command: string, levels: number) => {
        counts.roots = 0;
        counts.folds.clear();
        const snapshots = __livRefGroupInternals.resultFoldSnapshots, reads = fixture.dtoReads();
        run(command);
        expect(fixture.dtoReads() - reads, command).toBe(0);
        expect(counts.roots, command).toBe(levels * drawnGroups);
        expect(__livRefGroupInternals.resultFoldSnapshots, command).toBe(snapshots);
      };
      press("editor/collapse-children", 1);
      await settle();
      // The drawn leaf h0 has no entry in either pane: nothing reran its fold at the press.
      expect(counts.folds.get("h0") ?? 0).toBe(0);
      press("editor/expand-children", 1);
      await settle();
      press("editor/set-depth-1", 1);
      await settle();
      press("editor/collapse-children", 1); // every drawn result is folded now: the fallback folds Q
      await settle();
    } finally { a(); b(); }
  });

  it("a late root's first draw takes one snapshot and reads its fold as often as a late root drawn with no press", async () => {
    load();
    gate.near = (index) => index < LIVE;
    const dispose = mount("main");
    try {
      await settle();
      const draw = async (n: number) => {
        counts.folds.clear();
        for (const show of gate.held.splice(0, n)) show();
        await settle();
      };
      await draw(20); // groups 8..27, no press yet
      const baseline = Array.from({ length: 20 }, (_, i) => counts.folds.get(`h${LIVE + i}`));
      expect(baseline.every((n) => n !== undefined && n > 0)).toBe(true);
      selectBlock("query");
      run("editor/collapse-children");
      await settle();
      const snapshots = __livRefGroupInternals.resultFoldSnapshots;
      await draw(20); // groups 28..47, drawn after the press
      expect(__livRefGroupInternals.resultFoldSnapshots - snapshots).toBe(20);
      expect(Array.from({ length: 20 }, (_, i) => counts.folds.get(`h${LIVE + 20 + i}`))).toEqual(baseline);
      expect(document.querySelector('[data-block-id="c30"]')).toBeNull(); // and they drew folded
    } finally { dispose(); }
  });

  // Design row 7 (press and first-draw medians within 1.05x of develop's) is a cross-revision measurement:
  // TINE_1004_TIMING=1 prints the medians to compare against the same run on develop.
  it.skipIf(!process.env.TINE_1004_TIMING)("prints press and first-draw medians", async () => {
    const press: number[] = [], firstDraw: number[] = [];
    for (let rep = 0; rep < 9; rep++) {
      load();
      gate.index = 0;
      gate.held = [];
      gate.near = (index) => index < LIVE;
      const dispose = mount("main");
      try {
        await settle();
        selectBlock("query");
        let start = performance.now();
        run("editor/collapse-children");
        press.push(performance.now() - start);
        await settle();
        start = performance.now();
        for (const show of gate.held.splice(0, 40)) show();
        await settle();
        firstDraw.push(performance.now() - start);
      } finally { dispose(); resetStore(); document.body.innerHTML = ""; }
    }
    const median = (values: number[]) => [...values].sort((x, y) => x - y)[values.length >> 1];
    console.log(`TIMING press=${median(press).toFixed(2)}ms firstDraw40=${median(firstDraw).toFixed(2)}ms`);
  }, 120_000);
});
