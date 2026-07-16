import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import type { PaneRouter, QueryRoute } from "../router";
import type { PageDto, QueryExecution } from "../types";
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

afterEach(() => {
  clearTransientLayersForTest();
  document.body.innerHTML = "";
});

function materializeDeps(overrides: Partial<MaterializeQueryDependencies> = {}): MaterializeQueryDependencies {
  return {
    getPage: vi.fn(async () => null),
    savePage: vi.fn(async () => "rev-new"),
    runGraphSearch: vi.fn(async () => ({ hits: [], diagnostics: [], explanation: { branches: [{ description: "valid", children: [] }] }, cancelled: false })),
    ...overrides,
  };
}

describe("materializeQueryWorkspace", () => {
  it("rejects empty, exclusion-only, and Rust-diagnostic friendly searches before any graph write", async () => {
    for (const source of ["   ", "-draft", "/(a)\\1/"]) {
      const deps = materializeDeps({ runGraphSearch: vi.fn(async () => source === "-draft"
        ? { hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false }
        : { hits: [], diagnostics: [{ code: "invalid_regex", message: "invalid regex" }], explanation: { branches: [] }, cancelled: false }) });
      const result = await materializeQueryWorkspace({ title: "Unsafe", sourceKind: "search", source, presentation: "search", routeId: "query-unsafe" }, deps);
      expect(result.ok).toBe(false);
      expect(deps.getPage).not.toHaveBeenCalled();
      expect(deps.savePage).not.toHaveBeenCalled();
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
    expect(blank.getPage).not.toHaveBeenCalled();
    expect(blank.savePage).not.toHaveBeenCalled();
  });
  it("rejects JavaScript-invalid, cancelled, and failed Rust validation before page lookup", async () => {
    const input = { title: "Unsafe", sourceKind: "search" as const, source: "/(unclosed/", presentation: "search" as const, routeId: "query-rejected" };
    const diagnostic = materializeDeps({ runGraphSearch: vi.fn(async () => ({ hits: [], diagnostics: [{ code: "invalid_regex", message: "invalid regex" }], explanation: { branches: [] }, cancelled: false })) });
    await materializeQueryWorkspace(input, diagnostic);
    expect(diagnostic.runGraphSearch).toHaveBeenCalledWith("/(unclosed/", 0, 0, "query-workspace:query-rejected:materialize", true);
    expect(diagnostic.getPage).not.toHaveBeenCalled(); expect(diagnostic.savePage).not.toHaveBeenCalled();
    const cancelled = materializeDeps({ runGraphSearch: vi.fn(async () => ({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: true })) });
    await materializeQueryWorkspace({ ...input, source: "alpha" }, cancelled);
    expect(cancelled.getPage).not.toHaveBeenCalled(); expect(cancelled.savePage).not.toHaveBeenCalled();
    const failed = materializeDeps({ runGraphSearch: vi.fn(async () => { throw new Error("IPC unavailable"); }) });
    await materializeQueryWorkspace({ ...input, source: "alpha" }, failed);
    expect(failed.getPage).not.toHaveBeenCalled(); expect(failed.savePage).not.toHaveBeenCalled();
  });
  it("creates one canonical friendly query block through the guarded no-baseline save", async () => {
    const deps = materializeDeps();
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
      blocks: [{
        id: "",
        raw: '{{query (search "alpha -draft")}}\ntine.view:: search',
        collapsed: false,
        children: [],
      }],
    });
    expect(deps.getPage).toHaveBeenCalledWith("Project dashboard", "page");
    expect(deps.savePage).toHaveBeenCalledTimes(1);
    expect(deps.savePage).toHaveBeenCalledWith(result.page, null, false);
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
    const existing: PageDto = {
      name: "Taken",
      kind: "page",
      title: "Taken",
      pre_block: null,
      blocks: [],
    };
    const deps = materializeDeps({ getPage: vi.fn(async () => existing) });
    const result = await materializeQueryWorkspace({
      title: "Taken",
      sourceKind: "search",
      source: "alpha",
      presentation: "list",
      routeId: "query-taken",
    }, deps);

    expect(result).toMatchObject({ ok: false, kind: "exists" });
    expect(deps.savePage).not.toHaveBeenCalled();
  });

  it("keeps the workspace virtual when a create race reaches the save guard", async () => {
    const deps = materializeDeps({
      savePage: vi.fn(async () => { throw new Error("save conflict: page changed on disk"); }),
    });
    const result = await materializeQueryWorkspace({
      title: "Raced",
      sourceKind: "search",
      source: "alpha",
      presentation: "board",
      routeId: "query-raced",
    }, deps);

    expect(result).toMatchObject({ ok: false, kind: "conflict" });
    expect(deps.savePage).toHaveBeenCalledTimes(1);
  });
});

