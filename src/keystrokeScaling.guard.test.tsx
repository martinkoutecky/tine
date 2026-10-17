// Keystroke-scaling guard (sweep after GH #248). See keystrokeProbe.test.support.ts
// for what is measured. For each editing surface this mounts the surface on a
// small and a large page, performs single-character edits on one node, and
// compares the work done on OTHER nodes after the write.
//
// Modes: by default only the FAST surfaces run (about a minute); KEYSTROKE_SWEEP=1
// runs every surface on both write paths (about 40 minutes in jsdom, split it with
// KEYSTROKE_ONLY="substr|substr"); KEYSTROKE_SWEEP_OUT=<file> writes the JSON
// report; KEYSTROKE_SIZES=a,b overrides the page sizes.
//
// Verdicts are a ratchet: GROWS names the surfaces known (2026-10-17 sweep) to do
// work proportional to page/result size per keystroke. A surface that stops
// growing fails its "grows" assertion on purpose: fix landed, move it out of GROWS.
// A surface outside GROWS that starts growing is a new GH #248-shaped regression.
import "./keystrokeProbe.test.support";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import fs from "node:fs";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { resetStore } from "./document";
import { setDoc, doc } from "./document/model";
import { endEdit, startEditing } from "./editorController";
import { mainPaneRouter, resetTabsToJournals } from "./router";
import { resetNearObserverForTests } from "./lazyObserve";
import { installKeybindings } from "./keybindings";
import { PageView } from "./components/Page";
import { Block } from "./components/Block";
import { PaneTree } from "./App";
import { InPageFind } from "./components/InPageFind";
import { openInPageFind, closeInPageFind, setInPageFindQuery } from "./inpageFind";
import { layoutRoot, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import type { PaneSnapshot } from "./router";
import { RightSidebar } from "./components/RightSidebar";
import { TodayTaskSummary } from "./components/TodayTaskSummary";
import { loadSingle } from "./document/workingSet";
import { applySidebarSession, setRightSidebar } from "./ui";
import { journalTitle, appNow, currentDayKey } from "./journal";
import { setGraphMeta } from "./graphSession";
import { resetSharedQueryResultsForTests } from "./queryResultCache";
import { backendReadsQueries, blockRunResult } from "./tests/queryReadingsTestkit";
import { bindTestHost } from "./document/host/wiring.test.support";
import { measure, median, type Measurement } from "./keystrokeProbe.test.support";
import type { BlockDto, PageDto, RefGroup } from "./types";
import { flags } from "./keystrokeScaling.flags";

vi.mock("solid-js/store", async (importOriginal) => {
  const actual = await importOriginal<typeof import("solid-js/store")>();
  const probe = await import("./keystrokeProbe.test.support");
  return {
    ...actual,
    createStore: ((init: object, opts?: object) => {
      const [state, set] = (actual.createStore as (a: object, b?: object) => [object, unknown])(init, opts);
      return [probe.instrumentDocStore(state, init), set];
    }) as typeof actual.createStore,
  };
});
// A theme whose presentation selects the compact today-task summary is opt-in;
// turn it on only for the surfaces that name it.
vi.mock("./themeGallery", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./themeGallery")>();
  const f = await import("./keystrokeScaling.flags");
  return { ...actual, selectedThemePresentation: () => (f.flags.compactTheme ? { todayTaskSummary: "compact" } : {}) };
});

// ------------------------------------------------------------------ observers

