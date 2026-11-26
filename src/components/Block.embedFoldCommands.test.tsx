// GH #1032: "Collapse (one level)" on a block whose children are whole-block
// embeds folds each embed (the host owns the fold, as the embed row's own arrow
// does), not the block itself. Embeds the fold commands do not stand at keep
// their earlier answer.
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For } from "solid-js";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { blockProperty, hasSelection, liveDtoBlock, pageByName, resetStore, selectBlock, setRaw, topSelected, undo, undoTopTag } from "../document";
import { ensurePageLoaded, loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, Format, PageDto, RefGroup } from "../types";
import { Block, SurfaceContext } from "./Block";
import { endEdit } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { installKeybindings, runGlobalCommand } from "../keybindings";
import { bumpDataRev } from "../graphSession";
import { openPage, resetTabsToJournals } from "../router";
import { resolveBlockBatched } from "../resolveBatch";
import { embedHostSource } from "./macroFold";
import type { ParsedQuery, Source } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetNearObserverForTests } from "../lazyObserve";

class AllNearObserver implements IntersectionObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];
  constructor(private readonly callback: IntersectionObserverCallback) {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }
  unobserve(): void {}
  observe(target: Element): void {
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this);
  }
}

let disposeKeys: () => void;
beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});
afterAll(() => disposeKeys());
beforeEach(() => {
  bumpDataRev();
  vi.stubGlobal("IntersectionObserver", AllNearObserver);
});
afterEach(() => {
  endEdit("page-navigation");
  resetTabsToJournals();
  resetNearObserverForTests();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.innerHTML = "";
});

const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
const page = (name: string, blocks: BlockDto[], format: Format = "md", readOnly = false): PageDto =>
  ({ name, title: name, kind: "page", pre_block: null, blocks, format, ...(readOnly ? { read_only: true } : {}) });

/** The backend answers each embedded block from its source page. */
function mockResolve(source: PageDto, targets: BlockDto[]): void {
  const groups = new Map<string, RefGroup>(targets.map((b) => [b.id, { page: source.name, kind: source.kind, blocks: [{ ...b, children: [] }] }]));
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map((id) => groups.get(id) ?? null));
}

const tick = async (times = 4) => { for (let i = 0; i < times; i++) await new Promise((r) => setTimeout(r, 0)); };

function mount(ids: string[], surface = "main") {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <SurfaceContext.Provider value={surface}><For each={ids}>{(id) => <Block id={id} />}</For></SurfaceContext.Provider>, root);
  return { root, dispose };
}

/** Put the caret in `id`, then optionally select it (Escape). */
async function target(root: HTMLElement, id: string, mode: "caret" | "selected") {
  const content = root.querySelector<HTMLElement>(`[data-block-id="${id}"] .block-content`)!;
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  await tick();
  if (mode === "selected") {
    editorIn(root, id).keydown({ key: "Escape" });
    expect(hasSelection()).toBe(true);
    expect(topSelected()).toEqual([id]);
  }
}

const S1 = () => block("s1", "Source one", [block("s1c", "Child of one")]);
const S2 = () => block("s2", "Source two", [block("s2c", "Child of two")]);

/** Parent P on page Fixture, whose children are the given rows; sources on page Source. */
function loadParent(children: BlockDto[], sources: BlockDto[], format: Format = "md", sourceReadOnly = false) {
  const source = page("Source", sources, format, sourceReadOnly);
  mockResolve(source, sources.flatMap(function all(b): BlockDto[] { return [b, ...b.children.flatMap(all)]; }));
  loadSingle(page("Fixture", [block("p", "Parent", children)], format));
  ensurePageLoaded(source);
}

