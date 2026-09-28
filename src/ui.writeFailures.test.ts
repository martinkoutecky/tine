import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { graphMeta, setGraphMeta } from "./graphSession";
import { changeWorkflow, workflow, setWorkflow, changeShowBrackets, pruneSidebarBlocks, setRightSidebar, rightSidebar, toggleWideMode, wideMode, setFavorites, favorites, toggleFavorite, persistSidebarWidth } from "./ui";
import { toasts, setToasts } from "./toasts";

const flush = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); };
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); setGraphMeta(null); setToasts([]); setRightSidebar([]); setFavorites([]); });

it("reports a failed graph preference and restores the previous value", async () => {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...loaded.meta, root: "/test", show_brackets: true });
  setWorkflow("now");
  vi.spyOn(backend(), "setPreferredWorkflow").mockRejectedValueOnce(new Error("disk full"));
  vi.spyOn(backend(), "setShowBrackets").mockRejectedValueOnce(new Error("disk full"));
  changeWorkflow("todo");
  changeShowBrackets(false);
  await flush();
  expect(workflow()).toBe("now");
  expect(graphMeta()?.show_brackets).toBe(true);
  expect(toasts().filter((toast) => toast.kind === "error")).toHaveLength(2);
});

it("keeps the persisted local display preference when storage refuses a write", () => {
  const old = wideMode();
  vi.stubGlobal("localStorage", { setItem: () => { throw new Error("quota"); }, removeItem: () => { throw new Error("quota"); } });
  toggleWideMode();
  expect(wideMode()).toBe(old);
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
  vi.unstubAllGlobals();
});

it("reports a failed sidebar width preference write", () => {
  vi.stubGlobal("localStorage", { setItem: () => { throw new Error("quota"); } });
  persistSidebarWidth();
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
});

it("keeps a sidebar block when resolution fails", async () => {
  setRightSidebar([{ kind: "block", uuid: "live", page: "A", pageKind: "page" }]);
  vi.spyOn(backend(), "resolveBlock").mockRejectedValueOnce(new Error("I/O"));
  await pruneSidebarBlocks();
  expect(rightSidebar()).toHaveLength(1);
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
});

it("restores confirmed favorites after two queued failures", async () => {
  setFavorites([]);
  vi.spyOn(backend(), "setFavorites").mockRejectedValue(new Error("disk full"));
  toggleFavorite("A");
  toggleFavorite("B");
  await flush();
  await flush();
  expect(favorites()).toEqual([]);
  expect(toasts().filter((toast) => toast.kind === "error")).toHaveLength(2);
});

it("does not restore another graph's workflow after a rejected write", async () => {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...loaded.meta, root: "/old" });
  setWorkflow("now");
  vi.spyOn(backend(), "setPreferredWorkflow").mockResolvedValueOnce();
  changeWorkflow("todo");
  await flush();
  setGraphMeta({ ...loaded.meta, root: "/new" });
  setWorkflow("now");
  vi.spyOn(backend(), "setPreferredWorkflow").mockRejectedValueOnce(new Error("disk full"));
  changeWorkflow("todo");
  await flush();
  expect(workflow()).toBe("now");
});

it("does not dispatch a queued config write into the next graph", async () => {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...loaded.meta, root: "/queued-old" });
  setWorkflow("now");
  let finish!: () => void;
  const write = vi.spyOn(backend(), "setPreferredWorkflow")
    .mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }))
    .mockResolvedValue();
  changeWorkflow("todo");
  await flush();
  changeWorkflow("now");
  setGraphMeta({ ...loaded.meta, root: "/queued-new" });
  setWorkflow("todo");
  finish();
  await flush();
  await flush();
  expect(write).toHaveBeenCalledTimes(1);
  expect(workflow()).toBe("todo");
});
