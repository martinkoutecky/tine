import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { resetStore } from "./document";
import { setDoc } from "./document/model";
import { countFindHosts, createPendingFindHosts } from "./inpageFindHosts";
import { initParser } from "./render/parse";
import * as facets from "./render/facets";

beforeAll(initParser);
afterEach(() => { resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });

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
