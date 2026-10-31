// Report-only synthetic retention probe; opt in explicitly, outside the gate.
import { expect, it, vi } from "vitest";
import { backend } from "./backend";
import { bumpGraphEpoch } from "./graphSession";
import { applyGraphAnswers } from "./graphAnswers";
import { getTemplates } from "./editor/completionItems";
import type { TemplateDto } from "./types";

vi.mock("./warmCache", () => ({ waitForWarmCache: () => new Promise(() => {}) }));
vi.mock("./editor/templateVars", async importOriginal => ({
  ...await importOriginal<typeof import("./editor/templateVars")>(), prepareTemplateVars: async () => {},
}));

it.skipIf(!process.env.PERF_RESIDUE_RETENTION)("reports template DTO and reference-count retention", async () => {
  await import("./blockRefCounts");
  for (const n of [100, 1000]) {
    bumpGraphEpoch();
    const templates: TemplateDto[] = Array.from({ length: n }, (_, i) => ({
      name: `Template ${i}`, page: "Templates", kind: "page",
      blocks: Array.from({ length: 10 }, (_, j) => ({
        id: `t-${i}-${j}`, raw: `Template body ${i}:${j}`, collapsed: false, children: [],
      })),
    }));
    const listing = vi.spyOn(backend(), "listTemplates").mockResolvedValue(templates);
    const first = await getTemplates();
    expect(first).toBe(templates);
    expect(await getTemplates()).toBe(first);
    expect(listing).toHaveBeenCalledTimes(1);
    listing.mockRestore();

    const maps = new Set<Map<string, unknown>>();
    const original = Map.prototype.set;
    const spy = vi.spyOn(Map.prototype, "set").mockImplementation(function (this: Map<string, unknown>, key, value) {
      if (typeof key === "string" && key.startsWith("perf-count-")) maps.add(this);
      return original.call(this, key, value);
    });
    const values = Object.fromEntries(Array.from({ length: n }, (_, i) => [`perf-count-${i}`, 1]));
    applyGraphAnswers({ rev: "10000", inventoryChanged: false, blockRefCounts: values });
    applyGraphAnswers({ rev: "10001", inventoryChanged: false,
      blockRefCounts: Object.fromEntries(Object.keys(values).map(key => [key, 0])) });
    spy.mockRestore();
    const retained = [...maps].map(map => [...map].filter(([key]) => key.startsWith("perf-count-")).length);
    expect(retained).toEqual([n, n]); // counts plus per-identity revision updates
    console.log(JSON.stringify({ n, templateBlocks: n * 10, templateJsonBytes: JSON.stringify(first).length,
      retainedCountAndRevisionRowsAfterZero: retained }));
  }
});
