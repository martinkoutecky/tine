import { afterEach, describe, expect, it, vi } from "vitest";
import { __setBackendForTest, type Backend } from "./backend";
import {
  clearLiveSaveConflict,
  conflictQueue,
  refreshLiveSaveConflictDraft,
  registerLiveSaveConflict,
  retireLiveSaveConflict,
  setConflictQueue,
  setGraphMeta,
  setToasts,
  toasts,
} from "./ui";
import type { PageDto } from "./types";

// Wave-2 review B3-1: `clearLiveSaveConflict` dropped the in-memory draft but
// never retired the app-private capsule, so a resolved draft resurrected after
// the next restart. Every clear path retires durably; a retirement that cannot
// be proven keeps the draft in the queue (awaited path) or tells the user
// (fire-and-forget path).

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

const page: PageDto = {
  name: "Draft",
  path: "pages/Draft.md",
  format: "md",
  blocks: [{ id: "b1", raw: "retained draft", children: [] }],
  rev: "rev-1",
} as unknown as PageDto;

function stubBackend(overrides: Partial<Backend>): {
  retire: ReturnType<typeof vi.fn>;
  store: ReturnType<typeof vi.fn>;
} {
  const retire = vi.fn(async () => {});
  const store = vi.fn(async () => {});
  __setBackendForTest({
    storeConflictCapsule: store,
    loadConflictCapsules: async () => [],
    retireConflictCapsule: retire,
    ...overrides,
  } as unknown as Backend);
  return { retire, store };
}

afterEach(() => {
  __setBackendForTest(null);
  setConflictQueue([]);
  setGraphMeta(null);
  setToasts([]);
});

describe("live-save conflict retirement", () => {
  it("keeps a replacement queue entry when an older retirement completes", async () => {
    let finish!: () => void;
    stubBackend({ retireConflictCapsule: () => new Promise<void>((resolve) => { finish = resolve; }) });
    setGraphMeta({ root: "/graphs/A" } as never);
    await registerLiveSaveConflict(page, "rev-1", 1);
    const retiring = retireLiveSaveConflict("Draft");
    await registerLiveSaveConflict({ ...page, blocks: [{ ...page.blocks[0], raw: "new retained draft" }] }, "rev-2", 2);
    const replacement = conflictQueue()[0];
    finish();
    await expect(retiring).rejects.toThrow("retained draft changed");
    expect(conflictQueue()).toEqual([replacement]);
  });

  it("does not rewrite the capsule for identical conflicted draft save attempts", async () => {
    const { store } = stubBackend({});
    setGraphMeta({ root: "/graphs/A" } as never);
    await registerLiveSaveConflict(page, "rev-1", 1);

    await refreshLiveSaveConflictDraft(page);
    await refreshLiveSaveConflictDraft(page);
    await refreshLiveSaveConflictDraft(page);

    expect(store).toHaveBeenCalledTimes(1);
  });

  it("clearLiveSaveConflict retires the on-disk capsule for the cleared page", async () => {
    const { retire } = stubBackend({});
    setGraphMeta({ root: "/graphs/A" } as never);
    await registerLiveSaveConflict(page, "rev-1", 1);
    expect(conflictQueue().map((c) => c.page_name)).toEqual(["Draft"]);

    clearLiveSaveConflict("Draft");
    await flush();

    expect(conflictQueue()).toEqual([]);
    expect(retire).toHaveBeenCalledWith("/graphs/A", "Draft");
  });

  it("tells the user when a fire-and-forget retirement fails instead of silently resurrecting the draft", async () => {
    const { retire } = stubBackend({
      retireConflictCapsule: vi.fn(async () => {
        throw new Error("EIO");
      }),
    });
    void retire;
    setGraphMeta({ root: "/graphs/A" } as never);
    await registerLiveSaveConflict(page, "rev-1", 1);

    clearLiveSaveConflict("Draft");
    await flush();

    expect(conflictQueue()).toEqual([]);
    const toast = toasts().at(-1);
    expect(toast?.kind).toBe("error");
    expect(toast?.message).toContain("could not retire its restart-recovery copy");
  });

  it("retireLiveSaveConflict keeps the draft queued until the capsule is durably gone", async () => {
    stubBackend({
      retireConflictCapsule: vi.fn(async () => {
        throw new Error("EIO");
      }),
    });
    setGraphMeta({ root: "/graphs/A" } as never);
    await registerLiveSaveConflict(page, "rev-1", 1);

    await expect(retireLiveSaveConflict("Draft")).rejects.toThrow("EIO");
    expect(conflictQueue().map((c) => c.page_name)).toEqual(["Draft"]);
  });
});