/** Stub IntersectionObserver: see `observe`. */
class Observer {
  static all: Observer[] = [];
  targets = new Set<Element>();
  constructor(private cb: IntersectionObserverCallback) { Observer.all.push(this); }
  observe(t: Element) {
    this.targets.add(t);
    // Everything except the outline windows is "near the viewport": block bodies,
    // result/reference/embed groups and sticky sentinels mount as they would in
    // the viewport, while far outline windows stay shells (the windowing bound).
    if (!t.hasAttribute("data-outline-window")) {
      queueMicrotask(() => this.cb([{ target: t, isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver));
    }
  }
  unobserve(t: Element) { this.targets.delete(t); }
  disconnect() { this.targets.clear(); }
  takeRecords() { return []; }
}

// ------------------------------------------------------------------ fixtures

interface Blk extends BlockDto { has_id?: boolean }
const blk = (id: string, raw: string, children: Blk[] = [], collapsed = false): Blk => ({ id, has_id: false, raw, children, collapsed } as Blk);
function textFor(i: number): string {
  return i % 10 === 3 ? `TODO task ${i} [[Ref ${i % 7}]]`
    : i % 10 === 5 ? `Row ${i} with #tag${i % 5} and **bold**`
      : i % 25 === 7 ? `Row ${i} authored\nid:: 00000000-0000-4000-8000-${String(i).padStart(12, "0")}`
        : `Row ${i} unique content`;
}
export interface Fixture { blocks: Blk[]; midRoot: string; child: string; grand: string; great: string; count: number }
/** An outline of about `n` blocks. Each root: 9 children; child 0 has 2
 * grandchildren, the first of which has one great-grandchild (depth 3), so
 * zoom/breadcrumb/sticky ancestors exist. Ids: p-k, p-k.j, p-k.j.g, p-k.j.g.h. */
export function outline(n: number, prefix: string, fold: "none" | "all-but-mid" = "none"): Fixture {
  const roots: Blk[] = [];
  let made = 0, k = 0;
  const per = 13;
  const rootCount = Math.max(1, Math.round(n / per));
  const mid = Math.floor(rootCount / 2);
  while (k < rootCount) {
    const folded = fold === "all-but-mid" && k !== mid;
    const kids: Blk[] = [];
    for (let j = 0; j < 9; j++) {
      const id = `${prefix}-${k}.${j}`;
      const grands: Blk[] = [];
      if (j === 0) {
        for (let g = 0; g < 2; g++) {
          const gid = `${id}.${g}`;
          const greats = g === 0 ? [blk(`${gid}.0`, textFor(made++))] : [];
          grands.push(blk(gid, textFor(made++), greats));
        }
      }
      kids.push(blk(id, textFor(made++), grands, folded && grands.length > 0));
    }
    roots.push(blk(`${prefix}-${k}`, `Root ${k} text`, kids, folded));
    made++; k++;
  }
  const midRoot = `${prefix}-${mid}`;
  return { blocks: roots, midRoot, child: `${midRoot}.0`, grand: `${midRoot}.0.0`, great: `${midRoot}.0.0.0`, count: made };
}
function pageDto(name: string, blocks: Blk[], kind: "page" | "journal" = "page", pre: string | null = null): PageDto {
  return { id: `${kind === "journal" ? "journals" : "pages"}/${name}.md`, name, title: name, kind, pre_block: pre, blocks } as unknown as PageDto;
}

// ------------------------------------------------------------------ surfaces

interface Mounted { host: HTMLElement; dispose: () => void; editId: string; root?: HTMLElement }
interface Surface { name: string; enter?: "click"; sizes?: number[]; observer?: "none" | "refs"; edit?: "both" | "setDoc"; compactTheme?: boolean; mount: (n: number) => Promise<Mounted> }

function mountInto(view: () => import("solid-js").JSX.Element, className = "main-content"): { host: HTMLElement; dispose: () => void } {
  const paneEl = document.createElement("div"); paneEl.dataset.paneId = "main";
  const host = document.createElement("div"); host.className = className;
  paneEl.append(host); document.body.append(paneEl);
  const disposeRender = render(view, host);
  const disposeKeys = installKeybindings();
  return { host, dispose: () => { disposeKeys(); disposeRender(); } };
}
const titleShown = (host: HTMLElement, text: string) => vi.waitFor(() => expect(host.querySelector(".page-title")?.textContent).toContain(text));

/** Answer getPage / getPageByPath for these pages by name (everything else: absent). */
function serve(...pages: PageDto[]): void {
  const byName = new Map(pages.map(p => [p.name.toLowerCase(), p]));
  const byPath = new Map(pages.map(p => [(p as unknown as { id: string }).id, p]));
  vi.spyOn(backend(), "getPage").mockImplementation((async (name: string) => byName.get(name.toLowerCase()) ?? null) as never);
  vi.spyOn(backend(), "getPageByPath").mockImplementation((async (path: string) => byPath.get(path) ?? null) as never);
}

async function mountPage(name: string, dto: PageDto, ...others: PageDto[]): Promise<{ host: HTMLElement; dispose: () => void }> {
  serve(dto, ...others);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name, pageKind: "page" });
  const m = mountInto(() => <PageView />);
  await titleShown(m.host, name);
  return m;
}

