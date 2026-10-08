import { afterEach, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn(async () => ({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false })) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); invoke.mockClear(); });

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
