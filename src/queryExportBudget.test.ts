import { afterEach, expect, it, vi } from "vitest";
vi.mock("./toasts", () => ({ pushToast: vi.fn() }));
afterEach(() => { vi.restoreAllMocks(); vi.resetModules(); });
const tick = () => new Promise((r) => setTimeout(r, 0));
it("delegates the absent override to Rust and remembers a device limit", async () => {
  const { backend } = await import("./backend");
  vi.spyOn(backend(), "getAppString").mockResolvedValue("");
  const set = vi.spyOn(backend(), "setAppString").mockResolvedValue(undefined);
  const budget = await import("./queryExportBudget");
  await budget.initQueryExportBudget(); expect(budget.queryExportBudgetBytes()).toBeUndefined();
  budget.changeQueryExportBudgetMiB(2048); await tick();
  expect(budget.queryExportBudgetBytes()).toBe(2048 * 1024 * 1024);
  expect(set).toHaveBeenCalledWith("query_export_asset_budget_mib", "2048");
  budget.changeQueryExportBudgetMiB(null); await tick(); expect(budget.queryExportBudgetBytes()).toBeUndefined();
});
it("keeps the user's newer choice when startup finishes late and rolls back a refused save", async () => {
  let resolve!: (n: string) => void;
  const { backend } = await import("./backend");
  vi.spyOn(backend(), "getAppString").mockReturnValue(new Promise((r) => { resolve = r; }));
  const set = vi.spyOn(backend(), "setAppString").mockResolvedValue(undefined);
  const budget = await import("./queryExportBudget");
  const load = budget.initQueryExportBudget(); budget.changeQueryExportBudgetMiB(2); await tick();
  resolve("9"); await load; expect(budget.queryExportBudgetMiB()).toBe(2);
  set.mockRejectedValueOnce(new Error("disk")); budget.changeQueryExportBudgetMiB(4); await tick();
  expect(budget.queryExportBudgetMiB()).toBe(2);
  const { pushToast } = await import("./toasts"); expect(pushToast).toHaveBeenCalledWith("Could not save query export size limit.", "error");
});
