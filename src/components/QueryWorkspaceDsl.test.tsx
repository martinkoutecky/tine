import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QueryWorkspace, type QueryWorkspaceDependencies } from "./QueryWorkspace";
import type { PaneRouter, QueryRoute } from "../router";
import { blockRunResult } from "../tests/queryReadingsTestkit";

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });
it.each([
  ["@block", "@block", "macro_query"],
  ["{{tine-query @block and scheduled > today}}", "@block and scheduled > today", "macro_tql"],
])("keeps the source dialect of %s in the Search workspace", async (source, argument, dialect) => {
  const parseQuery = vi.fn(async () => ({ query: { anchor: "block" as const, filter: { kind: "true" as const }, diagnostics: [], source: { kind: "og" as const, original: argument } }, view: {} }));
  const deps = { parseQuery, queryRun: async () => blockRunResult([]), queryExplainEmpty: vi.fn(),
    resolvePage: vi.fn(), createPage: vi.fn(), runGraphSearch: vi.fn(), serializeDsl: vi.fn() } as QueryWorkspaceDependencies;
  const route: QueryRoute = { kind: "query", id: "dialect-proof", sourceKind: "dsl", source, presentation: "list" };
  const router = { route: () => route } as unknown as PaneRouter;
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);
  try { await vi.waitFor(() => expect(parseQuery).toHaveBeenCalledWith(argument, dialect)); }
  finally { dispose(); }
});
