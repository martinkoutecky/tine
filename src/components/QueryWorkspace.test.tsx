import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import type { PaneRouter, QueryRoute } from "../router";
import type { QueryExecution, ResolvedPage } from "../types";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  registerTransientLayer,
} from "../transientLayers";
import {
  QueryWorkspace,
  materializeQueryWorkspace,
  type MaterializeQueryDependencies,
  type QueryWorkspaceDependencies,
} from "./QueryWorkspace";
import { bumpDataRev, bumpGraphEpoch, pageInventoryRev } from "../graphSession";
import { startEditing, endEdit } from "../editorController";
import { backend } from "../backend";
import { resetStore } from "../document";
import { contextMenu, closeAllRightSidebarItems, rightSidebar } from "../ui";
import { resetPaneLayoutToSingle, paneRouter, layoutPaneIds } from "../panes";
import { resetQueryTextOpenForTests } from "../navSettings";

afterEach(() => {
  clearTransientLayersForTest();
  resetQueryTextOpenForTests(false);
  document.body.innerHTML = "";
});

function materializeDeps(overrides: Partial<MaterializeQueryDependencies> = {}): MaterializeQueryDependencies {
  return {
    resolvePage: vi.fn(async (name: string) => ({ kind: "absent" as const, id: `pages/${name}.md` })),
    savePages: vi.fn(async () => ({ ok: ["rev-new"] })),
    runGraphSearch: vi.fn(async () => ({ hits: [], diagnostics: [], explanation: { branches: [{ description: "valid", children: [] }] }, cancelled: false })),
    ...overrides,
  };
}