/** A today journal of ~n blocks plus two small older days. */
async function mountJournals(n: number): Promise<{ host: HTMLElement; dispose: () => void; fx: Fixture }> {
  const today = journalTitle(appNow());
  const fx = outline(n, "td");
  const older = ["Sep 20th, 2026", "Sep 19th, 2026"].map((name, i) => pageDto(name, outline(30, `old${i}`).blocks, "journal"));
  const pages = [pageDto(today, fx.blocks, "journal"), ...older];
  serve(...pages);
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
    pages: pages as never, as_of_day: currentDayKey(), next_before_day: null, done: true,
  });
  resetTabsToJournals();
  const m = mountInto(() => <PageView />);
  await vi.waitFor(() => expect(m.host.querySelector(".page-section")).not.toBeNull());
  return { ...m, fx };
}

const surfaces: Surface[] = [
  {
    name: "routed page",
    mount: async (n) => {
      const fx = outline(n, "pg");
      const m = await mountPage("Window", pageDto("Window", fx.blocks));
      return { ...m, editId: fx.grand };
    },
  },
  {
    name: "routed page, folded subtrees",
    mount: async (n) => {
      const fx = outline(n, "pg", "all-but-mid");
      const m = await mountPage("Window", pageDto("Window", fx.blocks));
      return { ...m, editId: fx.grand };
    },
  },
  {
    name: "routed page with page properties",
    mount: async (n) => {
      const fx = outline(n, "pg");
      const m = await mountPage("Window", pageDto("Window", fx.blocks, "page", "tags:: alpha, beta\nalias:: Win\ntype:: sample"));
      return { ...m, editId: fx.grand };
    },
  },
  {
    name: "zoomed block (routed), edit the zoom root",
    mount: async (n) => {
      const fx = outline(n, "pg");
      const m = await mountPage("Window", pageDto("Window", fx.blocks));
      mainPaneRouter.focusBlock(fx.child);
      await vi.waitFor(() => expect(m.host.querySelector(".zoomed-page")).not.toBeNull());
      return { ...m, editId: fx.child };
    },
  },
  {
    name: "zoomed block (routed), edit a descendant",
    mount: async (n) => {
      const fx = outline(n, "pg");
      const m = await mountPage("Window", pageDto("Window", fx.blocks));
      mainPaneRouter.focusBlock(fx.child);
      await vi.waitFor(() => expect(m.host.querySelector(".zoomed-page")).not.toBeNull());
      return { ...m, editId: fx.great };
    },
  },
  {
    name: "journals feed (today)",
    mount: async (n) => {
      const m = await mountJournals(n);
      return { ...m, editId: m.fx.grand };
    },
  },
  {
    name: "journals feed (today) with compact task summary",
    compactTheme: true,
    mount: async (n) => {
      const m = await mountJournals(n);
      await vi.waitFor(() => expect(m.host.querySelector(".today-task-summary")).not.toBeNull());
      return { ...m, editId: m.fx.grand };
    },
  },
  {
    name: "zoomed block (journal)",
    mount: async (n) => {
      const m = await mountJournals(n);
      mainPaneRouter.focusBlock(m.fx.child);
      await vi.waitFor(() => expect(m.host.querySelector(".zoomed-page")).not.toBeNull());
      return { ...m, editId: m.fx.child };
    },
  },
  {
    name: "TodayTaskSummary component alone (page edited elsewhere)",
    mount: async (n) => {
      const fx = outline(n, "ts");
      const dto = pageDto("Tasks", fx.blocks, "journal");
      const bound = await bindTestHost(); void bound;
      loadSingle(dto as never);
      const page = doc.pages.find(p => p.name === "Tasks")!;
      const m = mountInto(() => <TodayTaskSummary page={page} />);
      return { ...m, editId: fx.grand };
    },
    edit: "setDoc",
  },
  {
    name: "right sidebar, page item",
    observer: "none",
    mount: async (n) => {
      const fx = outline(n, "sp");
      loadSingle(pageDto("Side", fx.blocks) as never);
      applySidebarSession({ right: true, items: [{ kind: "page", name: "Side", pageKind: "page" }] });
      vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
      vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
      vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
      const root = document.createElement("div"); document.body.append(root);
      const dispose = render(() => <RightSidebar />, root);
      await vi.waitFor(() => expect(root.querySelector(".rs-item-body")).not.toBeNull());
      return { host: root, dispose: () => { dispose(); setRightSidebar([]); applySidebarSession({ right: false, items: [] }); }, editId: fx.grand };
    },
  },
  {
    name: "right sidebar, block item",
    observer: "none",
    mount: async (n) => {
      const fx = outline(n, "sb");
      loadSingle(pageDto("Side", fx.blocks) as never);
      applySidebarSession({ right: true, items: [{ kind: "block", uuid: fx.child, page: "Side", pageKind: "page" }] });
      vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
      vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
      vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
      const root = document.createElement("div"); document.body.append(root);
      const dispose = render(() => <RightSidebar />, root);
      await vi.waitFor(() => expect(root.querySelector(".rs-item-body .ls-block")).not.toBeNull());
      return { host: root, dispose: () => { dispose(); setRightSidebar([]); applySidebarSession({ right: false, items: [] }); }, editId: fx.child };
    },
  },
];


