import { beforeEach, expect, it, vi } from "vitest";
vi.mock("./warmCache", () => ({ waitForWarmCache: async () => true }));
import { backend, type GraphAnswersChange, type GraphChange } from "./backend";
import { blockRefCount } from "./blockRefCounts";
import { installPageIndex } from "./pageIndex";
import { applyGraphChange, applyGraphChangesBulk, resetStore, loadFeed, setRaw, flushPage, pageByName } from "./document";
import { bumpGraphEpoch } from "./graphSession";
import { applyGraphAnswers } from "./graphAnswers";
const one = "00000000-0000-4000-8000-000000000001";
const two = "00000000-0000-4000-8000-000000000002";
const answer = (rev: number, inventoryChanged: boolean, blockRefCounts: Record<string, number>): GraphAnswersChange => ({ rev: String(rev), inventoryChanged, blockRefCounts });
const event = (answers: GraphAnswersChange): GraphChange => ({ name: "Outside", kind: "page", created: false, removed: false, answers });
beforeEach(async () => {
  resetStore(); vi.restoreAllMocks();
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "100", entries: [] });
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({ [one]: 1 });
  installPageIndex(); bumpGraphEpoch();
  await vi.waitFor(() => expect(blockRefCount(one)).toBe(1));
  await vi.waitFor(() => expect(backend().pageInventory).toHaveBeenCalled());
  vi.mocked(backend().pageInventory).mockClear();
  vi.mocked(backend().getBlockRefCounts).mockClear();
});
it("save acknowledgements refresh changed names and only their count targets", async () => {
  loadFeed([{ id: "pages/A.md", rev: "old", name: "A", kind: "page", title: "A", pre_block: null,
    blocks: [{ id: "body", raw: "before", collapsed: false, children: [] }] }]);
  vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["new"], changes: answer(2, true, { [one]: 0, [two]: 3 }) });
  setRaw(pageByName("A")!.roots[0], "after");
  expect(await flushPage("A")).toBe(true);
  expect(blockRefCount(one)).toBe(0); expect(blockRefCount(two)).toBe(3);
  await vi.waitFor(() => expect(backend().pageInventory).toHaveBeenCalledTimes(1));
  expect(backend().getBlockRefCounts).not.toHaveBeenCalled();
});
it("external single and bulk changes update both answers, including signal-only own deletes/renames", async () => {
  await applyGraphChange(event(answer(2, true, { [two]: 2 })));
  expect(blockRefCount(two)).toBe(2);
  await vi.waitFor(() => expect(backend().pageInventory).toHaveBeenCalledTimes(1));
  await applyGraphChangesBulk({ changes: Array.from({length:40}, () => event(answer(3, false, {}))), answers: answer(3, true, { [one]:0, [two]:40 }) });
  expect(blockRefCount(one)).toBe(0); expect(blockRefCount(two)).toBe(40);
  await vi.waitFor(() => expect(backend().pageInventory).toHaveBeenCalledTimes(2));
  await applyGraphChangesBulk({ changes: [], answers: answer(4, true, { [two]:0 }) });
  expect(blockRefCount(two)).toBe(0);
  await vi.waitFor(() => expect(backend().pageInventory).toHaveBeenCalledTimes(3));
  expect(backend().getBlockRefCounts).not.toHaveBeenCalled();
});
it("out-of-order deltas order each target independently and reject another binding", async () => {
  applyGraphAnswers(answer(5, false, { [one]:5 }));
  applyGraphAnswers(answer(4, true, { [one]:4, [two]:4 }));
  expect(blockRefCount(one)).toBe(5); expect(blockRefCount(two)).toBe(4);
  await vi.waitFor(() => expect(backend().pageInventory).toHaveBeenCalledTimes(1));
  await applyGraphChange({ ...event(answer(10, true, { [one]:10 })), binding_generation: -1 });
  await applyGraphChangesBulk({ changes: [], binding_generation: -1, answers: answer(10, true, { [two]:10 }) });
  expect(blockRefCount(one)).toBe(5); expect(blockRefCount(two)).toBe(4);
  expect(backend().pageInventory).toHaveBeenCalledTimes(1);
});
it("an initial count response landing after a notification cannot overwrite its targets", async () => {
  let finish!: (counts: Record<string, number>) => void;
  vi.mocked(backend().getBlockRefCounts).mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  bumpGraphEpoch();
  await vi.waitFor(() => expect(finish).toBeDefined());
  applyGraphAnswers(answer(1, false, { [two]:7 }));
  finish({ [one]:1, [two]:0 });
  await vi.waitFor(() => expect(blockRefCount(one)).toBe(1));
  expect(blockRefCount(two)).toBe(7);
});