describe("materializeQueryWorkspace", () => {
  it("does not save when a page resolve lands after a graph switch (I-20)", async () => {
    let finish!: (value: { kind: "absent"; id: string }) => void;
    const deps = materializeDeps({
      resolvePage: vi.fn(() => new Promise<ResolvedPage>((resolve) => { finish = resolve; })),
    });
    const materializing = materializeQueryWorkspace({ title: "Saved", sourceKind: "dsl", source: "(todo TODO)", presentation: "list", routeId: "old-graph" }, deps);
    await vi.waitFor(() => expect(deps.resolvePage).toHaveBeenCalled());
    resetStore();
    finish({ kind: "absent", id: "pages/Saved.md" });
    expect(await materializing).toMatchObject({ ok: false, kind: "error" });
    expect(deps.savePages).not.toHaveBeenCalled();
  });

  it("rejects empty, exclusion-only, and Rust-diagnostic friendly searches before any graph write", async () => {
    for (const source of ["   ", "-draft", "/(a)\\1/"]) {
      const deps = materializeDeps({ runGraphSearch: vi.fn(async () => source === "-draft"
        ? { hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false }
        : { hits: [], diagnostics: [{ code: "invalid_regex", message: "invalid regex" }], explanation: { branches: [] }, cancelled: false }) });
      const result = await materializeQueryWorkspace({ title: "Unsafe", sourceKind: "search", source, presentation: "search", routeId: "query-unsafe" }, deps);
      expect(result.ok).toBe(false);
      expect(deps.resolvePage).not.toHaveBeenCalled();
      expect(deps.savePages).not.toHaveBeenCalled();
    }
  });
  it("uses an explicit stable route lane and zero-limit Rust validation for every nonblank friendly save", async () => {
    const validate = vi.fn(async () => ({ hits: [], diagnostics: [], explanation: { branches: [{ description: "valid", children: [] }] }, cancelled: false }));
    const deps = materializeDeps({ runGraphSearch: validate });
    const input = { title: "Saved", sourceKind: "search" as const, source: " alpha ", presentation: "search" as const, routeId: "query-stable" };
    await materializeQueryWorkspace(input, deps);
    await materializeQueryWorkspace(input, deps);
    expect(validate).toHaveBeenCalledTimes(2);
    expect(validate).toHaveBeenNthCalledWith(1, "alpha", 0, 0, "query-workspace:query-stable:materialize", true);
    expect(validate).toHaveBeenNthCalledWith(2, "alpha", 0, 0, "query-workspace:query-stable:materialize", true);

    const blank = materializeDeps();
    await materializeQueryWorkspace({ ...input, source: "   " }, blank);
    expect(blank.runGraphSearch).not.toHaveBeenCalled();
    expect(blank.resolvePage).not.toHaveBeenCalled();
    expect(blank.savePages).not.toHaveBeenCalled();
  });
  it("rejects JavaScript-invalid, cancelled, and failed Rust validation before page lookup", async () => {
    const input = { title: "Unsafe", sourceKind: "search" as const, source: "/(unclosed/", presentation: "search" as const, routeId: "query-rejected" };
    const diagnostic = materializeDeps({ runGraphSearch: vi.fn(async () => ({ hits: [], diagnostics: [{ code: "invalid_regex", message: "invalid regex" }], explanation: { branches: [] }, cancelled: false })) });
    await materializeQueryWorkspace(input, diagnostic);
    expect(diagnostic.runGraphSearch).toHaveBeenCalledWith("/(unclosed/", 0, 0, "query-workspace:query-rejected:materialize", true);
    expect(diagnostic.resolvePage).not.toHaveBeenCalled(); expect(diagnostic.savePages).not.toHaveBeenCalled();
    const cancelled = materializeDeps({ runGraphSearch: vi.fn(async () => ({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: true })) });
    await materializeQueryWorkspace({ ...input, source: "alpha" }, cancelled);
    expect(cancelled.resolvePage).not.toHaveBeenCalled(); expect(cancelled.savePages).not.toHaveBeenCalled();
    const failed = materializeDeps({ runGraphSearch: vi.fn(async () => { throw new Error("IPC unavailable"); }) });
    await materializeQueryWorkspace({ ...input, source: "alpha" }, failed);
    expect(failed.resolvePage).not.toHaveBeenCalled(); expect(failed.savePages).not.toHaveBeenCalled();
  });
  it("creates one canonical friendly query block through the guarded no-baseline save", async () => {
    const deps = materializeDeps();
    const beforeInventory = pageInventoryRev();
    const result = await materializeQueryWorkspace({
      title: "  Project dashboard  ",
      sourceKind: "search",
      source: "alpha -draft",
      presentation: "search",
      routeId: "query-project-dashboard",
    }, deps);

    expect(result.ok).toBe(true);
    if (!result.ok) throw new Error(result.message);
    expect(result.page).toEqual({
      name: "Project dashboard",
      kind: "page",
      title: "Project dashboard",
      pre_block: null,
      format: "md",
      blocks: [{
        id: "",
        raw: '{{query (search "alpha -draft")}}\ntine.view:: search',
        collapsed: false,
        children: [],
      }],
    });
    expect(deps.resolvePage).toHaveBeenCalledWith("Project dashboard", "page");
    expect(deps.savePages).toHaveBeenCalledTimes(1);
    // Saved to the backend's Absent id (its name format and preferred format).
    expect(deps.savePages).toHaveBeenCalledWith([{ id: "pages/Project dashboard.md", page: result.page, baseRev: null, force: false, kinds: ["create-page"] }], 1);
    expect(pageInventoryRev()).toBeGreaterThan(beforeInventory);
  });

  it("saves page-content membership as a query block property", async () => {
    const deps = materializeDeps();
    const result = await materializeQueryWorkspace({ title: "Content search", sourceKind: "search", source: "alpha",
      presentation: "search", pageMatchScope: "content", routeId: "content" }, deps);
    expect(result.ok).toBe(true);
    if (!result.ok) throw new Error(result.message);
    expect(result.page.blocks[0].raw).toBe('{{query (search "alpha")}}\ntine.view:: search\ntine.page-match-scope:: content');
  });

  it("materializes independent Display settings through the scoped writer", async () => {
    const result = await materializeQueryWorkspace({ title: "Search table", sourceKind: "search", source: "alpha",
      presentation: "search", pagePresentation: "table", blockPresentation: "list",
      pageDisplay: { columns: ["prop:owner"] }, routeId: "scoped-save" }, materializeDeps());
    expect(result.ok).toBe(true);
    if (!result.ok) throw new Error(result.message);
    expect(result.page.blocks[0].raw).toBe('{{query (search "alpha")}}\ntine.view:: search\ntine.page-view:: table\ntine.page-display:: 1\ntine.page-columns:: prop:owner\ntine.block-view:: list');
  });

  it("preserves canonical raw DSL and writes a presentation property only when needed", async () => {
    const listDeps = materializeDeps();
    const list = await materializeQueryWorkspace({
      title: "Tasks",
      sourceKind: "dsl",
      source: "  (and (todo TODO) (priority A))  ",
      presentation: "list",
      routeId: "query-tasks",
    }, listDeps);
    expect(list.ok && list.page.blocks).toHaveLength(1);
    expect(list.ok && list.page.blocks[0].raw).toBe("{{query (and (todo TODO) (priority A))}}");

    const tableDeps = materializeDeps();
    const table = await materializeQueryWorkspace({
      title: "Task table",
      sourceKind: "dsl",
      source: "(todo TODO)",
      presentation: "table",
      routeId: "query-task-table",
    }, tableDeps);
    expect(table.ok && table.page.blocks).toHaveLength(1);
    expect(table.ok && table.page.blocks[0].raw).toBe("{{query (todo TODO)}}\ntine.view:: table");
  });

  it("refuses an existing page without attempting a write", async () => {
    const deps = materializeDeps({
      resolvePage: vi.fn(async () => ({ kind: "existing" as const, id: "pages/Taken.md", others: [] })),
    });
    const result = await materializeQueryWorkspace({
      title: "Taken",
      sourceKind: "search",
      source: "alpha",
      presentation: "list",
      routeId: "query-taken",
    }, deps);

    expect(result).toMatchObject({ ok: false, kind: "exists" });
    expect(deps.savePages).not.toHaveBeenCalled();
  });

  it("refuses an alias title without a write and keeps the query in the workspace (B15b)", async () => {
    const deps = materializeDeps({
      resolvePage: vi.fn(async () => ({ kind: "alias" as const, owners: ["pages/Owner.md"] })),
    });
    const result = await materializeQueryWorkspace({
      title: "Nickname",
      sourceKind: "search",
      source: "alpha",
      presentation: "list",
      routeId: "query-alias",
    }, deps);

    expect(result).toMatchObject({ ok: false, kind: "exists" });
    expect(result.ok ? "" : result.message).toContain("alias");
    expect(deps.savePages).not.toHaveBeenCalled();
  });

  it("keeps the workspace virtual when a create race reaches the save guard", async () => {
    const deps = materializeDeps({
      savePages: vi.fn(async () => { throw new Error("conflict"); }),
    });
    const result = await materializeQueryWorkspace({
      title: "Raced",
      sourceKind: "search",
      source: "alpha",
      presentation: "board",
      routeId: "query-raced",
    }, deps);

    expect(result).toMatchObject({ ok: false, kind: "conflict" });
    expect(deps.savePages).toHaveBeenCalledTimes(1);
  });

  it("writes an Org graph's view and scope properties in a :PROPERTIES: drawer, not as body text (GH #25 class)", async () => {
    const deps = materializeDeps({
      resolvePage: vi.fn(async (name: string) => ({ kind: "absent" as const, id: `pages/${name}.org` })),
    });
    const result = await materializeQueryWorkspace({
      title: "Org saved", sourceKind: "search", source: "alpha", presentation: "table",
      pageMatchScope: "content", routeId: "query-org",
    }, deps);
    if (!result.ok) throw new Error(result.message);
    expect(result.page.format).toBe("org");
    const raw = result.page.blocks[0].raw;
    expect(raw).toBe('{{query (search "alpha")}}\n:PROPERTIES:\n:tine.view: table\n:tine.page-match-scope: content\n:END:');
    // The markdown spelling in an Org file is visible text that is never read back.
    expect(raw).not.toContain("::");
    expect(deps.savePages).toHaveBeenCalledWith([expect.objectContaining({ id: "pages/Org saved.org" })], 1);
  });

  it("keeps a markdown graph's property lines byte-identical to before", async () => {
    const deps = materializeDeps();
    const result = await materializeQueryWorkspace({
      title: "Md saved", sourceKind: "search", source: "alpha", presentation: "table",
      pageMatchScope: "content", routeId: "query-md",
    }, deps);
    if (!result.ok) throw new Error(result.message);
    expect(result.page.format).toBe("md");
    expect(result.page.blocks[0].raw).toBe('{{query (search "alpha")}}\ntine.view:: table\ntine.page-match-scope:: content');
  });

  it("refuses locally, writing nothing, when the caller's input moved on during the validation or the title lookup", async () => {
    for (const moveAt of ["validation", "resolve"] as const) {
      let current = true;
      const deps = materializeDeps({
        runGraphSearch: vi.fn(async () => {
          if (moveAt === "validation") current = false;
          return { hits: [], diagnostics: [], explanation: { branches: [{ description: "valid", children: [] }] }, cancelled: false };
        }),
        resolvePage: vi.fn(async (name: string) => {
          if (moveAt === "resolve") current = false;
          return { kind: "absent" as const, id: `pages/${name}.md` };
        }),
      });
      const before = pageInventoryRev();
      const result = await materializeQueryWorkspace(
        { title: "Late", sourceKind: "search", source: "alpha", presentation: "list", routeId: `query-${moveAt}` },
        deps,
        () => current,
      );
      expect(result, moveAt).toMatchObject({ ok: false, kind: "superseded" });
      expect(deps.savePages, moveAt).not.toHaveBeenCalled();
      expect(pageInventoryRev(), moveAt).toBe(before);
    }
  });
});

