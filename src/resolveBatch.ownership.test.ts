import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { bumpGraphEpoch } from "./graphSession";
import { resolveBlockBatched, resolvedBlockRefSync } from "./resolveBatch";
import type { RefGroup } from "./types";

afterEach(() => vi.restoreAllMocks());

it("does not deliver a block resolution from a retired graph to its caller", async () => {
  let complete!: (groups: (RefGroup | null)[]) => void;
  const read = vi.spyOn(backend(), "resolveBlocks").mockImplementationOnce(() =>
    new Promise((resolve) => { complete = resolve; })
  );
  const pending = resolveBlockBatched("old-block");
  await vi.waitFor(() => expect(read).toHaveBeenCalledWith(["old-block"]));
  bumpGraphEpoch();
  complete([{ page: "Old", kind: "page", blocks: [] }]);
  await expect(pending).resolves.toBeNull();
  expect(resolvedBlockRefSync("old-block")).toBeNull();
});
