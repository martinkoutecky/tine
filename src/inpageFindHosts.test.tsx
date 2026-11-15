import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { resetStore } from "./document";
import { setDoc } from "./document/model";
import { countFindHosts, createPendingFindHosts } from "./inpageFindHosts";
import { initParser } from "./render/parse";
import * as facets from "./render/facets";

beforeAll(initParser);
afterEach(() => { resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });

// PC-3/PC-8: match bench's one warmup + minimum of eight runs, while keeping
// the existing 500 ms interaction ceiling (selectionColdCost, MarginTiers).
// Fixture construction and DOM rendering are outside these inventory budgets.
it.each(["opening", "mounting"] as const)("pending-host timing budget: %s on a 10k-block outline", operation => {
  const roots = Array.from({ length: 10_000 }, (_, i) => `budget-${i}`);
  setDoc({ byId: Object.fromEntries(roots.map((id, i) => [id, {
    id, raw: i < 200 ? `{{query (task TODO)}} result ${i}` : `ordinary outline block ${i}`,
    children: [], parent: null, page: "Budget", collapsed: false,
  }])) });
  const scope = document.createElement("div");
  const hosts = roots.slice(0, 200).map(id => {
    const host = document.createElement("div");
    host.dataset.findHost = id;
    host.dataset.findAnswered = "true";
    return host;
  });
  scope.append(...hosts);
  document.body.append(scope);
  const samples: number[] = [];
  for (let run = 0; run < 9; run++) createRoot(dispose => {
    try {
      // 2,000 rendered query rows, delivered as individual child-list records
      // in one observer batch, including nested result text and pending cells.
      const records = operation === "opening" ? [] : Array.from({ length: 2_000 }, (_, i) => {
        const row = document.createElement("div");
        const cell = document.createElement("span");
        cell.className = "sheet-cell-defer";
        cell.textContent = `synthetic query result ${i}`;
        row.append(cell);
        const host = hosts[i % hosts.length];
        return { target: host, addedNodes: [row], removedNodes: [] } as unknown as MutationRecord;
      });
      let started = performance.now();
      const inventory = createPendingFindHosts(() => [{ roots, format: "md", element: scope }]);
      const firstCount = inventory.count();
      let elapsed = performance.now() - started;
      expect(firstCount).toBe(0);
      if (operation === "mounting") {
        for (const record of records) record.target.appendChild(record.addedNodes[0]);
        started = performance.now();
        inventory.refresh(records);
        const pending = inventory.count();
        elapsed = performance.now() - started;
        expect(pending).toBe(200);
      }
      if (run) samples.push(elapsed);
      for (const host of hosts) host.replaceChildren();
    } finally { dispose(); }
  });
  const minimum = Math.min(...samples);
  console.info(`pending-host ${operation}: min=${minimum.toFixed(2)} ms samples=${samples.map(ms => ms.toFixed(2)).join(",")} budget=500 ms`);
  expect(minimum, `${operation}: 10k blocks, 200 hosts, 2k result rows`).toBeLessThan(500);
});

it.each(["md", "org"] as const)("counts parser hosts, including inline queries, without counting literal macros (%s)", format => {
  expect(countFindHosts("{{embed [[Source]]}}", format)).toBe(1);
  expect(countFindHosts("inline {{EMBED [[Source]]}}", format)).toBe(1);
  expect(countFindHosts('before {{query (task TODO)}} and {{tine-query from blocks where text match "word"}}', format)).toBe(2);
  expect(countFindHosts('{{query (task TODO) {:title "}} {{embed [[Literal]]}}"}}}', format)).toBe(1);
  expect(countFindHosts("`{{embed [[Literal]]}}`", format)).toBe(0);
  expect(countFindHosts(format === "md" ? "```\n{{query (task TODO)}}\n```" : "#+BEGIN_SRC\n{{query (task TODO)}}\n#+END_SRC", format)).toBe(0);
  expect(countFindHosts("ordinary content", format)).toBe(0);
  // Review HOLD on f471f00d0: a whole-block BEGIN_QUERY container renders as a query host too.
  expect(countFindHosts("#+BEGIN_QUERY\n{:query (task TODO) :table-view? false}\n#+END_QUERY", format)).toBe(1);
});

