import { beforeEach, expect, it, vi } from "vitest";
import { saveExportComments, stageExportComments } from "./exportComments";
import { flushPage, setBlockProperty } from "./document";

const state = vi.hoisted(() => ({ value: null as string | null, writable: true, dirty: false }));
vi.mock("./document", () => ({
  blockProperty: () => state.value,
  blockWritable: () => state.writable,
  isDirty: () => state.dirty,
  node: () => ({ page: "Definitions", raw: "{{query (task TODO)}}" }),
  setBlockProperty: vi.fn((_id: string, _key: string, value: string | null) => { state.value = value; state.dirty = true; }),
  flushPage: vi.fn(async () => { state.dirty = false; return true; }),
}));
beforeEach(() => { vi.clearAllMocks(); state.value = null; state.writable = true; state.dirty = false; });

it("changes only the definition through its guarded page save, with no unchanged-choice write", async () => {
  await saveExportComments("definition", true, () => true);
  expect(setBlockProperty).not.toHaveBeenCalled();
  await saveExportComments("definition", false, () => true);
  expect(setBlockProperty).toHaveBeenLastCalledWith("definition", "tine.comments", "false");
  expect(flushPage).toHaveBeenCalledExactlyOnceWith("Definitions");
  await saveExportComments("definition", false, () => true);
  expect(flushPage).toHaveBeenCalledTimes(1);
  await saveExportComments("definition", true, () => true);
  expect(setBlockProperty).toHaveBeenLastCalledWith("definition", "tine.comments", null);
  expect(flushPage).toHaveBeenCalledTimes(2);
});

it("does no write after a graph switch and reports a save failure", async () => {
  await saveExportComments("definition", false, () => false);
  expect(setBlockProperty).not.toHaveBeenCalled();
  vi.mocked(flushPage).mockResolvedValueOnce(false);
  await expect(saveExportComments("definition", false, () => true)).rejects.toThrow("Could not save Definitions");
  await saveExportComments("definition", false, () => true);
  expect(flushPage).toHaveBeenCalledTimes(2);
});

it("does not add a comments choice to a definition changed since review", () => {
  expect(() => stageExportComments("definition", false, () => true, "{{query (task DOING)}}")).toThrow("Review it again");
  expect(setBlockProperty).not.toHaveBeenCalled();
  expect(flushPage).not.toHaveBeenCalled();
});
