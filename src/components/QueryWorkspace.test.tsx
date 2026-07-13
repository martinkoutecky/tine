import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { PaneRouter, QueryRoute } from "../router";
import type { PageDto, QueryExecution } from "../types";
import {
  QueryWorkspace,
  materializeQueryWorkspace,
  type MaterializeQueryDependencies,
  type QueryWorkspaceDependencies,
} from "./QueryWorkspace";

afterEach(() => {
  document.body.innerHTML = "";
});

function materializeDeps(overrides: Partial<MaterializeQueryDependencies> = {}): MaterializeQueryDependencies {
  return {
    getPage: vi.fn(async () => null),
    savePage: vi.fn(async () => "rev-new"),
    ...overrides,
  };
}

describe("materializeQueryWorkspace", () => {
  it("creates one canonical friendly query block through the guarded no-baseline save", async () => {
    const deps = materializeDeps();
    const result = await materializeQueryWorkspace({
      title: "  Project dashboard  ",
      sourceKind: "search",
      source: "alpha -draft",
      presentation: "search",
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
    }, listDeps);
    expect(list.ok && list.page.blocks).toHaveLength(1);
    expect(list.ok && list.page.blocks[0].raw).toBe("{{query (and (todo TODO) (priority A))}}");

    const tableDeps = materializeDeps();
    const table = await materializeQueryWorkspace({
      title: "Task table",
      sourceKind: "dsl",
      source: "(todo TODO)",
      presentation: "table",
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
    }, deps);

    expect(result).toMatchObject({ ok: false, kind: "conflict" });
    expect(deps.savePage).toHaveBeenCalledTimes(1);
  });
});

function routerMock() {
  return {
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
    runGraphSearch: vi.fn(async (_source, _pageLimit, _blockLimit, _lane, explain) =>
      executionFixture(explain)),
    runQuery: vi.fn(async () => []),
    runAdvancedQuery: vi.fn(async () => ({ groups: [], ran: [], ignored: [], supported: true })),
  };
}

async function waitFor(check: () => void): Promise<void> {
  await vi.waitFor(check, { timeout: 1_000, interval: 5 });
}

describe("QueryWorkspace", () => {
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

    const table = [...root.querySelectorAll<HTMLButtonElement>(".query-presentations button")]
      .find((button) => button.textContent === "Table")!;
    table.click();
    expect(root.querySelector(".query-results-table")).not.toBeNull();
    expect(router.updateActiveQuery).toHaveBeenCalledWith({ presentation: "table" });

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
