import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import type { PaneRouter, QueryRoute } from "../router";
import type { QueryExecution } from "../types";
import { beginResultsChange } from "../queryResultsEpoch";
import { QueryWorkspace, type QueryWorkspaceDependencies } from "./QueryWorkspace";

beforeAll(initParser);
afterEach(() => { document.body.innerHTML = ""; });

const empty = (): QueryExecution => ({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false });

function routerMock(route: QueryRoute) {
  return { route: () => route, updateActiveQuery: vi.fn(), replaceActiveRoute: vi.fn(), openPage: vi.fn(), openInNewTab: vi.fn(), openPageTarget: vi.fn(), openPageAtBlock: vi.fn() } as unknown as PaneRouter;
}

const sections = (root: HTMLElement) => root.querySelector(".query-result-sections")?.textContent ?? "";
const tick = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// GH #838: a Search tab showed "Pages 0 / Blocks 0 / No matching pages." while the search had not
// run yet, because the membership resource holds its read (and is idle, not `loading`) while a result
// change is in flight, and has no value for a freshly typed identity.
describe("GH #838 Search tab says it is still searching", () => {
  function setup() {
    const route: QueryRoute = { kind: "query", id: "pending-838", sourceKind: "search", source: "", presentation: "search" };
    let finish!: (value: QueryExecution) => void;
    const runGraphSearch = vi.fn(() => new Promise<QueryExecution>((resolve) => { finish = resolve; }));
    const deps = { serializeDsl: vi.fn(), resolvePage: vi.fn(), createPage: vi.fn(), runGraphSearch,
      parseQuery: vi.fn(), queryRun: vi.fn(), queryExplainEmpty: vi.fn() } as unknown as QueryWorkspaceDependencies;
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryWorkspace route={route} router={routerMock(route)} deps={deps} />, root);
    const type = (text: string) => {
      const input = root.querySelector<HTMLInputElement>(".query-workspace-source")!;
      input.value = text; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    };
    return { root, dispose, type, runGraphSearch, finish: (value: QueryExecution) => finish(value) };
  }

  it("while a result change holds the read back, nothing says 'No matching'", async () => {
    const { root, dispose, type, runGraphSearch, finish } = setup();
    const release = beginResultsChange();
    try {
      type("Liko");
      await tick(300);
      expect(runGraphSearch).not.toHaveBeenCalled();
      expect(sections(root)).toContain("Searching…");
      expect(sections(root)).not.toContain("No matching");
      expect(root.querySelector(".query-workspace-status")?.textContent).toContain("Searching…");
      release();
      await vi.waitFor(() => expect(runGraphSearch).toHaveBeenCalled(), { timeout: 3_000, interval: 5 });
      expect(sections(root)).not.toContain("No matching");
      finish(empty());
      // A finished empty search still says so.
      await vi.waitFor(() => expect(sections(root)).toContain("No matching pages."), { timeout: 3_000, interval: 5 });
      expect(sections(root)).not.toContain("Searching…");
      expect(root.querySelector(".query-workspace-status")?.textContent).toContain("0 results");
    } finally { release(); dispose(); }
  });

  it("a retype after an answered search shows Searching, not the old or an empty answer, until the new one lands", async () => {
    const { root, dispose, type, runGraphSearch, finish } = setup();
    try {
      type("Liko");
      await vi.waitFor(() => expect(runGraphSearch).toHaveBeenCalledTimes(1), { timeout: 3_000, interval: 5 });
      finish(empty());
      await vi.waitFor(() => expect(sections(root)).toContain("No matching pages."), { timeout: 3_000, interval: 5 });
      const release = beginResultsChange();
      type("Likov");
      await tick(200);
      expect(sections(root)).toContain("Searching…");
      expect(sections(root)).not.toContain("No matching");
      release();
    } finally { dispose(); }
  });
});