/** Page shapes where a window group's subtree is not bounded by the 24-row group weight. */
function heavy(n: number, shape: "two-roots" | "folded-archive" | "flat"): { blocks: Blk[]; editId: string } {
  if (shape === "flat") {
    const blocks = Array.from({ length: n }, (_, i) => blk(`fl-${i}`, textFor(i)));
    return { blocks, editId: `fl-${Math.floor(n / 2)}` };
  }
  if (shape === "two-roots") {
    // Two top-level sections, each ~n/2 blocks: 20 sub-sections of leaves.
    const section = (name: string) => blk(`hv-${name}`, `Section ${name}`, Array.from({ length: 20 }, (_, g) =>
      blk(`hv-${name}.${g}`, `Group ${g}`, Array.from({ length: Math.max(1, Math.floor(n / 40) - 1) }, (_, i) => blk(`hv-${name}.${g}.${i}`, textFor(i))))));
    return { blocks: [section("A"), section("B")], editId: "hv-A.3.2" };
  }
  // folded-archive: a small open section plus folded archive sections holding the rest.
  const today = blk("fa-today", "Today", Array.from({ length: 10 }, (_, i) => blk(`fa-today.${i}`, textFor(i))));
  // 90 folded sections: enough top-level weight (> 80) for the outline to window.
  const archives = Array.from({ length: 90 }, (_, k) => blk(`fa-arch${k}`, `Archive ${k}`,
    Array.from({ length: Math.max(1, Math.floor(n / 90)) }, (_, i) => blk(`fa-arch${k}.${i}`, textFor(i))), true));
  return { blocks: [today, ...archives], editId: "fa-today.3" };
}
for (const shape of ["two-roots", "folded-archive", "flat"] as const) {
  surfaces.push({
    name: `routed page, shape ${shape}`,
    mount: async (n) => {
      const h = heavy(n, shape);
      const m = await mountPage("Window", pageDto("Window", h.blocks));
      return { ...m, editId: h.editId };
    },
  });
}

const refRow = (i: number, extra = "") => blk(`rr-${i}`, `TODO result ${i} [[Target]]${extra}`);
const refGroups = (n: number, perGroup = 10): RefGroup[] =>
  Array.from({ length: Math.ceil(n / perGroup) }, (_, g) => ({
    page: `Source ${g}`, kind: "page" as const,
    blocks: Array.from({ length: Math.min(perGroup, n - g * perGroup) }, (_, i) => refRow(g * perGroup + i)),
  }));