function routerMock(activeRoute: QueryRoute = { kind: "query", id: "query-mock", sourceKind: "search", source: "", presentation: "search" }) {
  return {
    route: vi.fn(() => activeRoute),
    updateActiveQuery: vi.fn(),
    replaceActiveRoute: vi.fn(),
    openPage: vi.fn(),
    openInNewTab: vi.fn(),
    openPageTarget: vi.fn(),
    openPageAtBlock: vi.fn(),
  } as unknown as PaneRouter;
}

function executionFixture(explained: boolean): QueryExecution {
  return {
    hits: [
      {
        entity: "page",
        page: { name: "Alpha notes", kind: "page", date_key: null, path: "pages/alpha.md" },
        display_text: "Alpha notes",
        evidence: [{
          clause_id: 1,
          field: "page_name",
          mode: "fuzzy",
          spans: [{ start: 0, end: 5 }],
          score: 100,
        }],
        score: 100,
      },
      {
        entity: "block",
        page: "Research",
        kind: "page",
        path: "pages/client-b/Research.md",
        block: {
          id: "block-1",
          raw: "An alpha result",
          collapsed: false,
          children: [],
          breadcrumb: ["Parent"],
          properties: [["ID", "authored-block-1"]],
        },
        display_text: "An alpha result",
        evidence: [{
          clause_id: 2,
          field: "visible_content",
          mode: "contains",
          spans: [{ start: 3, end: 8 }],
        }],
      },
    ],
    diagnostics: [{ code: "bounded", message: "Results are limited for this preview." }],
    explanation: {
      branches: explained ? [{
        description: "Search page names and visible block text",
        children: [{ clause_id: 2, description: "contains alpha", children: [] }],
      }] : [],
    },
    cancelled: false,
  };
}

function workspaceDeps(): QueryWorkspaceDependencies {
  return {
    resolvePage: vi.fn(async (name: string) => ({ kind: "absent" as const, id: `pages/${name}.md` })),
    savePages: vi.fn(async () => ({ ok: ["saved-rev"] })),
    runGraphSearch: vi.fn(async (_source, pageLimit, blockLimit, _lane, explain) =>
      pageLimit === 0 && blockLimit === 0
        ? { hits: [], diagnostics: [], explanation: { branches: [{ description: "valid", children: [] }] }, cancelled: false }
        : executionFixture(explain)),
    parseQuery: vi.fn((source: string) => backend().parseQuery(source, "macro_query")),
    queryRun: vi.fn(async () => ({
      anchor: "block" as const, groups: [], diagnostics: [],
      report: { ran: [], ignored: [], supported: true }, total: 0, exceeded: false,
    })),
    queryExplainEmpty: vi.fn(async () => ({ rows: [], diagnostics: [], report: { ran: [], ignored: [], supported: true } })),
  };
}

async function waitFor(check: () => void): Promise<void> {
  await vi.waitFor(check, { timeout: 1_000, interval: 5 });
}

