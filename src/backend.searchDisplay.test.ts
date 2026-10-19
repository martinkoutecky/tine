import { afterEach, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn(async (_cmd?: string, _args?: unknown) => ({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false })) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.resetModules(); invoke.mockClear(); });

it("sends independent friendly Display views through the native search command", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { backend } = await import("./backend");
  const page = { sort: [["name", "desc"]] as [string, "desc"][], group_by: "owner", sample: 2 };
  const block = { sort: [["priority", "asc"]] as [string, "asc"][], sample: 1 };
  await backend().runGraphSearch("alpha", 40, 100, "display", false, undefined, "both", { page, block });
  expect(invoke).toHaveBeenCalledWith("run_graph_search", expect.objectContaining({
    source: "alpha", pageLimit: 40, blockLimit: 100, pageMatchScope: "both",
    pageView: page, blockView: block,
  }));
});

// A keystroke's older search can reach the native lane after the newer one and
// cancel it, leaving "Search superseded by a newer request" with no results.
// Each call carries its issue order so the lane can refuse the overtaken one.
it("stamps lane searches with a strictly increasing issue order, across reloads", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { backend } = await import("./backend");
  await backend().runGraphSearch("Keyboar", 40, 100, "query-workspace:a");
  await backend().runGraphSearch("Keyboard", 40, 100, "query-workspace:a");
  await backend().runGraphSearch("Keyboard", 40, 100, "query-workspace:b");
  const seqs = invoke.mock.calls.filter(([cmd]) => cmd === "run_graph_search")
    .map(([, args]) => (args as { laneSeq: number }).laneSeq);
  expect(seqs).toHaveLength(3);
  expect(seqs[1]).toBeGreaterThan(seqs[0]);
  expect(seqs[2]).toBeGreaterThan(seqs[1]);
  // A fresh WebView (module reload) starts above everything the first issued.
  vi.spyOn(Date, "now").mockReturnValue(Date.now() + 5000);
  vi.resetModules();
  const second = await import("./backend");
  await second.backend().runGraphSearch("Keyboard", 40, 100, "query-workspace:a");
  const last = invoke.mock.calls.filter(([cmd]) => cmd === "run_graph_search").at(-1)![1] as { laneSeq: number };
  expect(last.laneSeq).toBeGreaterThan(seqs[2]);
});

it("stamps the ((-picker block search with the same issue order", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  invoke.mockResolvedValueOnce([] as never).mockResolvedValueOnce([] as never);
  const { backend } = await import("./backend");
  await backend().search("ab", 20, "block-picker");
  await backend().search("abc", 20, "block-picker");
  const seqs = invoke.mock.calls.filter(([cmd]) => cmd === "search").map(([, args]) => (args as { laneSeq: number }).laneSeq);
  expect(seqs).toHaveLength(2);
  expect(seqs[1]).toBeGreaterThan(seqs[0]);
});
