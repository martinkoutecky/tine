import { describe, it, expect, vi, beforeEach, beforeAll } from "vitest";
import { initParser } from "../render/parse";
beforeAll(initParser);
import { render } from "solid-js/web";
import type { BlockDto, PageDto } from "../types";

// GH #605: a page marked `tine/journal-sections:: true` decides what shows under
// today's journal; with none, config.edn `:default-queries` and the agenda show
// as before. The query renderers and the block-macro detection are stubbed:
// this pins which sections are chosen and in what order, not query execution.
const h = vi.hoisted(() => ({
  pages: new Map<string, PageDto>(),
  marked: [] as string[],
  locateFails: false,
  lookupFails: false,
  toasts: [] as string[],
  created: [] as PageDto[],
  opened: [] as string[],
  sidebar: [] as string[],
  getPageWait: null as Promise<void> | null,
  createWait: null as Promise<void> | null,
  parseWait: null as Promise<void> | null,
  queryRuns: 0,
}));
vi.mock("../backend", () => ({
  backend: () => ({
    parseQuery: async (text: string) => { await h.parseWait; return { query: text, view: {} }; },
    queryRun: async () => {
      h.queryRuns += 1;
      if (h.locateFails) throw new Error("engine down");
      return { anchor: "page", pages: h.marked.map((name) => ({ name })) };
    },
    getPage: async (name: string) => { await h.getPageWait; if (h.lookupFails) throw new Error("lookup failed"); return h.pages.get(name) ?? null; },
  }),
}));
vi.mock("./Macro", () => ({
  QueryMacro: (props: { body: string; title?: string }) => <div class="q" data-body={props.body} data-title={props.title ?? ""} />,
}));
vi.mock("./BeginQuery", () => ({
  inspectBeginQuery: (raw: string) => raw.startsWith("#+BEGIN_QUERY") ? { kind: "query", query: raw } : null,
  BeginQuery: (props: { match: { query: string }; hideWhenEmpty?: boolean }) => <div class="bq" data-body={props.match.query} data-hide-empty={String(!!props.hideWhenEmpty)} />,
}));
vi.mock("./Rendered", () => ({
  detectMacro: (raw: string) => {
    const m = /^\{\{((?:tine-)?query .*)\}\}$/.exec(raw);
    return m ? { kind: "query", inner: m[1] } : null;
  },
}));
vi.mock("../ui", () => ({
  openPageInSidebar: (name: string) => h.sidebar.push(name),
  agendaQuery: () => "query (agenda)",
  agendaTql: () => "agenda tql",
}));
vi.mock("../guide", () => ({
  ensureGuidePagesLoaded: async () => [],
  guidePageName: (title: string) => `Tine-guide/${title}`,
}));
vi.mock("../document", () => ({
  createPage: async (name: string, dto: PageDto) => { h.created.push(dto); await h.createWait; h.pages.set(name, dto); return null; },
  journalSectionsPage: (name: string, marker: string, blocks: BlockDto[]): PageDto => ({ name, kind: "page", title: name, pre_block: marker, blocks }),
}));
vi.mock("../router", () => ({ openPage: (name: string) => { h.opened.push(name); } }));
vi.mock("../toasts", () => ({ pushToast: (text: string) => h.toasts.push(text) }));

import { bumpDataRev, bumpGraphEpoch, setGraphMeta } from "../graphSession";
import { invalidateBinding } from "../binding";
import { locateSectionsPages } from "../journalSections";
import { JournalSectionsHeader, TodaySections, customizeTodaySections } from "./TodaySections";
import type { GraphMeta } from "../types";

const block = (raw: string, children: BlockDto[] = []): BlockDto => ({ id: "", raw, collapsed: false, children });
const meta = (extra: Partial<GraphMeta> = {}) => ({
  default_journal_queries: [{ title: "From config", body: "query (task NOW)", error: null, seed: "#+BEGIN_QUERY\n{:query (task NOW)}\n#+END_QUERY" }],
  journal_config_diagnostics: [],
  disable_scheduled_and_deadline_query: false,
  preferred_format: "md",
  ...extra,
}) as unknown as GraphMeta;

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
async function mount() {
  const div = document.createElement("div");
  const dispose = render(() => <TodaySections today="Oct 14th, 2026" upcoming={<div class="upcoming-proof" />} />, div);
  await settle(); await settle();
  return { div, dispose };
}
const bodies = (div: HTMLElement) => [...div.querySelectorAll("[data-body]")].map((e) => e.getAttribute("data-body"));

beforeEach(() => {
  h.pages.clear(); h.marked = []; h.locateFails = false; h.created = []; h.opened = [];
  h.getPageWait = null; h.createWait = null;
  h.lookupFails = false; h.toasts = [];
  h.parseWait = null; h.queryRuns = 0;
  setGraphMeta(meta());
});