describe("QueryWorkspace", () => {
  it("GH #660: a saved revision refreshes routed query membership after editing finishes", async () => {
    const route: QueryRoute = { kind: "query", id: "membership", sourceKind: "search", source: "alpha", presentation: "search" };
    const deps = workspaceDeps();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock(route)} deps={deps} />, root);
    try {
      await waitFor(() => expect(root.textContent).toContain("An alpha result"));
      startEditing("block-1");
      vi.mocked(deps.runGraphSearch).mockResolvedValue({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false });
      bumpDataRev();
      await Promise.resolve(); await Promise.resolve();
      expect(root.textContent).toContain("An alpha result");
      endEdit("blur");
      await waitFor(() => expect(root.textContent).not.toContain("An alpha result"));
      expect(deps.runGraphSearch).toHaveBeenCalledTimes(2);
    } finally { endEdit("blur"); dispose(); }
  });
  it("sends both effective Display views to live friendly search", async () => {
    const route: QueryRoute = { kind: "query", id: "live-display", sourceKind: "search", source: "alpha",
      presentation: "search", pagePresentation: "board", blockPresentation: "list",
      pageDisplay: { sort: [["name", "desc"]], group_by: "prop:owner", sample: 2 },
      blockDisplay: { sort: [["priority", "asc"]], sample: 1 } };
    const deps = workspaceDeps();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock(route)} deps={deps} />, root);
    try {
      await waitFor(() => expect(deps.runGraphSearch).toHaveBeenCalledWith("alpha", 40, 100,
        "query-workspace:live-display", false, undefined, "names", {
          page: expect.objectContaining({ sort: [["name", "desc"]], group_by: "prop:owner", sample: 2 }),
          block: expect.objectContaining({ sort: [["priority", "asc"]], sample: 1 }),
        }));
    } finally { dispose(); }
  });
  it("keeps independent section Display choices and shows authored page columns", async () => {
    const route: QueryRoute = { kind: "query", id: "scoped-display", sourceKind: "search", source: "alpha",
      presentation: "search", pagePresentation: "table", blockPresentation: "list",
      pageDisplay: { columns: ["prop:owner"] } };
    const deps = workspaceDeps();
    vi.mocked(deps.runGraphSearch).mockResolvedValue({ ...executionFixture(false), hits: executionFixture(false).hits.map((hit) =>
      hit.entity === "page" ? { ...hit, row: { path: "pages/alpha.md", name: "Alpha notes", kind: "page",
        properties: [["Owner", "Mira"]] } } : hit) });
    const router = routerMock(route);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);
    try {
      await waitFor(() => expect(root.querySelector('[data-query-result-kind="page"] table')).not.toBeNull());
      expect(root.querySelector('[data-query-result-kind="page"] thead')?.textContent).toContain("owner");
      expect(root.querySelector('[data-query-result-kind="page"] tbody')?.textContent).toContain("Mira");
      expect(root.querySelector('[data-query-result-kind="block"] ul')).not.toBeNull();
      expect(root.querySelectorAll(".qd-trigger")).toHaveLength(2);
      root.querySelector<HTMLButtonElement>('[data-query-result-kind="page"] .qd-trigger')!.click();
      root.querySelector<HTMLButtonElement>('[data-query-result-kind="page"] [role="group"][aria-label="Query view"] button:nth-child(4)')!.click();
      expect(router.updateActiveQuery).toHaveBeenCalledWith(expect.objectContaining({ pagePresentation: "board" }));
      expect(router.updateActiveQuery).not.toHaveBeenCalledWith(expect.objectContaining({ blockPresentation: "board" }));
    } finally { dispose(); }
  });
  it("routes and persists page match scope through the search control", async () => {
    const route: QueryRoute = { kind: "query", id: "scope-test", sourceKind: "search", source: "alpha", presentation: "search" };
    const deps = workspaceDeps();
    const router = routerMock(route);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);
    try {
      await waitFor(() => expect(deps.runGraphSearch).toHaveBeenCalledWith("alpha", 40, 100, "query-workspace:scope-test", false, undefined, "names", expect.any(Object)));
      const select = root.querySelector<HTMLSelectElement>('[aria-label="Pages match"]')!;
      select.value = "content";
      select.dispatchEvent(new Event("change", { bubbles: true }));
      await waitFor(() => expect(deps.runGraphSearch).toHaveBeenCalledWith("alpha", 40, 100, "query-workspace:scope-test", false, undefined, "content", expect.any(Object)));
      expect(router.updateActiveQuery).toHaveBeenCalledWith({ pageMatchScope: "content" });
    } finally { dispose(); }
  });
  it("keeps Pages and Blocks as separate result sections, including an empty family", async () => {
    const route: QueryRoute = { kind: "query", id: "section-test", sourceKind: "search", source: "alpha", presentation: "search" };
    const deps = workspaceDeps();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock(route)} deps={deps} />, root);
    try {
      await waitFor(() => expect(root.querySelectorAll("[data-query-result-kind]")).toHaveLength(2));
      expect([...root.querySelectorAll("[data-query-result-kind] h3")].map((heading) => heading.textContent?.split(" ")[0])).toEqual(["Pages", "Blocks"]);
      await waitFor(() => expect(root.querySelector('[data-query-result-kind="page"]')?.textContent).toContain("Alpha notes"));
      expect(root.querySelector('[data-query-result-kind="page"]')?.textContent).toContain("Alpha notes");
      expect(root.querySelector('[data-query-result-kind="block"]')?.textContent).toContain("An alpha result");
      vi.mocked(deps.runGraphSearch).mockResolvedValue({ ...executionFixture(false), hits: executionFixture(false).hits.filter((hit) => hit.entity === "block") });
      const input = root.querySelector<HTMLInputElement>(".query-workspace-source")!;
      input.value = "beta"; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await waitFor(() => expect(root.querySelector('[data-query-result-kind="page"]')?.textContent).toContain("No matching pages."));
      expect(root.querySelector('[data-query-result-kind="block"]')?.textContent).toContain("An alpha result");
    } finally { dispose(); }
  });
  it("opens an alias match through the owner path and marks each family's truncation", async () => {
    const route: QueryRoute = { kind: "query", id: "alias-section", sourceKind: "search", source: "nickname", presentation: "search" };
    const deps = workspaceDeps();
    vi.mocked(deps.runGraphSearch).mockResolvedValue({
      ...executionFixture(false),
      hits: [{ entity: "page", page: { name: "Owner", kind: "page", path: "pages/owner.md", date_key: null },
        display_text: "Nickname", evidence: [], score: 100, matched_alias: "Nickname" }],
      has_more: { pages: true, blocks: false },
    });
    const router = routerMock(route);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);
    try {
      await waitFor(() => expect(root.querySelector('[data-query-result-kind="page"]')?.textContent).toContain("matched alias Nickname"));
      expect(root.querySelector('[data-query-result-kind="page"]')?.textContent).toContain("More pages match than are shown.");
      expect(root.querySelector('[data-query-result-kind="block"]')?.textContent).not.toContain("More blocks");
      root.querySelector<HTMLButtonElement>('[data-query-result-kind="page"] .query-result-row')!.click();
      expect(router.openInNewTab).toHaveBeenCalledWith({ kind: "page", name: "Owner", pageKind: "page", path: "pages/owner.md" }, true);
    } finally { dispose(); }
  });
  it("shows the block excerpt that admitted a page by content", async () => {
    const route: QueryRoute = { kind: "query", id: "content-excerpt", sourceKind: "search", source: "needle", presentation: "search", pageMatchScope: "content" };
    const deps = workspaceDeps();
    vi.mocked(deps.runGraphSearch).mockResolvedValue({ ...executionFixture(false), hits: [{
      entity: "page", page: { name: "Owner", kind: "page", path: "pages/Owner.md", date_key: null },
      display_text: "a hidden needle", evidence: [{ clause_id: 2, field: "visible_content", mode: "contains", spans: [{ start: 9, end: 15 }] }], score: 1,
    }] });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock(route)} deps={deps} />, root);
    try {
      await waitFor(() => expect(root.querySelector(".query-page-content-excerpt")?.textContent).toBe("a hidden needle"));
      expect(root.querySelector(".query-page-content-excerpt mark")?.textContent).toBe("needle");
      expect(root.querySelector('[data-query-result-kind="page"]')?.textContent).toContain("Owner");
    } finally { dispose(); }
  });
  // Ported from master src/components/QueryWorkspace.test.tsx (same title).
  it("peels a QueryBuilder child before its Advanced parent and preserves the draft", async () => {
    const route: QueryRoute = {
      kind: "query",
      id: "query-transient-ladder",
      sourceKind: "dsl",
      source: "(and (task TODO))",
      presentation: "list",
    };
    // The pane shows what the PRINTER returned (I-12); the dev-preview backend
    // has no printer, so this test says what Rust would answer.
    vi.spyOn(backend(), "printQuery").mockResolvedValue(route.source);
    // GH #619 item 4: the text pane is behind a remembered toggle; this test is about the pane.
    resetQueryTextOpenForTests(true);
    const lower = vi.fn(() => true);
    const unregisterLower = registerTransientLayer({ id: "query-workspace-lower", dismiss: lower });
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock(route)} deps={workspaceDeps()} />, root);
    try {
      const toggle = root.querySelector<HTMLButtonElement>(".query-advanced-toggle")!;
      toggle.click();
      await Promise.resolve();
      const dialog = root.querySelector<HTMLElement>(".query-advanced-modal")!;
      expect(dialog).not.toBeNull();

      // The sheet is over the IR now, so its rows appear once the ENGINE has
      // read the route's text — there is no frontend parser left to do it
      // synchronously. In the workspace the sheet is always open and NOT
      // portalled: it is inside the Advanced modal, so the modal's Tab trap
      // keeps containing it.
      // The dev-preview backend has no parser, so this route's text comes back
      // as ONE retained row — which still has its own ⋮ popover to peel.
      await waitFor(() => expect(root.querySelector(".qs-row .qs-row-menu")).not.toBeNull());
      root.querySelector<HTMLButtonElement>(".qs-row .qs-row-menu")!.click();
      expect(root.querySelector(".qs-menu")).not.toBeNull();

      // Rung one: Escape peels the child popover and leaves the modal standing.
      expect(dismissTopTransient("escape")).toBe(true);
      expect(root.querySelector(".qs-menu")).toBeNull();
      expect(root.querySelector(".query-advanced-modal")).not.toBeNull();

      // Same rung by pointer (GH #472): a press on the modal's own header is an
      // outside press for the row menu, so the menu closes and only the menu.
      root.querySelector<HTMLButtonElement>(".qs-row .qs-row-menu")!.click();
      expect(root.querySelector(".qs-menu")).not.toBeNull();
      dialog.querySelector(".query-advanced-header")!
        .dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      expect(root.querySelector(".qs-menu")).toBeNull();
      expect(root.querySelector(".query-advanced-modal")).not.toBeNull();
      await waitFor(() =>
        expect(root.querySelector<HTMLTextAreaElement>(".query-text-pane-input")?.value).toBe(route.source),
      );
      expect(lower).not.toHaveBeenCalled();

      expect(dismissTopTransient("back")).toBe(true);
      await Promise.resolve();
      expect(root.querySelector(".query-advanced-modal")).toBeNull();
      expect(document.activeElement).toBe(toggle);
      expect(lower).not.toHaveBeenCalled();
    } finally {
      unregisterLower();
      dispose();
    }
  });

  it.each(["print", "parse"])("L12:41: builder edits compose while an older %s is pending", async (phase) => {
    const route: QueryRoute = { kind: "query", id: "builder-race", sourceKind: "dsl", source: "initial", presentation: "list" };
    const initial = { anchor: "block" as const, filter: { kind: "and" as const, items: [
      { kind: "raw" as const, text: "first", diagnostic_kind: "syntax" as const },
      { kind: "raw" as const, text: "second", diagnostic_kind: "syntax" as const },
    ] }, source: { kind: "builder" as const } };
    let finishParse: ((value: never) => void) | undefined;
    vi.spyOn(backend(), "parseQuery").mockImplementation((text) => text === "initial"
      ? Promise.resolve({ query: initial, view: {} } as never)
      : new Promise((finish) => { finishParse = finish; }));
    const pending: Array<{ query: typeof initial; finish: (text: string) => void }> = [];
    vi.spyOn(backend(), "printQuery").mockImplementation((query) => new Promise<string>((finish) => {
      pending.push({ query: query as typeof initial, finish });
    }));
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock(route)} deps={workspaceDeps()} />, root);
    try {
      root.querySelector<HTMLButtonElement>(".query-advanced-toggle")!.click();
      await waitFor(() => expect(root.querySelectorAll(".qs-row")).toHaveLength(2));
      // Keep a first printer in flight, then edit the other condition.
      root.querySelector<HTMLButtonElement>('.qs-row .qs-enabled')!.click();
      if (phase === "parse") {
        pending.at(-1)!.finish("printed first");
        await waitFor(() => expect(finishParse).toBeDefined());
      }
      root.querySelectorAll<HTMLElement>(".qs-row")[1].querySelector<HTMLButtonElement>(".qs-enabled")!.click();
      const edits = pending.filter((p) => p.query.filter.items.some((f) => f.kind === "off" as string));
      const latest = edits.at(-1)!;
      expect(latest.query.filter.items.map((f) => f.kind)).toEqual(["off", "off"]);
      finishParse?.({ query: initial, view: {} } as never);
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(root.querySelectorAll(".qs-row")).toHaveLength(2);
      latest.finish("both edits");
      await Promise.resolve(); await Promise.resolve();
      for (const item of pending) if (item !== latest) item.finish("older print");
      await Promise.resolve(); await Promise.resolve();
      const modal = root.querySelector(".query-advanced-modal")!;
      modal.querySelector<HTMLButtonElement>(".query-advanced-actions .primary")!.click();
      expect(root.querySelector<HTMLInputElement>(".query-workspace-source")?.value).toBe("both edits");
    } finally { finishParse?.({ query: initial, view: {} } as never); for (const item of pending) item.finish("cleanup"); dispose(); vi.restoreAllMocks(); }
  });

  it.each(["close", "display", "switch"])("L04:37: workspace close uses its graph binding after %s", (phase) => {
    const route: QueryRoute = { kind: "query", id: "closing-search", sourceKind: "search", source: "needle", presentation: "search" };
    const deps = { ...workspaceDeps(), closeSearchWorkspace: vi.fn(async () => {}) };
    const generation = backend().graphBindingGeneration();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock(route)} deps={deps} />, root);
    if (phase === "display") bumpGraphEpoch();
    if (phase === "switch") resetStore();
    dispose();
    if (phase === "switch") expect(deps.closeSearchWorkspace).not.toHaveBeenCalled();
    else expect(deps.closeSearchWorkspace).toHaveBeenCalledWith(route.id, generation);
  });

  it.each(["source", "presentation", "display"])("keeps the workspace search alive after a reactive %s edit (I-20/I-21)", async (edit) => {
    const first: QueryRoute = { kind: "query", id: "live-search", sourceKind: "search", source: "alpha", presentation: "search" };
    const [active, setActive] = createSignal(first);
    const deps = { ...workspaceDeps(), closeSearchWorkspace: vi.fn(async () => {}) };
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={active()} router={routerMock(first)} deps={deps} />, root);
    try {
      await waitFor(() => expect(root.querySelector(".query-workspace-status")?.textContent).toContain("2 results"));
      const previousReads = vi.mocked(deps.runGraphSearch).mock.calls.length;
      setActive({ ...first, ...(edit === "source" ? { source: "beta" }
        : edit === "presentation" ? { presentation: "table" as const }
          : { blockDisplay: { sort: [["content", "desc"]] as [string, "desc"][] } }) });
      await waitFor(() => expect(vi.mocked(deps.runGraphSearch).mock.calls.length).toBeGreaterThan(previousReads));
      await waitFor(() => expect(root.querySelector(".query-workspace-status")?.textContent).toContain("2 results"));
      expect(deps.closeSearchWorkspace, "I-21: same workspace edits must not release its search lane; imitate QueryWorkspace's identity memo").not.toHaveBeenCalled();
      if (edit === "presentation") expect(root.querySelector(".query-results-table")?.textContent).toContain("Alpha notes");
      setActive({ ...active(), id: "next-search" });
      await waitFor(() => expect(deps.closeSearchWorkspace).toHaveBeenCalledTimes(1));
      expect(deps.closeSearchWorkspace).toHaveBeenLastCalledWith(first.id, backend().graphBindingGeneration());
    } finally { dispose(); }
    expect(deps.closeSearchWorkspace).toHaveBeenCalledTimes(2);
    expect(deps.closeSearchWorkspace).toHaveBeenLastCalledWith("next-search", backend().graphBindingGeneration());
  });

  it("keeps empty workspaces local, neutral, and query-free", async () => {
    const route: QueryRoute = { kind: "query", id: "query-empty", sourceKind: "search", source: "", presentation: "search" };
    const deps = workspaceDeps();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock()} deps={deps} focusSource />, root);
    await Promise.resolve(); await Promise.resolve();
    expect(root.querySelector(".query-workspace-status")?.textContent).toContain("Enter a search to begin.");
    expect(root.querySelector(".query-workspace-status")?.textContent).not.toContain("0 results");
    expect(root.querySelector(".query-workspace")?.getAttribute("data-query-route-id")).toBe("query-empty");
    expect(deps.runGraphSearch).not.toHaveBeenCalled(); expect(deps.parseQuery).not.toHaveBeenCalled(); expect(deps.queryRun).not.toHaveBeenCalled();
    dispose();
  });
  it("uses the correct empty DSL prompt and focuses on pane activation or a route transition without stealing same-route control focus", async () => {
    const emptyDsl: QueryRoute = { kind: "query", id: "query-dsl", sourceKind: "dsl", source: "", presentation: "list" };
    const root = document.createElement("div"); document.body.append(root);
    const disposeDsl = render(() => <QueryWorkspace route={emptyDsl} router={routerMock(emptyDsl)} deps={workspaceDeps()} focusSource />, root);
    await Promise.resolve(); await Promise.resolve();
    expect(root.querySelector(".query-workspace-status")?.textContent).toContain("Enter a query to begin.");
    expect(root.querySelector(".query-workspace-status")?.textContent).not.toContain("0 results");
    disposeDsl(); root.innerHTML = "";

    const first: QueryRoute = { kind: "query", id: "query-focus-a", sourceKind: "search", source: "", presentation: "search" };
    const second: QueryRoute = { ...first, id: "query-focus-b" };
    const [active, setActive] = createSignal<QueryRoute>(first);
    const [focusSource, setFocusSource] = createSignal(false);
    const router = routerMock(first);
    vi.mocked(router.route).mockImplementation(active);
    const dispose = render(() => <QueryWorkspace route={active()} router={router} deps={workspaceDeps()} focusSource={focusSource()} />, root);
    await Promise.resolve(); await Promise.resolve();
    const source = root.querySelector<HTMLInputElement>(".query-workspace-source")!;
    const filters = root.querySelector<HTMLButtonElement>(".query-advanced-toggle")!;
    filters.focus();
    expect(document.activeElement).toBe(filters);
    // A restored route may have mounted while its pane was inactive. Activating
    // that pane must focus this same route once, rather than waiting for a new
    // tab/route identity.
    setFocusSource(true);
    await Promise.resolve(); await Promise.resolve();
    expect(document.activeElement).toBe(source);
    filters.focus();
    setActive({ ...first, source: "alpha" });
    await Promise.resolve(); await Promise.resolve();
    expect(document.activeElement).toBe(filters);
    setActive({ ...first, source: "alpha", presentation: "table" });
    await Promise.resolve(); await Promise.resolve();
    expect(document.activeElement).toBe(filters);
    setActive(second);
    await Promise.resolve(); await Promise.resolve();
    expect(document.activeElement).toBe(source);
    dispose();
  });
  it("keeps invalid friendly input editable and reports the Rust diagnostic", async () => {
    const route: QueryRoute = { kind: "query", id: "query-invalid", sourceKind: "search", source: "/(a)\\1/", presentation: "search" };
    const deps = workspaceDeps();
    vi.mocked(deps.runGraphSearch).mockResolvedValue({ hits: [], diagnostics: [{ code: "invalid_regex", message: "invalid regex" }], explanation: { branches: [] }, cancelled: false });
    const root = document.createElement("div"); document.body.append(root);
    const router = routerMock(route);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);
    await waitFor(() => expect(root.querySelector(".query-workspace-diagnostics")?.textContent).toContain("invalid regex"));
    const input = root.querySelector<HTMLInputElement>(".query-workspace-source")!;
    expect(input.value).toBe("/(a)\\1/");
    input.value = "  /(a)\\1/  "; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    expect(router.updateActiveQuery).toHaveBeenLastCalledWith({ source: "  /(a)\\1/  ", sourceKind: "search" });
    dispose();
  });
  it.each(["search", "list", "table", "board"] as const)("GH #416: preserves Search with result actions in %s presentation", async (presentation) => {
    const route: QueryRoute = { kind: "query", id: "persistent", sourceKind: "search", source: "alpha", presentation };
    resetPaneLayoutToSingle({ tabs: [{ history: [route], pos: 0, pinned: false }], activeIndex: 0 });
    const router = paneRouter("main");
    const searchId = router.activeId();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={workspaceDeps()} />, root);
    try {
      await waitFor(() => expect(root.querySelectorAll("[data-inpage-find-surface]")).toHaveLength(2));
      const page = root.querySelector<HTMLButtonElement>('[data-query-result-kind="page"] button.query-result-row')!;
      const blockSurface = root.querySelector<HTMLElement>('[data-query-result-kind="block"] [data-inpage-find-surface]')!;
      const block = blockSurface instanceof HTMLButtonElement ? blockSurface : blockSurface.querySelector<HTMLButtonElement>("button")!;
      page.click();
      expect(router.route()).toMatchObject({ kind: "page", name: "Alpha notes", path: "pages/alpha.md" });
      expect(router.tabs().find((tab) => tab.id === searchId)?.history).toEqual([route]);
      router.setActiveTab(searchId);
      block.dispatchEvent(new MouseEvent("auxclick", { button: 1, bubbles: true }));
      expect(router.activeId()).toBe(searchId);
      expect(router.tabs().some((tab) => tab.history.some((r) => r.kind === "page" && r.block === "authored-block-1"))).toBe(true);
      page.dispatchEvent(new MouseEvent("click", { ctrlKey: true, bubbles: true }));
      expect(router.activeId()).toBe(searchId);
      page.dispatchEvent(new MouseEvent("click", { shiftKey: true, bubbles: true }));
      expect(rightSidebar()).toContainEqual(expect.objectContaining({ kind: "page", name: "Alpha notes", path: "pages/alpha.md" }));
      block.dispatchEvent(new MouseEvent("click", { shiftKey: true, bubbles: true }));
      expect(rightSidebar()).toContainEqual(expect.objectContaining({ kind: "block", uuid: "authored-block-1" }));
      page.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 20 }));
      expect(contextMenu()).toMatchObject({ kind: "page", name: "Alpha notes", path: "pages/alpha.md" });
      page.dispatchEvent(new MouseEvent("click", { altKey: true, bubbles: true }));
      const other = layoutPaneIds().find((id) => id !== "main")!;
      expect(paneRouter(other).route()).toMatchObject({ kind: "page", name: "Alpha notes", path: "pages/alpha.md" });
      expect(router.route()).toEqual(route);
      expect(root.querySelector<HTMLInputElement>(".query-workspace-source")!.value).toBe("alpha");
    } finally { dispose(); closeAllRightSidebarItems(); resetPaneLayoutToSingle(); }
  });

  it("shows evidence, diagnostics and explanations, and switches presentations without changing membership", async () => {
    const route: QueryRoute = {
      kind: "query",
      id: "query-test",
      sourceKind: "search",
      source: "alpha",
      presentation: "search",
    };
    const router = routerMock();
    const deps = workspaceDeps();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);

    await waitFor(() => expect(root.querySelector(".query-workspace-status")?.textContent).toContain("2 results"));
    expect(root.querySelector(".query-workspace-diagnostics")?.textContent).toContain("Results are limited");
    expect([...root.querySelectorAll("mark")].map((mark) => mark.textContent)).toEqual(["Alpha", "alpha"]);
    expect(root.querySelector(".search-result-context")?.textContent).toContain("Page");
    expect(root.querySelectorAll(".query-result-row")).toHaveLength(2);
    const resultRows = [...root.querySelectorAll<HTMLButtonElement>(".query-result-row")];
    resultRows[0].click();
    expect(router.openInNewTab).toHaveBeenCalledWith({ kind: "page",
      name: "Alpha notes", pageKind: "page", path: "pages/alpha.md",
    }, true);
    resultRows[1].click();
    expect(router.openInNewTab).toHaveBeenCalledWith({ kind: "page",
      name: "Research", pageKind: "page", path: "pages/client-b/Research.md", block: "authored-block-1",
    }, true);

    for (const [label, selector] of [
      ["List", ".query-results-list"],
      ["Table", ".query-results-table"],
      ["Board", ".query-results-board"],
    ] as const) {
      const button = [...root.querySelectorAll<HTMLButtonElement>(".query-presentations button")]
        .find((candidate) => candidate.textContent === label)!;
      button.click();
      await waitFor(() => expect(root.querySelector(selector)).not.toBeNull());
      await waitFor(() => expect([...root.querySelectorAll("mark")].map((mark) => mark.textContent)).toEqual(["Alpha", "alpha"]));
      expect(router.updateActiveQuery).toHaveBeenCalledWith({ presentation: label.toLowerCase() });
    }

    const explain = root.querySelector(".query-explain-toggle") as HTMLButtonElement;
    explain.click();
    await waitFor(() => expect(root.querySelector(".query-workspace-explanation")?.textContent).toContain("contains alpha"));
    expect(deps.runGraphSearch).toHaveBeenLastCalledWith("alpha", 40, 100, "query-workspace:query-test", true, undefined, "names", expect.any(Object));

    dispose();
  });

  function typeInto(root: HTMLElement, selector: string, value: string) {
    const input = root.querySelector<HTMLInputElement>(selector)!;
    input.value = value;
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  }
  function submitSave(root: HTMLElement) {
    (root.querySelector(".query-workspace-save") as HTMLFormElement)
      .dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));
  }

  it("does not publish or route a save whose search was retyped while the title lookup was in flight", async () => {
    const route: QueryRoute = { kind: "query", id: "query-stale", sourceKind: "search", source: "alpha", presentation: "list" };
    const router = routerMock(route);
    const deps = workspaceDeps();
    let release!: (value: ResolvedPage) => void;
    deps.resolvePage = vi.fn(() => new Promise<ResolvedPage>((resolve) => { release = resolve; }));
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);
    await waitFor(() => expect(root.querySelector(".query-workspace-status")?.textContent).toContain("2 results"));
    typeInto(root, ".query-workspace-save input", "Stale");
    submitSave(root);
    await waitFor(() => expect(deps.resolvePage).toHaveBeenCalled());
    typeInto(root, ".query-workspace-source", "beta");
    release({ kind: "absent", id: "pages/Stale.md" });
    await waitFor(() => expect(root.querySelector(".query-workspace-save-error")?.textContent).toContain("Try saving again"));
    expect(deps.savePages).not.toHaveBeenCalled();
    expect(router.replaceActiveRoute).not.toHaveBeenCalled();
    expect(root.querySelector<HTMLButtonElement>('.query-workspace-save button[type="submit"]')!.disabled).toBe(false);
    dispose();
  });

  it("keeps the workspace where it is and says so when a save committed from an earlier search", async () => {
    const route: QueryRoute = { kind: "query", id: "query-late", sourceKind: "search", source: "alpha", presentation: "list" };
    const router = routerMock(route);
    const deps = workspaceDeps();
    let finish!: (value: { ok: string[] }) => void;
    deps.savePages = vi.fn(() => new Promise<{ ok: string[] }>((resolve) => { finish = resolve; }));
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);
    await waitFor(() => expect(root.querySelector(".query-workspace-status")?.textContent).toContain("2 results"));
    typeInto(root, ".query-workspace-save input", "Committed");
    submitSave(root);
    await waitFor(() => expect(deps.savePages).toHaveBeenCalled());
    typeInto(root, ".query-workspace-source", "beta");
    finish({ ok: ["rev"] });
    await waitFor(() => expect(root.querySelector(".query-workspace-save-notice")?.textContent).toContain("Committed"));
    expect(router.replaceActiveRoute).not.toHaveBeenCalled();
    expect(root.querySelector(".query-workspace-save-error")).toBeNull();
    dispose();
  });

  it("saves by naming and replaces the virtual route only after the guarded write succeeds", async () => {
    const route: QueryRoute = {
      kind: "query",
      id: "query-save",
      sourceKind: "search",
      source: "alpha OR beta",
      presentation: "board",
    };
    const router = routerMock(route);
    const deps = workspaceDeps();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);
    await waitFor(() => expect(root.querySelector(".query-workspace-status")?.textContent).toContain("2 results"));

    const title = root.querySelector<HTMLInputElement>('.query-workspace-save input')!;
    title.value = "Saved search";
    title.dispatchEvent(new InputEvent("input", { bubbles: true }));
    (root.querySelector(".query-workspace-save") as HTMLFormElement)
      .dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));

    await waitFor(() => expect(router.replaceActiveRoute).toHaveBeenCalledWith({
      kind: "page",
      name: "Saved search",
      pageKind: "page",
    }));
    const saved = vi.mocked(deps.savePages).mock.calls[0][0][0].page;
    expect(saved.blocks).toHaveLength(1);
    expect(saved.blocks[0].raw).toBe('{{query (search "alpha OR beta")}}\ntine.view:: board');
    expect(deps.savePages).toHaveBeenCalledWith([{ id: "pages/Saved search.md", page: saved, baseRev: null, force: false, kinds: ["create-page"] }], 1);

    dispose();
  });

  it("opens a focus-managed modal and preserves an untouched OR search exactly", async () => {
    const route: QueryRoute = {
      kind: "query",
      id: "query-filters",
      sourceKind: "search",
      source: "Alpha -Draft OR Beta -Draft",
      presentation: "search",
    };
    const router = routerMock();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={workspaceDeps()} />, root);
    await waitFor(() => expect(root.querySelector(".query-workspace-status")?.textContent).toContain("2 results"));

    const toggle = root.querySelector(".query-advanced-toggle") as HTMLButtonElement;
    toggle.focus();
    toggle.click();
    await Promise.resolve();
    expect(root.querySelector('[role="dialog"]')).not.toBeNull();
    expect((document.activeElement as HTMLInputElement)?.value).toBe("");
    const fields = root.querySelectorAll<HTMLInputElement>(".query-friendly-fields input");
    expect(fields[1].value).toBe("alpha beta");
    expect(fields[3].value).toBe("draft");

    const apply = [...root.querySelectorAll<HTMLButtonElement>(".query-advanced-actions button")]
      .find((button) => button.textContent === "Apply")!;
    apply.click();
    expect(router.updateActiveQuery).toHaveBeenCalledWith({
      source: "Alpha -Draft OR Beta -Draft",
      sourceKind: "search",
    });
    await Promise.resolve();
    expect(document.activeElement).toBe(toggle);

    dispose();
  });

  it.each([
    { pattern: "(?i)abc", accepted: true },
    { pattern: "(a)\\1", accepted: false },
    { pattern: "a{1000000}", accepted: false },
  ])("OG-R1 friendly regex field uses Rust validation: $pattern", async ({ pattern, accepted }) => {
    const route: QueryRoute = { kind: "query", id: "regex-fields", sourceKind: "search", source: "", presentation: "search" };
    const router = routerMock(route);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={router} deps={workspaceDeps()} />, root);
    try {
      (root.querySelector(".query-advanced-toggle") as HTMLButtonElement).click();
      await Promise.resolve();
      const field = root.querySelectorAll<HTMLInputElement>(".query-friendly-fields input")[4];
      field.value = pattern;
      field.dispatchEvent(new InputEvent("input", { bubbles: true }));
      const apply = [...root.querySelectorAll<HTMLButtonElement>(".query-advanced-actions button")]
        .find((button) => button.textContent === "Apply")!;
      apply.click();
      if (accepted) {
        expect(router.updateActiveQuery).toHaveBeenCalledWith({ source: `/${pattern}/`, sourceKind: "search" });
      } else {
        expect(root.querySelector(".query-advanced-error")?.textContent).toBeTruthy();
        expect(router.updateActiveQuery).not.toHaveBeenCalled();
      }
    } finally { dispose(); }
  });
});