it("patches mount deltas and serves repeated counts without rewalking or parsing the outline", () => {
  setDoc({ byId: {
    embed: { id: "embed", raw: "{{embed [[Source]]}}", children: [], parent: null, page: "Host", collapsed: false },
    query: { id: "query", raw: "{{query (task TODO)}}", children: [], parent: null, page: "Host", collapsed: false },
  } });
  const parse = vi.spyOn(facets, "parseBody");
  const scope = document.createElement("div");
  document.body.append(scope);
  createRoot(dispose => {
    try {
      const inventory = createPendingFindHosts(() => [{ roots: ["embed", "query"], format: "md", element: scope }]);
      expect(inventory.count()).toBe(2);
      expect(parse).toHaveBeenCalledTimes(2);
      parse.mockClear();
      for (let i = 0; i < 1000; i++) expect(inventory.count()).toBe(2);
      expect(parse).not.toHaveBeenCalled();
      const host = document.createElement("div");
      host.dataset.findHost = "embed"; host.dataset.findAnswered = "true";
      scope.append(host);
      const addition = { target: scope, addedNodes: [host], removedNodes: [] } as unknown as MutationRecord;
      inventory.refresh([addition]);
      expect(inventory.count()).toBe(1);
      inventory.refresh([addition]);
      expect(inventory.count()).toBe(1);
      const walk = vi.spyOn(host, "querySelectorAll");
      host.dataset.findAnswered = "false";
      inventory.refresh([{ target: host, addedNodes: [], removedNodes: [] } as unknown as MutationRecord]);
      expect(inventory.count()).toBe(2);
      host.dataset.findAnswered = "true";
      inventory.refresh([{ target: host, addedNodes: [], removedNodes: [] } as unknown as MutationRecord]);
      expect(inventory.count()).toBe(1);
      const deferred = document.createElement("span");
      deferred.className = "sheet-cell-defer";
      host.append(deferred);
      inventory.refresh([{ target: host, addedNodes: [deferred], removedNodes: [] } as unknown as MutationRecord]);
      expect(inventory.count()).toBe(2);
      deferred.remove();
      inventory.refresh([{ target: host, addedNodes: [], removedNodes: [deferred] } as unknown as MutationRecord]);
      expect(inventory.count()).toBe(1);
      expect(walk).not.toHaveBeenCalled();
      setDoc("byId", "embed", "raw", "ordinary content");
      expect(inventory.count()).toBe(1);
      host.remove();
      inventory.refresh([{ target: scope, addedNodes: [], removedNodes: [host] } as unknown as MutationRecord]);
      expect(inventory.count()).toBe(1);
      expect(parse).not.toHaveBeenCalled();
      setDoc("byId", "embed", "raw", "{{embed [[Source]]}}");
      expect(inventory.count()).toBe(2);
      expect(parse).toHaveBeenCalledTimes(1);
      parse.mockClear();
      setDoc("byId", "query", "raw", "ordinary content");
      expect(inventory.count()).toBe(1);
      setDoc("byId", "query", "raw", "{{query (task TODO)}} {{embed [[Other]]}}");
      expect(inventory.count()).toBe(3);
      expect(parse).toHaveBeenCalledTimes(1);
    } finally { dispose(); }
  });
});

// Review HOLD on d7244ed23: journal days are separate scopes sharing one pane element; discovery walked it once per day.
it("walks a pane shared by many journal-day scopes once when the inventory builds", () => {
  const days = Array.from({ length: 32 }, (_, i) => `day-${i}`);
  setDoc({ byId: Object.fromEntries(days.map(id => [id, { id, raw: "{{embed [[Source]]}}", children: [], parent: null, page: id, collapsed: false }])) });
  const pane = document.createElement("div");
  document.body.append(pane);
  const walk = vi.spyOn(pane, "querySelectorAll");
  createRoot(dispose => {
    try {
      const inventory = createPendingFindHosts(() => days.map(id => ({ roots: [id], format: "md" as const, element: pane })));
      expect(inventory.count()).toBe(32);
      expect(walk).toHaveBeenCalledTimes(1);
    } finally { dispose(); }
  });
});

// Review HOLD on f43757ae2: a query table renders 200 rows and a load-more control; the rest is not searched yet.
it("counts a host whose table or board still offers more rows as unsearched", () => {
  setDoc({ byId: { query: { id: "query", raw: "{{query (task TODO)}}", children: [], parent: null, page: "Host", collapsed: false } } });
  const scope = document.createElement("div");
  document.body.append(scope);
  createRoot(dispose => {
    try {
      const inventory = createPendingFindHosts(() => [{ roots: ["query"], format: "md", element: scope }]);
      const host = document.createElement("div");
      host.dataset.findHost = "query"; host.dataset.findAnswered = "true";
      scope.append(host);
      inventory.refresh([{ target: scope, addedNodes: [host], removedNodes: [] } as unknown as MutationRecord]);
      expect(inventory.count()).toBe(0);
      const more = document.createElement("button");
      more.className = "sheet-add-row-ghost sheet-load-more";
      host.append(more);
      inventory.refresh([{ target: host, addedNodes: [more], removedNodes: [] } as unknown as MutationRecord]);
      expect(inventory.count()).toBe(1);
      more.remove();
      inventory.refresh([{ target: host, addedNodes: [], removedNodes: [more] } as unknown as MutationRecord]);
      expect(inventory.count()).toBe(0);
    } finally { dispose(); }
  });
});
