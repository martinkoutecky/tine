// A page deleted right after a cross-page move (P1): the move reaches the host
// as one request carrying both pages (STEP3 §8), and the delete settles the
// page's input first; the host refuses a delete over unsaved input (§12, D4),
// so a conflicted page takes disk before its delete (Q-TS3).
import { afterEach, beforeEach, expect, it, vi, type MockInstance } from "vitest";
import { backend, type Backend } from "../../backend";
import { setToasts, toasts } from "../../toasts";
import { deletePage, isConflicted, loadFeed, moveBlock, pageByName, resetStore } from "../index";
import { startEditing } from "../../editorController";
import { bindTestHost, type TestHost } from "../host/wiring.test.support";
import type { PageDto } from "../../types";

const moved = { id: "delete-group-move", raw: "X", collapsed: false, children: [] };
const page = (name: string, blocks: typeof moved[] = []) => ({
  id: `pages/${name}.md`, name, kind: "page" as const, title: name,
  pre_block: null, rev: `${name}-rev`, blocks,
});
const staying = { id: "delete-group-stay", raw: "Y", collapsed: false, children: [] };
const raws = (dto: PageDto) => dto.blocks.map((block) => block.raw);

beforeEach(() => { resetStore(); setToasts([]); });
afterEach(() => { vi.restoreAllMocks(); resetStore(); });

async function setup(holdA = false): Promise<{ host: TestHost; move: MockInstance<Backend["pageMove"]>;
  remove: MockInstance<Backend["pageDelete"]>; submit: MockInstance<Backend["pageSubmit"]> }> {
  loadFeed([page("A", holdA ? [moved, staying] : [moved]), page("B")]);
  const host = await bindTestHost();
  // The user is editing A's other block: the window holds A, so the host's
  // mail for A reaches it.
  if (holdA) startEditing(staying.id);
  const move = vi.spyOn(backend(), "pageMove");
  const submit = vi.spyOn(backend(), "pageSubmit");
  const remove = vi.spyOn(backend(), "pageDelete");
  await moveBlock(moved.id, null, 0, "B");
  await vi.waitFor(() => expect(move).toHaveBeenCalledOnce());
  return { host, move, remove, submit };
}

it.each(["A", "B"])("P1: deleting %s after a cross-page move sends the move as one request before the delete", async (name) => {
  const { move, remove, submit } = await setup();
  expect(await deletePage(name, "page")).toBe(true);
  expect(move).toHaveBeenCalledOnce();
  const [, , [sourceKey, sourceDto], [receiverKey, receiverDto]] = move.mock.calls[0];
  expect([sourceKey, raws(sourceDto)]).toEqual(["pages/A.md", []]);
  expect([receiverKey, raws(receiverDto)]).toEqual(["pages/B.md", ["X"]]);
  expect(submit).not.toHaveBeenCalled();
  expect(remove).toHaveBeenCalledOnce();
  expect(remove.mock.calls[0].slice(1, 3)).toEqual([name, "page"]);
  expect(move.mock.invocationCallOrder[0]).toBeLessThan(remove.mock.invocationCallOrder[0]);
  expect(pageByName(name)).toBeUndefined();
  expect(pageByName(name === "A" ? "B" : "A")).toBeTruthy();
});

it.each(["A", "B"])("P1: a move the host took but cannot save refuses deletion of %s with a save error", async (name) => {
  const { remove } = await setup();
  // The host owes both pages (it took the move) and its writes keep failing.
  vi.spyOn(backend(), "pageOwed").mockImplementation(async (_session, paths) =>
    (paths ?? ["pages/A.md", "pages/B.md"]).map((key) => ({ key, version: 2 })));
  vi.spyOn(backend(), "pageWait").mockResolvedValue(false);
  expect(await deletePage(name, "page")).toBe(false);
  expect(remove).not.toHaveBeenCalled();
  expect(pageByName("A")).toBeTruthy();
  expect(pageByName("B")).toBeTruthy();
  expect(toasts().some((toast) => toast.message === `Couldn't save “${name}”; the page was not deleted.`)).toBe(true);
});

// Restated for Q-TS3 (master parity, 695527da9; relaxation ledger): Delete on
// a conflicted page takes disk (`page_discard`) and then deletes; the unsaved
// buffer is never saved over the file first (the confirmation says it goes).
it("P1: a conflicted page's deletion takes disk first, saves nothing, then deletes", async () => {
  const { host, remove, submit } = await setup(true);
  host.notice("pages/A.md", { conflictReported: true }, { version: 9, conflict: true });
  await vi.waitFor(() => expect(isConflicted("A")).toBe(true));
  const discard = vi.spyOn(backend(), "pageDiscard");
  const before = submit.mock.calls.length;
  expect(await deletePage("A", "page")).toBe(true);
  expect(submit.mock.calls.length).toBe(before);
  expect(discard).toHaveBeenCalledOnce();
  expect(discard.mock.calls[0][2]).toBe("pages/A.md");
  expect(remove).toHaveBeenCalledOnce();
  expect(discard.mock.invocationCallOrder[0]).toBeLessThan(remove.mock.invocationCallOrder[0]);
  expect(pageByName("A")).toBeUndefined();
});