function routerMock(activeRoute: QueryRoute = { kind: "query", id: "query-mock", sourceKind: "search", source: "", presentation: "search" }) {
  return {
    route: vi.fn(() => activeRoute),
    updateActiveQuery: vi.fn(),
    replaceActiveRoute: vi.fn(),
    openPage: vi.fn(),
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
        block: {
          id: "block-1",
          raw: "An alpha result",
          collapsed: false,
          children: [],
          breadcrumb: ["Parent"],
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
    getPage: vi.fn(async () => null),
    savePage: vi.fn(async () => "saved-rev"),
    runGraphSearch: vi.fn(async (_source, pageLimit, blockLimit, _lane, explain) =>
      pageLimit === 0 && blockLimit === 0
        ? { hits: [], diagnostics: [], explanation: { branches: [{ description: "valid", children: [] }] }, cancelled: false }
        : executionFixture(explain)),
    runQuery: vi.fn(async () => []),
    runAdvancedQuery: vi.fn(async () => ({ groups: [], ran: [], ignored: [], supported: true })),
  };
}

async function waitFor(check: () => void): Promise<void> {
  await vi.waitFor(check, { timeout: 1_000, interval: 5 });
}

describe("QueryWorkspace", () => {
  it("peels a QueryBuilder child before its Advanced parent and preserves the draft", async () => {
    const route: QueryRoute = {
      kind: "query",
      id: "query-transient-ladder",
      sourceKind: "dsl",
      source: "(and (task TODO))",
      presentation: "list",
    };
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

      root.querySelector<HTMLButtonElement>(".qb-chip")!.click();
      expect(root.querySelector(".qb-menu")).not.toBeNull();
      dialog.querySelector(".query-advanced-header")!
        .dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));

      expect(dismissTopTransient("escape")).toBe(true);
      expect(root.querySelector(".qb-menu")).toBeNull();
      expect(root.querySelector(".query-advanced-modal")).not.toBeNull();
      expect(root.querySelector<HTMLTextAreaElement>(".query-dsl-editor textarea")?.value).toBe(route.source);
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

  it("keeps empty workspaces local, neutral, and query-free", async () => {
    const route: QueryRoute = { kind: "query", id: "query-empty", sourceKind: "search", source: "", presentation: "search" };
    const deps = workspaceDeps();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock()} deps={deps} focusSource />, root);
    await Promise.resolve(); await Promise.resolve();
    expect(root.querySelector(".query-workspace-status")?.textContent).toContain("Enter a search to begin.");
    expect(root.querySelector(".query-workspace-status")?.textContent).not.toContain("0 results");
    expect(root.querySelector(".query-workspace")?.getAttribute("data-query-route-id")).toBe("query-empty");
    expect(deps.runGraphSearch).not.toHaveBeenCalled(); expect(deps.runQuery).not.toHaveBeenCalled(); expect(deps.runAdvancedQuery).not.toHaveBeenCalled();
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

    for (const [label, selector] of [
      ["List", ".query-results-list"],
      ["Table", ".query-results-table"],
      ["Board", ".query-results-board"],
    ] as const) {
      const button = [...root.querySelectorAll<HTMLButtonElement>(".query-presentations button")]
        .find((candidate) => candidate.textContent === label)!;
      button.click();
      expect(root.querySelector(selector)).not.toBeNull();
      expect([...root.querySelectorAll("mark")].map((mark) => mark.textContent)).toEqual(["Alpha", "alpha"]);
      expect(router.updateActiveQuery).toHaveBeenCalledWith({ presentation: label.toLowerCase() });
    }

    const explain = root.querySelector(".query-explain-toggle") as HTMLButtonElement;
    explain.click();
    await waitFor(() => expect(root.querySelector(".query-workspace-explanation")?.textContent).toContain("contains alpha"));
    expect(deps.runGraphSearch).toHaveBeenLastCalledWith("alpha", 40, 100, "query-workspace:query-test", true);

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
    const router = routerMock();
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
    const saved = vi.mocked(deps.savePage).mock.calls[0][0];
    expect(saved.blocks).toHaveLength(1);
    expect(saved.blocks[0].raw).toBe('{{query (search "alpha OR beta")}}\ntine.view:: board');
    expect(deps.savePage).toHaveBeenCalledWith(saved, null, false);

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
});