surfaces.push(
  {
    name: "block embed, edit a host block (host page large)",
    observer: "refs",
    mount: async (n) => {
      const fx = outline(n, "pg");
      const uuid = "00000000-0000-4000-8000-0000000000e1";
      const embedHost = blk("pg-embed", `{{embed ((${uuid}))}}`);
      const src = pageDto("Source", [blk("src-1", `Embedded source\nid:: ${uuid}`, [blk("src-1.0", "Embedded child")]), blk("src-2", "Other source")]);
      const host = pageDto("Window", [embedHost, ...fx.blocks]);
      serve(host, src);
      vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids: string[]) =>
        ids.map(id => id === uuid ? { page: "Source", kind: "page", blocks: [src.blocks[0]] } as RefGroup : null));
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Window", pageKind: "page" });
      const m = mountInto(() => <PageView />);
      await titleShown(m.host, "Window");
      mainPaneRouter.focusBlock(null);
      return { ...m, editId: fx.grand };
    },
  },
  {
    name: "block embed, edit inside the embed (host page large)",
    enter: "click",
    observer: "refs",
    mount: async (n) => {
      const fx = outline(n, "pg");
      const uuid = "00000000-0000-4000-8000-0000000000e1";
      const embedHost = blk("pg-embed", `{{embed ((${uuid}))}}`);
      const src = pageDto("Source", [blk("src-1", `Embedded source\nid:: ${uuid}`, [blk("src-1.0", "Embedded child")]), blk("src-2", "Other source")]);
      const host = pageDto("Window", [embedHost, ...fx.blocks]);
      serve(host, src);
      vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids: string[]) =>
        ids.map(id => id === uuid ? { page: "Source", kind: "page", blocks: [src.blocks[0]] } as RefGroup : null));
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Window", pageKind: "page" });
      const m = mountInto(() => <PageView />);
      await titleShown(m.host, "Window");
      await vi.waitFor(() => expect(doc.byId["src-1.0"]).toBeDefined(), { timeout: 3000 });
      return { ...m, editId: "src-1.0" };
    },
  },
  {
    name: "page embed of a large page (host small)",
    enter: "click",
    observer: "refs",
    sizes: [200, 1000],
    mount: async (n) => {
      const fx = outline(n, "big");
      const big = pageDto("Big", fx.blocks);
      const host = pageDto("Window", [blk("h-embed", "{{embed [[Big]]}}"), blk("h-0", "Intro")]);
      serve(host, big);
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Window", pageKind: "page" });
      const m = mountInto(() => <PageView />);
      await titleShown(m.host, "Window");
      await vi.waitFor(() => expect(doc.byId[fx.grand]).toBeDefined(), { timeout: 15000 });
      return { ...m, editId: fx.grand };
    },
  },
  {
    name: "simple query, results N (edit a result)",
    enter: "click",
    observer: "refs",
    sizes: [200, 1000],
    mount: async (n) => {
      const q = "(task TODO)";
      backendReadsQueries({ [q]: { form: q } });
      const host = pageDto("Window", [blk("h-q", `{{query ${q}}}`), blk("h-0", "Intro")]);
      const groups = refGroups(n);
      serve(host, ...groups.map(g => pageDto(g.page, g.blocks as Blk[])));
      vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Window", pageKind: "page" });
      const m = mountInto(() => <PageView />);
      await titleShown(m.host, "Window");
      const target = `rr-${Math.floor(n / 2)}`;
      await vi.waitFor(() => expect(doc.byId[target]).toBeDefined(), { timeout: 20000 });
      return { ...m, editId: target };
    },
  },
  {
    name: "simple query, results N (edit a block elsewhere on a large host page)",
    observer: "refs",
    mount: async (n) => {
      const q = "(task TODO)";
      backendReadsQueries({ [q]: { form: q } });
      const fx = outline(n, "pg");
      const host = pageDto("Window", [blk("h-q", `{{query ${q}}}`), ...fx.blocks]);
      const groups = refGroups(30);
      serve(host, ...groups.map(g => pageDto(g.page, g.blocks as Blk[])));
      vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Window", pageKind: "page" });
      const m = mountInto(() => <PageView />);
      await titleShown(m.host, "Window");
      await vi.waitFor(() => expect(doc.byId["rr-5"]).toBeDefined(), { timeout: 20000 });
      return { ...m, editId: fx.grand };
    },
  },
  {
    name: "advanced query, results N (edit a result)",
    enter: "click",
    observer: "refs",
    sizes: [200, 1000],
    mount: async (n) => {
      const adv = '{:query [:find (pull ?b [*]) :where [?b :block/marker "TODO"]]}';
      backendReadsQueries({ [adv]: { form: adv, kind: "advanced" } });
      const host = pageDto("Window", [blk("h-q", `{{query ${adv}}}`), blk("h-0", "Intro")]);
      const groups = refGroups(n);
      serve(host, ...groups.map(g => pageDto(g.page, g.blocks as Blk[])));
      vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Window", pageKind: "page" });
      const m = mountInto(() => <PageView />);
      await titleShown(m.host, "Window");
      const target = `rr-${Math.floor(n / 2)}`;
      await vi.waitFor(() => expect(doc.byId[target]).toBeDefined(), { timeout: 20000 });
      return { ...m, editId: target };
    },
  },
  {
    name: "query table view, results N (edit another block on the host page)",
    observer: "refs",
    sizes: [200, 1000],
    mount: async (n) => {
      const q = "(task TODO)";
      backendReadsQueries({ [q]: { form: q, legacy_table: true, view: { view: "table" } as never } });
      const host = pageDto("Window", [blk("h-q", `{{query ${q}}}\nquery-table:: true`), blk("h-0", "Intro")]);
      const groups = refGroups(n);
      serve(host, ...groups.map(g => pageDto(g.page, g.blocks as Blk[])));
      vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));
      mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Window", pageKind: "page" });
      const m = mountInto(() => <PageView />);
      await titleShown(m.host, "Window");
      const target = `rr-${Math.floor(n / 2)}`;
      await vi.waitFor(() => expect(m.host.querySelector("table, [class*='query-table'], [class*='sheet']")).not.toBeNull(), { timeout: 20000 });
      void target;
      return { ...m, editId: "h-0" };
    },
  },
  {
    name: "linked references, refs N (edit a reference)",
    enter: "click",
    observer: "refs",
    sizes: [200, 500],
    mount: async (n) => {
      setGraphMeta({ linked_references_collapsed_threshold: 1_000_000 } as never);
      const groups = refGroups(n);
      vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
      vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
      const m = await mountPage("Target", pageDto("Target", [blk("t-0", "Target page text")]), ...groups.map(g => pageDto(g.page, g.blocks as Blk[])));
      const target = `rr-${Math.floor(n / 2)}`;
      await vi.waitFor(() => expect(doc.byId[target]).toBeDefined(), { timeout: 20000 });
      // A clean verdict is vacuous if the references never mounted: every row is on screen here.
      expect(m.host.querySelectorAll('[data-block-id^="rr-"]').length).toBe(n);
      return { ...m, editId: target };
    },
  },
  {
    name: "linked references, large target page (edit a page block)",
    observer: "refs",
    mount: async (n) => {
      setGraphMeta({ linked_references_collapsed_threshold: 1_000_000 } as never);
      const fx = outline(n, "pg");
      const groups = refGroups(30);
      vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
      vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
      const m = await mountPage("Target", pageDto("Target", fx.blocks), ...groups.map(g => pageDto(g.page, g.blocks as Blk[])));
      await vi.waitFor(() => expect(doc.byId["rr-5"]).toBeDefined(), { timeout: 20000 });
      return { ...m, editId: fx.grand };
    },
  },
);

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});
async function mountSplit(left: string, right: string): Promise<{ host: HTMLElement; dispose: () => void }> {
  restorePaneLayout(
    { kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }] },
    new Map([["main", pageSnapshot(left)], ["pane-2", pageSnapshot(right)]]),
    "main",
  );
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PaneTree node={layoutRoot()} path={[]} />, root);
  const disposeKeys = installKeybindings();
  await vi.waitFor(() => expect(root.querySelectorAll("[data-pane-id] .page-title").length).toBeGreaterThanOrEqual(2));
  return { host: root, dispose: () => { disposeKeys(); dispose(); resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 }); } };
}

