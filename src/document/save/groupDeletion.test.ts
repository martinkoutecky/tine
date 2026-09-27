import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { setToasts, toasts } from "../../toasts";
import { deletePage, loadFeed, moveBlock, pageByName, resetStore } from "../index";
import { group, markConflict } from "./engine";

const moved = { id: "delete-group-move", raw: "X", collapsed: false, children: [] };
const page = (name: string, blocks: typeof moved[] = []) => ({
  id: `pages/${name}.md`, name, kind: "page" as const, title: name,
  pre_block: null, rev: `${name}-rev`, blocks,
});

beforeEach(() => { resetStore(); setToasts([]); });
afterEach(() => vi.restoreAllMocks());

function setup() {
  loadFeed([page("A", [moved]), page("B")]);
  const disk = new Map([["A", ["X"]], ["B", [] as string[]]]);
  const trashed: string[][] = [];
  const save = vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
    for (const entry of entries) disk.set(entry.page.name, entry.page.blocks.map((block) => block.raw));
    return { ok: entries.map((_, index) => `saved-${index}`) };
  });
  const remove = vi.spyOn(backend(), "deletePage").mockImplementation(async (name) => {
    trashed.push(disk.get(name)?.slice() ?? []);
    disk.delete(name);
  });
  return { disk, trashed, save, remove };
}

it.each(["A", "B"])("P1: deleting grouped %s flushes one ordered request before delete", async (name) => {
  const { disk, trashed, save, remove } = setup();
  await moveBlock(moved.id, null, 0, "B");
  expect(await deletePage(name, "page")).toBe(true);
  expect(save).toHaveBeenCalledTimes(1);
  expect(save.mock.calls[0][0].map((entry) => entry.page.name)).toEqual(["B", "A"]);
  expect(save.mock.invocationCallOrder[0]).toBeLessThan(remove.mock.invocationCallOrder[0]);
  expect(disk.has(name)).toBe(false);
  if (name === "A") expect(disk.get("B")).toContain("X");
  else expect(trashed[0]).toContain("X");
  expect(pageByName(name)).toBeUndefined();
  expect(group("A")).toBeUndefined();
  expect(group("B")).toBeUndefined();
});

it.each(["A", "B"])("P1: a failed grouped request refuses deletion of %s without changing pages", async (name) => {
  const { disk, save, remove } = setup();
  save.mockResolvedValue({ failed: { index: 0, family: "conflict", undoFailed: [] } });
  await moveBlock(moved.id, null, 0, "B");
  expect(await deletePage(name, "page")).toBe(false);
  expect(save).toHaveBeenCalledTimes(1);
  expect(remove).not.toHaveBeenCalled();
  expect(disk.get("A")).toEqual(["X"]);
  expect(disk.get("B")).toEqual([]);
  expect(pageByName("A")).toBeTruthy();
  expect(pageByName("B")).toBeTruthy();
  expect(toasts().some((toast) => toast.message === "Resolve the conflict on “B” first.")).toBe(true);
});

it.each(["A", "B"])("P1: a conflicted member refuses deletion of grouped %s before saving", async (name) => {
  const { disk, save, remove } = setup();
  await moveBlock(moved.id, null, 0, "B");
  markConflict("A");
  expect(await deletePage(name, "page")).toBe(false);
  expect(save).not.toHaveBeenCalled();
  expect(remove).not.toHaveBeenCalled();
  expect(disk.get("A")).toEqual(["X"]);
  expect(disk.get("B")).toEqual([]);
  expect(pageByName(name)).toBeTruthy();
  expect(toasts().some((toast) => toast.message === "Resolve the conflict on “A” first.")).toBe(true);
});

it("an io failure refuses deletion with a save error", async () => {
  const { save, remove } = setup();
  save.mockResolvedValue({ failed: { index: 0, family: "io:Other", undoFailed: [] } });
  await moveBlock(moved.id, null, 0, "B");
  expect(await deletePage("A", "page")).toBe(false);
  expect(remove).not.toHaveBeenCalled();
  expect(toasts().some((toast) => toast.message === "Couldn't save “A”; the page was not deleted.")).toBe(true);
});
