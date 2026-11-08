import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { bumpDataRev, bumpGraphEpoch } from "../graphSession";
import { applyChangeSummary } from "../changeSummary";
import { QueryWorkspace, type QueryWorkspaceDependencies } from "./QueryWorkspace";
import type { PaneRouter, QueryRoute } from "../router";
import type { QueryExecution } from "../types";
import type { QuerySubscription } from "../querySubscription";
import { beginResultsChange } from "../queryResultsEpoch";

beforeAll(initParser);
afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers(); document.body.innerHTML = ""; });

it.each(["search", "list", "table", "board"] as const)("GH #907: external write streams retain %s sections, unchanged rows and lower-result navigation while answers stay live", async (presentation) => {
  vi.useFakeTimers();
  bumpGraphEpoch();
  let revision = 0;
  const publish = (queryTo: string[]) => {
    const finish = beginResultsChange();
    applyChangeSummary({ rev: String(++revision), pages: ["writer"], blocks: [], linksTo: [],
      props: {}, inventoryChanged: false, full: false, queryTo });
    bumpDataRev(true);
    finish();
  };
  publish([]);
  const original = backend().registerQuerySubscription;
  const register = vi.fn(async (_id: string, _request: QuerySubscription | null) => ({ mode: "narrow" as const, rev: "1" }));
  backend().registerQuerySubscription = register;
  let answer: QueryExecution = { hits: Array.from({ length: 60 }, (_, i) => ({
    entity: "block" as const, page: "Needle results", kind: "page" as const, path: "pages/Needle results.md",
    block: { id: `block-${i}`, raw: `needle result ${i}`, children: [], collapsed: false,
      properties: i % 2 === 0 ? [["id", `authored-${i}`] as [string, string]] : [] },
    display_text: `needle result ${i}`, evidence: [],
  })), diagnostics: [], explanation: { branches: [] }, cancelled: false };
  for (const path of ["pages/Needle.md", "pages/twin/Needle.md"]) answer.hits.push({
    entity: "page", page: { name: "Needle", kind: "page", path, date_key: null },
    display_text: "Needle", evidence: [], score: 0,
  });
  let complete: (() => void) | undefined;
  const search = vi.fn(async () => {
    const copy = structuredClone(answer);
    if (search.mock.calls.length > 1) await new Promise<void>(resolve => { complete = resolve; });
    return copy;
  });
  const route: QueryRoute = { kind: "query", id: "churn", sourceKind: "search", source: "needle", presentation };
  const router = { updateActiveQuery: vi.fn(), openPageAtBlockInNewTab: vi.fn(), openInNewTab: vi.fn() } as unknown as PaneRouter;
  const deps = { runGraphSearch: search } as unknown as QueryWorkspaceDependencies;
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryWorkspace route={route} router={router} deps={deps} />, root);
  try {
    await vi.advanceTimersByTimeAsync(0);
    expect(root.querySelectorAll("[data-query-result]")).toHaveLength(62);
    const id = register.mock.calls[0][0];
    const section = root.querySelector('[data-query-result-kind="block"]')!;
    const pageSection = root.querySelector('[data-query-result-kind="page"]')!;
    const rows = [...root.querySelectorAll<HTMLButtonElement>("[data-query-result]")];
    const lower = rows.find(row => row.dataset.queryResult!.includes('"block-55"'))!;
    let removals = 0;
    const observer = new MutationObserver(records => { for (const record of records) {
      for (const removed of record.removedNodes) if (rows.some(row => removed === row || removed.contains(row))) removals++;
    } });
    observer.observe(root, { childList: true, subtree: true });
    root.scrollTop = 900;
    for (let i = 0; i < 8; i++) { publish([]); await vi.advanceTimersByTimeAsync(200); }
    expect(search).toHaveBeenCalledTimes(1);
    expect(rows.every(row => row.isConnected)).toBe(true);
    // Change a matching result, add another, and stream related invalidations
    // while the native read is in flight. Old rows must remain usable throughout.
    answer.hits[0].display_text = "needle changed result";
    for (let i = 0; i < 60; i++) {
      const hit = answer.hits[i];
      if (hit.entity === "block") hit.block.id = `renewed-${i}`;
    }
    const pageHit = answer.hits[60];
    if (pageHit.entity !== "page") throw new Error("page fixture");
    pageHit.page.name = "Needle changed page";
    pageHit.display_text = pageHit.page.name;
    answer.hits.push({ ...structuredClone(answer.hits[1]), block: { ...(answer.hits[1] as Extract<typeof answer.hits[number], {entity: "block"}>).block, id: "new-block" } } as typeof answer.hits[number]);
    publish([id]); await vi.advanceTimersByTimeAsync(200);
    expect(search).toHaveBeenCalledTimes(2);
    expect(lower.isConnected).toBe(true);
    for (let i = 0; i < 5; i++) publish([id]);
    await vi.advanceTimersByTimeAsync(0);
    expect(search).toHaveBeenCalledTimes(2);
    complete?.(); await vi.advanceTimersByTimeAsync(0);
    expect(lower.isConnected).toBe(true);
    await vi.advanceTimersByTimeAsync(200);
    expect(search).toHaveBeenCalledTimes(3);
    complete?.(); await vi.advanceTimersByTimeAsync(0);
    expect(root.querySelector('[data-query-result-kind="block"]')).toBe(section);
    expect(root.querySelector('[data-query-result-kind="page"]')).toBe(pageSection);
    expect(root.textContent).toContain("needle changed result");
    expect(root.textContent).toContain("Needle changed page");
    expect(root.querySelectorAll("[data-query-result]")).toHaveLength(63);
    expect(rows.every(row => row.isConnected)).toBe(true);
    expect(removals).toBe(0);
    expect(root.scrollTop).toBe(900);
    lower.click();
    expect(router.openInNewTab).toHaveBeenCalledWith(expect.objectContaining({ name: "Needle results", block: "renewed-55", path: "pages/Needle results.md" }), true);
    observer.disconnect();
  } finally { complete?.(); dispose(); backend().registerQuerySubscription = original; }
});