surfaces.push(
  {
    name: "split pane, same large page in both panes",
    mount: async (n) => {
      const fx = outline(n, "pg");
      serve(pageDto("Window", fx.blocks));
      const m = await mountSplit("Window", "Window");
      return { ...m, editId: fx.grand };
    },
  },
  {
    name: "split pane, large page left and unrelated small page right",
    mount: async (n) => {
      const fx = outline(n, "pg");
      serve(pageDto("Window", fx.blocks), pageDto("Other", outline(30, "ot").blocks));
      const m = await mountSplit("Window", "Other");
      return { ...m, editId: fx.grand };
    },
  },
  {
    name: "in-page find open with a query (routed page)",
    mount: async (n) => {
      const fx = outline(n, "pg");
      const m = await mountPage("Window", pageDto("Window", fx.blocks));
      // jsdom has no layout: Range#getClientRects is absent, and the overlay painter needs it.
      if (!Range.prototype.getClientRects) Range.prototype.getClientRects = (() => []) as never;
      const findHost = document.createElement("div"); document.body.append(findHost);
      const disposeFind = render(() => <InPageFind />, findHost);
      openInPageFind();
      setInPageFindQuery("unique");
      return { ...m, dispose: () => { disposeFind(); closeInPageFind({ restoreFocus: false }); setInPageFindQuery(""); m.dispose(); }, editId: fx.grand };
    },
  },
);

