// master 695527da9 ("preserve delete semantics after rebase"): a by-name delete
// of a page that is not loaded must still delete it, and deleting a page the user
// has left in conflict must not first write its draft over the external bytes.
// og's deletePage (document/workingSet.ts) never had master's quiescence gate,
// so these pin the equivalent outcome rather than a fix.
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { deletePage, isDirty, pageByName, resetStore, setRaw } from ".";
import { markConflict } from "./save/engine";
import { loadSingle } from "./workingSet";

beforeAll(() => initParser());
beforeEach(() => {
  resetStore();
  vi.restoreAllMocks();
});

describe("delete semantics (master 695527da9)", () => {
  it("deletes a page that is not loaded, by name", async () => {
    const remove = vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined);
    const save = vi.spyOn(backend(), "savePages");
    expect(pageByName("Elsewhere")).toBeUndefined();
    expect(await deletePage("Elsewhere", "page")).toBe(true);
    expect(remove).toHaveBeenCalledWith("Elsewhere", "page");
    expect(save).not.toHaveBeenCalled();
  });

  it("deletes a conflicted page without first writing its draft over the disk copy", async () => {
    loadSingle({
      name: "Clash", kind: "page", title: "Clash", pre_block: null, id: "pages/Clash.md", rev: "r1",
      blocks: [{ id: "c1", raw: "base", collapsed: false, children: [] }],
    } as never);
    setRaw("c1", "my draft");
    markConflict("Clash");
    expect(isDirty("Clash")).toBe(true);
    const save = vi.spyOn(backend(), "savePages");
    const remove = vi.spyOn(backend(), "deletePage").mockResolvedValue(undefined);
    expect(await deletePage("Clash", "page", "pages/Clash.md")).toBe(true);
    expect(save).not.toHaveBeenCalled();
    expect(remove).toHaveBeenCalledWith("Clash", "page", "pages/Clash.md");
    expect(pageByName("Clash")).toBeUndefined();
  });
});