describe("Collapse (one level) over block embeds (GH #1032)", () => {
  it.each(["main", "sidebar:test"].flatMap((surface) => (["md", "org"] as const).flatMap((format) =>
    (["caret", "selected"] as const).map((mode) => ({ surface, format, mode })))))
  ("folds each embed and leaves the parent open: $surface / $format / $mode", async ({ surface, format, mode }) => {
    loadParent([block("h1", "{{embed ((s1))}}"), block("h2", "{{embed ((s2))}}")], [S1(), S2()], format);
    const sourceRaw = [doc.byId.s1.raw, doc.byId.s2.raw];
    const { root, dispose } = mount(["p"], surface);
    try {
      await tick();
      expect(root.textContent).toContain("Child of one");
      expect(root.textContent).toContain("Child of two");
      await target(root, "p", mode);
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(blockProperty("h1", "collapsed")).toBe("true");
      expect(blockProperty("h2", "collapsed")).toBe("true");
      expect(doc.byId.p.collapsed).toBe(false);
      expect(blockProperty("p", "collapsed")).toBeNull();
      expect(root.textContent).toContain("Source one");
      expect(root.textContent).toContain("Source two");
      expect(root.textContent).not.toContain("Child of one");
      expect(root.textContent).not.toContain("Child of two");
      expect([doc.byId.s1.raw, doc.byId.s2.raw]).toEqual(sourceRaw);
      expect(doc.byId.s1.collapsed).toBe(false);
      undo(); await tick();
      expect(blockProperty("h1", "collapsed")).toBeNull();
      expect(blockProperty("h2", "collapsed")).toBeNull();
      expect(root.textContent).toContain("Child of one");
      expect(root.textContent).toContain("Child of two");
    } finally { dispose(); }
  });

  it("Expand opens a folded embed as its arrow does, writing the host's override", async () => {
    loadParent([block("h1", "{{embed ((s1))}}"), block("h2", "{{embed ((s2))}}")], [S1(), S2()]);
    const { root, dispose } = mount(["p"]);
    try {
      await tick();
      await target(root, "p", "caret");
      runGlobalCommand("editor/collapse-children"); await tick();
      runGlobalCommand("editor/expand-children"); await tick();
      expect(blockProperty("h1", "collapsed")).toBe("false");
      expect(blockProperty("h2", "collapsed")).toBe("false");
      expect(root.textContent).toContain("Child of one");
      expect(doc.byId.s1.raw).toBe("Source one");
    } finally { dispose(); }
  });

  it("with no target on a routed page, folds a top-level embed through its host", async () => {
    const source = page("Source", [S1()]);
    mockResolve(source, [S1()]);
    loadSingle(page("Fixture", [block("h1", "{{embed ((s1))}}")]));
    ensurePageLoaded(source);
    const { root, dispose } = mount(["h1"]);
    try {
      await tick();
      openPage("Fixture", "page");
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(blockProperty("h1", "collapsed")).toBe("true");
      expect(doc.byId.s1.raw).toBe("Source one");
      expect(root.textContent).not.toContain("Child of one");
    } finally { dispose(); }
  });
});

describe("embeds the fold commands keep their earlier answer for (GH #1032 guards)", () => {
  it("a host with children of its own folds itself, as before", async () => {
    loadParent([block("h1", "{{embed ((s1))}}", [block("own", "Own kid")])], [S1()]);
    const { root, dispose } = mount(["p"]);
    try {
      await tick();
      await target(root, "p", "caret");
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(doc.byId.h1.collapsed).toBe(true);
      expect(blockProperty("h1", "collapsed")).toBe("true");
      expect(doc.byId.p.collapsed).toBe(false);
      expect(root.textContent).not.toContain("Own kid");
      expect(doc.byId.s1.raw).toBe("Source one");
    } finally { dispose(); }
  });

  it("a host of a read-only source is a leaf: the parent folds, as before", async () => {
    loadParent([block("h1", "{{embed ((s1))}}")], [S1()], "md", true);
    const { root, dispose } = mount(["p"]);
    try {
      await tick();
      expect(root.textContent).toContain("Child of one");
      await target(root, "p", "caret");
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(doc.byId.p.collapsed).toBe(true);
      expect(blockProperty("h1", "collapsed")).toBeNull();
    } finally { dispose(); }
  });

  it("a host of a host (a chain) is a leaf: the parent folds, as before", async () => {
    const t = block("t", "Deep source", [block("tc", "Deep child")]);
    // The middle embed has a child of its own, so only the chain rule keeps the outer host a leaf.
    loadParent([block("h1", "{{embed ((s1))}}")], [block("s1", "{{embed ((t))}}", [block("s1own", "Chain own kid")]), t]);
    const { root, dispose } = mount(["p"]);
    try {
      await tick(8);
      expect(root.textContent).toContain("Deep child");
      await target(root, "p", "caret");
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(doc.byId.p.collapsed).toBe(true);
      expect(blockProperty("h1", "collapsed")).toBeNull();
      expect(blockProperty("s1", "collapsed")).toBeNull();
    } finally { dispose(); }
  });

  it("Expand with no target over a page of ordinary leaves changes nothing and adds no Undo step", () => {
    loadSingle(page("Fixture", [block("a", "Leaf a"), block("b", "Leaf b")]));
    openPage("Fixture", "page");
    const before = undoTopTag();
    runGlobalCommand("editor/expand-children");
    expect(undoTopTag()).toBe(before);
    expect(doc.byId.a.raw).toBe("Leaf a");
  });
});

