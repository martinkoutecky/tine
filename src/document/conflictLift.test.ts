// og I1c (port of master c68c0b6e7, Direct Files data-safety audit F17): a
// conflict raised by a transient removal (an external editor's temp+rename, a
// mid-delivery sync pass) is lifted when the file provably comes back to the
// editor's own baseline, and the edit it froze is saved. A file that returns
// with different bytes stays conflicted. Driven through applyGraphChange, the
// entry point App wires to the native watcher event.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend, type GraphChange } from "../backend";
import { applyGraphChange, ensurePageLoaded, installExternalChangeUiHandler, isConflicted, isDirty, pageByName, resetStore, setRaw } from "./index";
import { doc } from "./model";
import { flushAll } from "./save/engine";
import { setToasts } from "../toasts";
import type { PageDto } from "../types";

const NAME = "Synced";
const dto = (rev: string, raw: string): PageDto & { id: string; rev: string } => ({
  id: `pages/${NAME}.md`, name: NAME, title: NAME, kind: "page", pre_block: null, rev,
  blocks: [{ id: "b1", raw, collapsed: false, children: [] }],
});
const event = (patch: Partial<GraphChange>): GraphChange => ({ name: NAME, kind: "page", created: false, removed: false, ...patch });
const raws = () => pageByName(NAME)?.roots.map((id) => doc.byId[id].raw) ?? [];

let disk: (PageDto & { id: string; rev: string }) | null;
beforeAll(() => initParser());
beforeEach(() => {
  resetStore(); setToasts([]); disk = null;
  vi.spyOn(backend(), "getPage").mockImplementation(async () => disk as never);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => disk as never);
  installExternalChangeUiHandler(() => ({ pageOpen: () => true, journalsOpen: false, leaveRemovedPage: () => {}, restartJournalFeed: () => {} }));
});
afterEach(() => vi.restoreAllMocks());

async function dirtyThenRemoved() {
  ensurePageLoaded(dto("rev-1", "original"));
  setRaw(pageByName(NAME)!.roots[0], "mine");
  await applyGraphChange(event({ removed: true }));
  expect(isConflicted(NAME)).toBe(true);
}

describe("a transient removal's conflict (og I1c, master c68c0b6e7)", () => {
  it("is lifted when the file returns to the baseline, and the frozen edit saves against it", async () => {
    await dirtyThenRemoved();
    const saves = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["rev-2"] } as never);
    disk = dto("rev-1", "original");
    await applyGraphChange(event({ created: true }));

    expect(isConflicted(NAME)).toBe(false);
    expect(raws()).toEqual(["mine"]);
    expect(isDirty(NAME)).toBe(true);
    await expect(flushAll()).resolves.toBe(true);
    // Saved against the baseline the file came back to, not forced over it.
    expect(JSON.stringify(saves.mock.calls.at(-1))).toContain("rev-1");
  });

  it("re-arms an edit a refused save had already dropped, so it is written once the file is back", async () => {
    ensurePageLoaded(dto("rev-1", "original"));
    setRaw(pageByName(NAME)!.roots[0], "mine");
    // The save runs while the file is gone: the backend refuses it as deleted.
    const saves = vi.spyOn(backend(), "savePages").mockRejectedValueOnce(new Error("deleted"));
    await expect(flushAll()).resolves.toBe(false);
    expect(isConflicted(NAME)).toBe(true);
    expect(isDirty(NAME)).toBe(false);

    saves.mockResolvedValue({ ok: ["rev-2"] } as never);
    disk = dto("rev-1", "original");
    await applyGraphChange(event({ created: true }));
    expect(isConflicted(NAME)).toBe(false);
    await expect(flushAll()).resolves.toBe(true);
    expect(saves).toHaveBeenCalledTimes(2);
    expect(raws()).toEqual(["mine"]);
  });

  it("stays when the file returns with different bytes", async () => {
    await dirtyThenRemoved();
    const saves = vi.spyOn(backend(), "savePages");
    disk = dto("rev-9", "theirs");
    await applyGraphChange(event({ created: true }));

    expect(isConflicted(NAME)).toBe(true);
    expect(raws()).toEqual(["mine"]);
    expect(saves).not.toHaveBeenCalled();
  });
});