describe("today's sections (GH #605)", () => {
  it("GH #710: explains sections and opens the scheduling Guide in the sidebar", async () => {
    h.sidebar = [];
    const div = document.createElement("div");
    document.body.append(div);
    const dispose = render(() => <JournalSectionsHeader />, div);
    try {
      expect(div.textContent).toContain("This page controls what shows with today's journal");
      expect(div.textContent).toContain("{{upcoming}}");
      div.querySelector("button")!.click(); await settle();
      expect(h.sidebar).toEqual(["Tine-guide/Reference/Journals, tasks, and scheduling"]);
    } finally { dispose(); div.remove(); }
  });
  it("GH #710: places Upcoming in outline order and hides it live on removal or page deletion", async () => {
    h.marked = ["Sections"];
    const page: PageDto = { name: "Sections", kind: "page", title: "Sections", pre_block: null,
      blocks: [block("{{query (task TODO)}}"), block("notes", [block("{{upcoming}}")]), block("{{query (task DONE)}}")] };
    h.pages.set(page.name, page);
    const { div, dispose } = await mount();
    try {
      expect([...div.querySelectorAll(".q, .upcoming-proof")].map(e => e.className)).toEqual(["q", "upcoming-proof", "q"]);
      h.pages.set(page.name, { ...page, blocks: [block("notes")] }); bumpDataRev();
      await settle(); await settle();
      expect(div.querySelector(".upcoming-proof")).toBeNull();
      h.pages.set(page.name, page); bumpDataRev(); await settle(); await settle();
      expect(div.querySelector(".upcoming-proof")).not.toBeNull();
      h.marked = []; h.pages.delete(page.name); bumpDataRev(); await settle(); await settle();
      expect(div.querySelector(".upcoming-proof")).toBeNull();
    } finally { dispose(); }
  });
  it.each(["md", "org"] as const)("GH #710: only a standalone parsed Upcoming macro is a section in %s", async format => {
    h.marked = ["Sections"];
    h.pages.set("Sections", { name: "Sections", kind: "page", title: "Sections", pre_block: null, format,
      blocks: [block("Example {{upcoming}}"), block("`{{upcoming}}`"), block("{{upcoming invalid}}"), block("{{upcoming}}") ] });
    const { div, dispose } = await mount();
    try { expect(div.querySelectorAll(".upcoming-proof")).toHaveLength(1); }
    finally { dispose(); }
  });
  it("does not show temporary config sections before the sections-page lookup settles", async () => {
    let release!: () => void;
    h.getPageWait = new Promise<void>((resolve) => { release = resolve; });
    h.marked = ["My sections"];
    h.pages.set("My sections", { name: "My sections", kind: "page", title: "My sections", pre_block: null,
      blocks: [block("#+BEGIN_QUERY\n{:query (task NOW)}\n#+END_QUERY")] });
    const { div, dispose } = await mount();
    try {
      expect(bodies(div)).toEqual([]);
      release(); await settle(); await settle();
      expect(div.querySelector(".bq")?.getAttribute("data-hide-empty")).toBe("true");
    } finally { release(); dispose(); }
  });

  it("a graph switch while parsing the marker query cannot run it in the replacement graph", async () => {
    let release!: () => void;
    h.parseWait = new Promise<void>((resolve) => { release = resolve; });
    const pending = locateSectionsPages();
    invalidateBinding();
    release();
    expect(await pending).toEqual([]);
    expect(h.queryRuns).toBe(0);
  });

  it("a failed Customize name lookup reports the creation failure and writes nothing", async () => {
    h.lookupFails = true;
    await customizeTodaySections();
    expect(h.created).toEqual([]);
    expect(h.opened).toEqual([]);
    expect(h.toasts).toHaveLength(1);
    expect(h.toasts[0]).toContain("lookup failed");
  });

  it("a graph switch during the name lookup prevents creation in the replacement graph", async () => {
    let release!: () => void;
    h.getPageWait = new Promise<void>((resolve) => { release = resolve; });
    const pending = customizeTodaySections();
    invalidateBinding();
    release();
    await pending;
    expect(h.created).toEqual([]);
    expect(h.opened).toEqual([]);
  });

  it("a completed creation cannot route a replacement graph to the old graph's page", async () => {
    let release!: () => void;
    h.createWait = new Promise<void>((resolve) => { release = resolve; });
    const pending = customizeTodaySections();
    await settle();
    expect(h.created).toHaveLength(1);
    invalidateBinding();
    release();
    await pending;
    expect(h.opened).toEqual([]);
  });

  it("a display repaint during lookup still allows creation in the same graph", async () => {
    let release!: () => void;
    h.getPageWait = new Promise<void>((resolve) => { release = resolve; });
    const pending = customizeTodaySections();
    bumpGraphEpoch();
    release();
    await pending;
    expect(h.created).toHaveLength(1);
    expect(h.opened).toEqual(["Journal sections"]);
  });

  it("saved query edits refresh the sections without remounting, and deletion restores config", async () => {
    h.marked = ["My sections"];
    const page: PageDto = { name: "My sections", kind: "page", title: "My sections", pre_block: null, blocks: [block("{{query (task TODO)}}")] };
    h.pages.set(page.name, page);
    const { div, dispose } = await mount();
    expect(bodies(div)).toEqual(["query (task TODO)"]);
    h.pages.set(page.name, { ...page, blocks: [block("{{query (task DONE)}}")] });
    bumpDataRev();
    await settle(); await settle();
    expect(bodies(div)).toEqual(["query (task DONE)"]);
    h.marked = []; h.pages.delete(page.name);
    bumpDataRev();
    await settle(); await settle();
    expect(bodies(div)).toEqual(["query (task NOW)", "query (agenda)"]);
    dispose();
  });
  it("without a sections page, shows config.edn queries then the agenda, and offers Customize", async () => {
    const { div, dispose } = await mount();
    expect(bodies(div)).toEqual(["query (task NOW)", "query (agenda)"]);
    expect(div.textContent).toContain("Customize today's sections");
    dispose();
  });

  it("a marked page replaces config and agenda with its query blocks, in outline order, any depth", async () => {
    h.marked = ["My sections"];
    h.pages.set("My sections", { name: "My sections", kind: "page", title: "My sections", pre_block: "tine/journal-sections:: true", format: "md",
      blocks: [block("a note to self"), block("{{query (task LATER)}}", [block("#+BEGIN_QUERY\n{:query (task NOW)}\n#+END_QUERY")]), block("{{tine-query tasks}}")] });
    const { div, dispose } = await mount();
    expect(bodies(div)).toEqual(["query (task LATER)", "#+BEGIN_QUERY\n{:query (task NOW)}\n#+END_QUERY", "tine-query tasks"]);
    expect(div.textContent).not.toContain("a note to self");
    expect(div.textContent).not.toContain("Customize");
    dispose();
  });

  it("an empty marked page shows no sections at all (the user removed them)", async () => {
    h.marked = ["Empty"];
    h.pages.set("Empty", { name: "Empty", kind: "page", title: "Empty", pre_block: "tine/journal-sections:: true", blocks: [] });
    const { div, dispose } = await mount();
    expect(bodies(div)).toEqual([]);
    dispose();
  });

  it("several marked pages: the first by name is used and the rest are named", async () => {
    h.marked = ["Zed", "Alpha"];
    for (const name of h.marked)
      h.pages.set(name, { name, kind: "page", title: name, pre_block: null, blocks: [block(`{{query (page ${name})}}`)] });
    const { div, dispose } = await mount();
    expect(bodies(div)).toEqual(["query (page Alpha)"]);
    expect(div.querySelector("[role=status]")?.textContent).toContain("Zed");
    dispose();
  });

  it("a failed locate says so instead of silently showing config sections", async () => {
    h.locateFails = true;
    const { div, dispose } = await mount();
    expect(div.querySelector(".resource-failure")).not.toBeNull();
    expect(bodies(div)).toEqual([]);
    dispose();
  });

  it("Customize seeds the page from config and the agenda, never over an existing page, and opens it", async () => {
    h.pages.set("Journal sections", { name: "Journal sections", kind: "page", title: "Journal sections", pre_block: null, blocks: [block("mine")] });
    await customizeTodaySections();
    expect(h.created.map((p) => p.name)).toEqual(["Journal sections 2"]);
    expect(h.created[0].pre_block).toBe("tine/journal-sections:: true");
    expect(h.created[0].blocks.slice(1).map((b) => b.raw)).toEqual(["#+BEGIN_QUERY\n{:query (task NOW)}\n#+END_QUERY", "{{tine-query agenda tql}}"]);
    expect(h.opened).toEqual(["Journal sections 2"]);
  });

  it("Customize in an Org graph writes the Org marker and omits a disabled agenda", async () => {
    setGraphMeta(meta({ preferred_format: "org", disable_scheduled_and_deadline_query: true } as Partial<GraphMeta>));
    await customizeTodaySections();
    expect(h.created[0].pre_block).toBe("#+tine/journal-sections: true");
    expect(h.created[0].blocks.slice(1).map((b) => b.raw)).toEqual(["#+BEGIN_QUERY\n{:query (task NOW)}\n#+END_QUERY"]);
  });
});
