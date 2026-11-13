import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { initNavSettings, setShowWorkspaces, showWorkspaces } from "./navSettings";

afterEach(() => vi.restoreAllMocks());

it("loads and toggles Workspaces with bounded frontend time and one IPC each", async () => {
  // Native disk/IPC latency is measured by scripts/bench-perf-proofs.mjs.
  // This isolates frontend overhead, including preference ownership and queueing.
  const read = vi.spyOn(backend(), "getAppBool").mockImplementation(async (_key, fallback) => fallback);
  let written!: () => void;
  const write = vi.spyOn(backend(), "setAppBool").mockImplementation(async () => written());
  const loads: number[] = [], toggles: number[] = [];
  for (let i = 0; i < 17; i++) {
    read.mockClear(); write.mockClear();
    let at = performance.now();
    await initNavSettings();
    const load = performance.now() - at;
    expect(read.mock.calls.filter(([key]) => key === "show_workspaces")).toEqual([["show_workspaces", true]]);
    const on = !showWorkspaces();
    const persisted = new Promise<void>(resolve => { written = resolve; });
    at = performance.now();
    setShowWorkspaces(on);
    await persisted;
    const toggle = performance.now() - at;
    expect(showWorkspaces()).toBe(on);
    expect(write).toHaveBeenCalledExactlyOnceWith("show_workspaces", on);
    if (i > 0) { loads.push(load); toggles.push(toggle); }
  }
  // Initial loaded-host medians: 0.021 ms load / 0.006 ms toggle. These
  // frontend-only bounds leave 7–8x for slower CPUs; native bounds live below.
  for (const [name, samples, budget] of [["preference load", loads, 0.15], ["preference toggle", toggles, 0.05]] as const) {
    samples.sort((a, b) => a - b);
    console.log(`${name}: median_ms=${samples[8]} samples_ms=${JSON.stringify(samples)}`);
    // A median, as in sorted_search_budget, excludes isolated scheduler stalls.
    expect(samples[8], name).toBeLessThan(budget);
  }
});