for (const view of ["table", "board", "grid"] as const) {
  surfaces.push({
    name: `sheet ${view} view, N rows (edit a row)`,
    observer: "refs",
    sizes: [200, 1000],
    edit: "setDoc",
    mount: async (n) => {
      const group = view === "board" ? "\ntine.group-by:: prop:owner" : "";
      const rows = Array.from({ length: n }, (_, i) => blk(`sh-${i}`, `**Row ${i}** task\nowner:: ${["Martin", "Jan", "Eva"][i % 3]}\nqty:: ${i}`));
      const owner = blk("sh-owner", `Sheet\ntine.view:: ${view}\ntine.fields:: owner=text;qty=number${group}`, rows);
      const nd = (id: string, raw: string, parent: string | null, children: string[] = []) => ({ id, raw, collapsed: false, parent, page: "Sheet", children });
      const byId: Record<string, ReturnType<typeof nd>> = { "sh-owner": nd("sh-owner", owner.raw, null, rows.map(r => r.id)) };
      for (const r of rows) byId[r.id] = nd(r.id, view === "grid" ? `Row` : r.raw, "sh-owner", view === "grid" ? [0, 1, 2].map(c => `${r.id}.${c}`) : []);
      // Grid rows are children of the owner and its cells are the rows' children.
      if (view === "grid") for (const r of rows) for (const c of [0, 1, 2]) byId[`${r.id}.${c}`] = nd(`${r.id}.${c}`, `cell ${r.id} ${c}`, r.id);
      setDoc({ byId, pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["sh-owner"], format: "md", readOnly: false, guide: false }], feed: ["Sheet"], loaded: true } as never);
      const m = mountInto(() => <Block id="sh-owner" />);
      await vi.waitFor(() => expect(m.host.querySelector("[class*='sheet']")).not.toBeNull(), { timeout: 5000 });
      return { ...m, editId: view === "grid" ? "sh-3.1" : `sh-${Math.floor(n / 2)}` };
    },
  });
}

/** Surfaces known to do per-keystroke work that grows with page/result/row count (the sweep notes name each computation). */
const GROWS = new Set([
  "journals feed (today) with compact task summary",
  "TodayTaskSummary component alone (page edited elsewhere)",
  "in-page find open with a query (routed page)",
  "simple query, results N (edit a result)",
  "advanced query, results N (edit a result)",
  "sheet table view, N rows (edit a row)",
  "sheet board view, N rows (edit a row)",
]);
/** The default (fast) set: zoom/plain-page controls, the compact-summary grower, and the fixed large-subtree window. */
const FAST = new Set([
  "routed page",
  "zoomed block (routed), edit the zoom root",
  "TodayTaskSummary component alone (page edited elsewhere)",
  "routed page, shape two-roots",
]);

surfaces.push({
  name: "unlinked references, refs N (edit an unlinked reference)",
  edit: "setDoc", // excerpt rows are read-only: there is no input path
  observer: "refs",
  sizes: [200, 500],
  mount: async (n) => {
    setGraphMeta({ linked_references_collapsed_threshold: 1_000_000 } as never);
    const groups = refGroups(n);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue(groups);
    const m = await mountPage("Target", pageDto("Target", [blk("t-0", "Target page text")]), ...groups.map(g => pageDto(g.page, g.blocks as Blk[])));
    // Unlinked references start collapsed: open the disclosure the way a user does.
    await vi.waitFor(() => expect(m.host.querySelector(".references-header")).not.toBeNull(), { timeout: 20000 });
    for (const h of m.host.querySelectorAll<HTMLElement>(".references-header")) h.click();
    const target = `rr-${Math.floor(n / 2)}`;
    await vi.waitFor(() => expect(doc.byId[target]).toBeDefined(), { timeout: 20000 });
    // Unlinked references render excerpt rows (no outline blocks); all must be mounted.
    expect(m.host.querySelectorAll(".reference-excerpt-row").length).toBe(n);
    return { ...m, editId: target };
  },
});

// ------------------------------------------------------------------ driver

const SIZES = [200, 2000];
const REPEATS = 3;
const envSizes = process.env.KEYSTROKE_SIZES?.split(",").map(Number);
const results: Record<string, Record<string, unknown>> = {};
const only = process.env.KEYSTROKE_ONLY?.split("|").filter(Boolean);
const fullSweep = !!process.env.KEYSTROKE_SWEEP || !!only;
const wanted = (name: string) => only ? only.some(o => name.includes(o)) : fullSweep || FAST.has(name);
const flush = () => { const out = process.env.KEYSTROKE_SWEEP_OUT; if (out) fs.writeFileSync(out, JSON.stringify(results, null, 1)); };

afterEach(() => {
  endEdit("page-navigation");
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  Observer.all = [];
  resetNearObserverForTests(); resetSharedQueryResultsForTests(); resetStore(); resetTabsToJournals();
  flags.compactTheme = false;
  setGraphMeta(null);
  document.body.replaceChildren();
});
beforeAll(async () => { await initParser(); });

