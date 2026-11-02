import { afterEach, expect, it, vi } from "vitest";
import type { RefGroup } from "./types";

const flushed = vi.hoisted(() => ({ cb: null as null | ((affected?: Set<string>) => void) }));
vi.mock("./changeKeys", async (original) => ({
  ...await original<typeof import("./changeKeys")>(),
  onChangeKeysFlushed: (cb: (affected?: Set<string>) => void) => { flushed.cb = cb; },
}));

import { backend } from "./backend";
import { resolveBlockBatched } from "./resolveBatch";

afterEach(() => vi.restoreAllMocks());
const fresh: RefGroup = { page: "Fresh", kind: "page", blocks: [] };

// L2-10: a save of the target during the lookup is not "the block does not exist".
it("asks again when the block's key advances during the lookup", async () => {
  let complete!: (groups: (RefGroup | null)[]) => void;
  const read = vi.spyOn(backend(), "resolveBlocks")
    .mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }))
    .mockResolvedValueOnce([fresh]);
  const answer = resolveBlockBatched("moving");
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
  flushed.cb!(new Set(["block:moving"]));
  complete([{ page: "Old", kind: "page", blocks: [] }]);
  await expect(answer).resolves.toEqual(fresh);
  expect(read).toHaveBeenCalledTimes(2);
});

it("asks again when the key advances before the batch was sent", async () => {
  const read = vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([fresh]);
  const answer = resolveBlockBatched("queued");
  flushed.cb!(new Set(["block:queued"]));
  await expect(answer).resolves.toEqual(fresh);
  expect(read).toHaveBeenCalledTimes(1);
});

it("still answers null for a block that is truly absent", async () => {
  vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([null]);
  await expect(resolveBlockBatched("absent")).resolves.toBeNull();
});