describe("query blocks keep their earlier answer (GH #1032 guards)", () => {
  beforeEach(() => {
    vi.spyOn(backend(), "parseQuery").mockImplementation(async (text) => ({
      query: { anchor: "block", filter: { kind: "raw", text: text.trim(), diagnostic_kind: "not_applicable" }, diagnostics: [],
        source: { kind: "og", original: text.trim(), og_options: "" } as Source },
      view: {}, legacy_table: false,
    }) as ParsedQuery);
  });
  const answer = (groups: RefGroup[]) => vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult(groups));

  it("an embed shown as a query result stays a result row: the query folds, the embed's host is untouched", async () => {
    const source = page("Source", [S1()]);
    const groups = new Map<string, RefGroup>([["s1", { page: "Source", kind: "page", blocks: [{ ...S1(), children: [] }] }]]);
    vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map((id) => groups.get(id) ?? null));
    loadSingle(page("Tasks", [block("query", "{{query (task TODO)}}"), block("src", "Holder", [block("hr", "{{embed ((s1))}}")])]));
    ensurePageLoaded(source);
    answer([{ page: "Tasks", kind: "page", blocks: [{ id: "hr", raw: "{{embed ((s1))}}", collapsed: false, children: [] }] }]);
    const { root, dispose } = mount(["query"]);
    try {
      await tick(8);
      expect(root.querySelector(".query-group")?.textContent).toContain("Child of one");
      selectBlock("query");
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(doc.byId.query.raw).toContain("collapsed:: true");
      expect(doc.byId.hr.raw).toBe("{{embed ((s1))}}");
    } finally { dispose(); }
  });

  it("on a read-only page, a query with an ordinary leaf child is still refused", async () => {
    loadSingle(page("Tasks", [block("src", "Holder", [block("todo", "TODO Result row", [block("kid", "Kid row")])])]));
    loadSingle(page("Locked", [block("query", "{{query (task TODO)}}", [block("x", "Leaf child")])], "md", true));
    answer([{ page: "Tasks", kind: "page", blocks: [{ id: "todo", raw: "TODO Result row", marker: "TODO", collapsed: false,
      children: [{ id: "kid", raw: "Kid row", collapsed: false, children: [] }] }] }]);
    const { root, dispose } = mount(["query"]);
    const shownKid = () => [...root.querySelectorAll('.query-group [data-block-id="kid"]')].some((el) => !el.closest(".collapsed"));
    try {
      await tick(8);
      expect(shownKid()).toBe(true);
      const before = undoTopTag();
      selectBlock("query");
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(shownKid()).toBe(true);
      expect(doc.byId.query.raw).toBe("{{query (task TODO)}}");
      expect(undoTopTag()).toBe(before);
    } finally { dispose(); }
  });
});

describe("the source identity check shared by the embed row and the fold commands", () => {
  const U = "11111111-1111-4111-8111-111111111111";
  it("an answer whose block no longer carries that id does not qualify", async () => {
    const s1 = block("s1", `Source one\nid:: ${U}`, [block("s1c", "Child of one")]);
    const source = page("Source", [s1]);
    const dto: BlockDto = { ...s1, children: [], properties: [["id", U]] };
    vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) =>
      ids.map((id) => id === U ? { page: "Source", kind: "page", blocks: [dto] } : null));
    loadSingle(page("Fixture", [block("p", "Parent", [block("h1", `{{embed ((${U}))}}`)])]));
    ensurePageLoaded(source);
    await resolveBlockBatched(U);
    expect(liveDtoBlock(dto, { page: "Source", kind: "page" })).toBe("s1");
    expect(embedHostSource("h1")).toBe("s1");
    // The block is still resident and the answer still cached, but its id is gone.
    setRaw("s1", "Source one");
    expect(doc.byId.s1).toBeDefined();
    expect(liveDtoBlock(dto, { page: "Source", kind: "page" })).toBeNull();
    expect(embedHostSource("h1")).toBeNull();
    expect(pageByName("Source")).toBeDefined();
  });
});