let seq = 0;
async function edit(m: Mounted, via: "setDoc" | "input"): Promise<Measurement> {
  const id = m.editId;
  if (via === "setDoc") {
    const base = doc.byId[id].raw.replace(/ ?~\d+$/, "");
    return measure(id, () => setDoc("byId", id, "raw", `${base} ~${++seq}`));
  }
  const ta = document.querySelector<HTMLTextAreaElement>("textarea.block-editor");
  if (!ta) throw new Error("no editor textarea");
  return measure(id, () => {
    ta.value = ta.value + "x";
    ta.setSelectionRange(ta.value.length, ta.value.length);
    ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "x" }));
  });
}

async function run(surface: Surface, n: number, via: "setDoc" | "input") {
  flags.compactTheme = !!surface.compactTheme;
  
  vi.stubGlobal("IntersectionObserver", Observer);
  const m = await surface.mount(n);
  try {
    if (via === "input") {
      if (surface.enter === "click") {
        const content = document.querySelector<HTMLElement>(`[data-block-id="${m.editId}"] .block-content`);
        if (!content) throw new Error(`no rendered block ${m.editId} to click`);
        content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
        content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
      } else startEditing(m.editId, 0);
      await vi.waitFor(() => expect(document.querySelector("textarea.block-editor")).not.toBeNull());
    }
    const first = await edit(m, via);
    const steady: Measurement[] = [];
    for (let i = 0; i < REPEATS; i++) steady.push(await edit(m, via));
    const last = steady[steady.length - 1];
    return {
      first: summary(first),
      steady: { foreignRaw: median(steady.map(s => s.foreignRaw)), foreignAny: median(steady.map(s => s.foreignAny)),
        foreignRawNodes: median(steady.map(s => s.foreignRawNodes)), runs: median(steady.map(s => s.runs)), ms: median(steady.map(s => s.ms)) },
      sites: [...last.bySite.entries()]
        .filter(([, s]) => s.rawReads > 0 || s.anyReads > 0)
        .sort((a, b) => b[1].rawReads - a[1].rawReads || b[1].anyReads - a[1].anyReads).slice(0, 10)
        .map(([site, s]) => ({ site, runs: s.runs, rawReads: s.rawReads, anyReads: s.anyReads, readers: [...s.readers.entries()].slice(0, 3) })),
      runSites: [...last.bySite.entries()].sort((a, b) => b[1].runs - a[1].runs).slice(0, 8)
        .map(([site, s]) => ({ site, runs: s.runs, rawReads: s.rawReads, anyReads: s.anyReads })),
    };
  } finally { m.dispose(); endEdit("page-navigation"); resetStore(); document.body.replaceChildren(); }
}
const summary = (s: Measurement) => ({ foreignRaw: s.foreignRaw, foreignAny: s.foreignAny, foreignRawNodes: s.foreignRawNodes, runs: s.runs, ms: s.ms });

describe("keystroke scaling", () => {
  for (const surface of surfaces) {
    if (!wanted(surface.name)) continue;
    for (const via of (surface.edit === "setDoc" || !fullSweep ? ["setDoc"] : ["setDoc", "input"]) as ("setDoc" | "input")[]) {
      it(`${surface.name} / ${via}`, async () => {
        const out: Record<string, unknown> = {};
        const sizes = envSizes ?? (fullSweep ? surface.sizes ?? SIZES : [200, 1000]);
        for (const n of sizes) out[String(n)] = await run(surface, n, via);
        results[`${surface.name} / ${via}`] = out; flush();
        const small = (out[String(sizes[0])] as { steady: { foreignRaw: number; runs: number } }).steady;
        const big = (out[String(sizes[1])] as { steady: { foreignRaw: number; runs: number } }).steady;
        console.log(`${surface.name} / ${via}: foreignRaw ${small.foreignRaw} -> ${big.foreignRaw}; runs ${small.runs} -> ${big.runs}`);
        if (GROWS.has(surface.name)) {
          expect(big.foreignRaw, `${surface.name} no longer grows (${small.foreignRaw} -> ${big.foreignRaw}): fixed? remove it from GROWS`)
            .toBeGreaterThanOrEqual(small.foreignRaw * 2 + 8);
        } else {
          expect(big.foreignRaw, `${surface.name}: per-keystroke reads of other blocks' text grow with size (${small.foreignRaw} -> ${big.foreignRaw}): the GH #248 shape`)
            .toBeLessThanOrEqual(small.foreignRaw * 1.25 + 4);
        }
      }, 240_000);
    }
  }
  it("writes the report", () => {
    flush();
  });
});
void backendReadsQueries; void blockRunResult; void ({} as RefGroup);
