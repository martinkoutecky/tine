import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { applyChangeSummary } from "./changeSummary";
import { bumpDataRev, bumpGraphEpoch } from "./graphSession";
import { resolveBlockBatched } from "./resolveBatch";
import { pageIcon } from "./pageIconBatch";
import type { ChangeSummary } from "./backendTypes";

let rev = 0;
function publish(fields: Partial<ChangeSummary> = {}) {
  applyChangeSummary({ rev: String(++rev), full: false, pages: [], blocks: [], linksTo: [], props: {}, inventoryChanged: false, ...fields });
  bumpDataRev(true);
}
beforeEach(() => { bumpGraphEpoch(); rev = 0; publish(); });
afterEach(() => vi.restoreAllMocks());

it("an unrelated save keeps block answers cached; a changed runtime or persisted id reloads only that id", async () => {
  const read = vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([null, null]);
  await Promise.all([resolveBlockBatched("runtime"), resolveBlockBatched("persisted")]);
  read.mockClear();
  publish({ blocks: ["other"] });
  await Promise.all([resolveBlockBatched("runtime"), resolveBlockBatched("persisted")]);
  expect(read).not.toHaveBeenCalled();
  for (const id of ["runtime", "persisted"]) {
    publish({ blocks: [id] });
    await Promise.all([resolveBlockBatched("runtime"), resolveBlockBatched("persisted")]);
    expect(read).toHaveBeenLastCalledWith([id]);
  }
});

it("a partially invalidated batch rejects only its stale block and preserves the other answer", async () => {
  type Groups = Awaited<ReturnType<ReturnType<typeof backend>["resolveBlocks"]>>;
  let finish!: (groups: Groups) => void;
  const read = vi.spyOn(backend(), "resolveBlocks").mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValue([null]);
  const a = resolveBlockBatched("a"), b = resolveBlockBatched("b");
  await vi.waitFor(() => expect(read).toHaveBeenCalledWith(["a", "b"]));
  publish({ blocks: ["a"] });
  await resolveBlockBatched("a");
  const group = { page: "B", kind: "page" as const, blocks: [] };
  finish([group, group]);
  await expect(a).resolves.toBeNull();
  await expect(b).resolves.toEqual(group);
  expect(read).toHaveBeenCalledTimes(2);
  await expect(resolveBlockBatched("b")).resolves.toEqual(group);
  expect(read).toHaveBeenCalledTimes(2);
});

it("only icon properties reload an unloaded page icon; full and a gap reload it", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  const read = vi.spyOn(backend(), "pageIcons").mockResolvedValue({ Target: "T" });
  pageIcon("Target");
  await vi.waitFor(() => expect(pageIcon("Target")).toBe("T"));
  read.mockClear();
  publish({ pages: ["target"], props: { target: ["tags"] } });
  pageIcon("Target");
  await Promise.resolve();
  expect(read).not.toHaveBeenCalled();
  for (const fields of [{ props: { target: ["icon"] } }, { props: { target: ["tine.icon"] } }, { full: true }]) {
    publish(fields);
    pageIcon("Target");
    await vi.waitFor(() => expect(read).toHaveBeenCalledWith(["Target"]));
    read.mockClear();
  }
  rev += 2;
  publish();
  pageIcon("Target");
  await vi.waitFor(() => expect(read).toHaveBeenCalledWith(["Target"]));
});
