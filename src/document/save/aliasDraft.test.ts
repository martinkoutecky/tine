import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { flushPage, isConflicted, loadFeed, pageByName, resetStore, setRaw } from "../index";
import { conflictReason, persistTogether, resolveConflict } from "./engine";
import { doc } from "../model";
import { initParser } from "../../render/parse";
import { setToasts } from "../../toasts";
import { startEditing, endEdit } from "../../editorController";
import { pinPageWhileDrafting } from "../workingSet";
import type { BlockDto, PageRead } from "../../types";

// L13 (B): an alias draft's first save appends it to the alias owner; an edit
// typed while that save was in flight leaves the draft conflicted. "Keep mine"
// must write the draft's CURRENT content in place of the landed copy, never
// append the whole draft a second time.

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `alias-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[], rev: string): PageRead => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev, blocks: raws.map(block),
});

function ownerDisk(initial: string[]) {
  const disk = { raws: initial.slice(), rev: "owner-0", writes: 0 };
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "alias", owners: ["pages/Owner.md"] });
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => page("Owner", disk.raws, disk.rev));
  return disk;
}

beforeAll(() => initParser());
beforeEach(() => { serial = 0; resetStore(); setToasts([]); });
afterEach(() => vi.restoreAllMocks());

describe("alias draft save", () => {
  it("Keep mine after a mid-save edit replaces the landed copy instead of appending the draft again", async () => {
    loadFeed([{ ...page("Draft", ["a", "b"], "x"), id: undefined, rev: undefined }]);
    const [, second] = pageByName("Draft")!.roots;
    setRaw(second, "b1");
    const disk = ownerDisk(["owner"]);
    vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
      const entry = entries[0];
      if (entry.baseRev !== disk.rev) return { failed: { index: 0, family: "conflict", diskRev: disk.rev, undoFailed: [] } };
      disk.raws = entry.page.blocks.map((b) => b.raw);
      disk.rev = `owner-${++disk.writes}`;
      // The user keeps typing in the draft while its first save is in flight.
      if (disk.writes === 1) setRaw(second, "b2");
      return { ok: [disk.rev] };
    });
    expect(await flushPage("Draft")).toBe(false);
    expect(isConflicted("Draft")).toBe(true);
    expect(disk.raws).toEqual(["owner", "a", "b1"]);
    expect(await resolveConflict("Draft", "mine")).toBe(true);
    expect(disk.raws).toEqual(["owner", "a", "b2"]);
    expect(pageByName("Draft")).toBeUndefined();
    expect(pageByName("Owner")!.roots.map((id) => doc.byId[id].raw)).toEqual(["owner", "a", "b2"]);
  });

  it("a landed draft whose owner changed since refuses Keep mine and writes nothing", async () => {
    loadFeed([{ ...page("Draft", ["a"], "x"), id: undefined, rev: undefined }]);
    const [first] = pageByName("Draft")!.roots;
    setRaw(first, "a1");
    const disk = ownerDisk(["owner"]);
    const save = vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
      disk.raws = entries[0].page.blocks.map((b) => b.raw);
      disk.rev = `owner-${++disk.writes}`;
      if (disk.writes === 1) setRaw(first, "a2");
      return { ok: [disk.rev] };
    });
    expect(await flushPage("Draft")).toBe(false);
    // Another tool rewrote the owner's tail after the draft landed.
    disk.raws = ["owner", "rewritten elsewhere"];
    disk.rev = "external";
    expect(await resolveConflict("Draft", "mine")).toBe(false);
    expect(save).toHaveBeenCalledTimes(1);
    expect(disk.raws).toEqual(["owner", "rewritten elsewhere"]);
    expect(isConflicted("Draft")).toBe(true);
    expect(pageByName("Draft")!.roots.map((id) => doc.byId[id].raw)).toEqual(["a2"]);
  });

  it("a grouped alias draft edited mid-save is not appended again by Keep mine", async () => {
    loadFeed([{ ...page("Draft", ["a", "b"], "x"), id: undefined, rev: undefined }, page("Other", ["other"], "other-0")]);
    const [, second] = pageByName("Draft")!.roots;
    const disk = ownerDisk(["owner"]);
    vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
      const owner = entries.find((entry) => entry.id === "pages/Owner.md")!;
      if (owner.baseRev !== disk.rev) return { failed: { index: 0, family: "conflict", diskRev: disk.rev, undoFailed: [] } };
      disk.raws = owner.page.blocks.map((b) => b.raw);
      disk.rev = `owner-${++disk.writes}`;
      if (disk.writes === 1) setRaw(second, "b2");
      return { ok: entries.map((entry) => entry === owner ? disk.rev : "other-1") };
    });
    setRaw(second, "b1");
    void persistTogether(["Draft", "Other"], "move-blocks");
    await flushPage("Draft");
    expect(conflictReason("Draft")?.kind).toBe("alias-owner-busy");
    expect(disk.raws).toEqual(["owner", "a", "b1"]);
    expect(await resolveConflict("Draft", "mine")).toBe(true);
    expect(disk.raws).toEqual(["owner", "a", "b2"]);
  });
});

describe("alias owner replacement protects uncommitted input", () => {
  it.each(["single", "group"] as const)("%s refuses an owner already held by an IME editor", async (mode) => {
    loadFeed([{ ...page("Draft", ["draft"], "x"), id: undefined, rev: undefined }, page("Owner", ["owner"], "owner-0"), page("Other", ["other"], "other-0")]);
    const disk = ownerDisk(["owner"]);
    const ownerRoot = pageByName("Owner")!.roots[0];
    const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["owner-1", "other-1"] });
    startEditing(ownerRoot, 0, null); // IME value is still DOM-local; no setRaw.
    setRaw(pageByName("Draft")!.roots[0], "draft edited");
    try {
      if (mode === "group") void persistTogether(["Draft", "Other"], "move-blocks");
      expect(await flushPage("Draft")).toBe(false);
      expect(save).not.toHaveBeenCalled();
      expect(doc.byId[ownerRoot]?.raw).toBe("owner");
      expect(disk.raws).toEqual(["owner"]);
    } finally { endEdit("page-navigation"); }
  });

  it.each(["single", "group"] as const)("%s preserves an owner whose editor/draft becomes busy during the save", async (mode) => {
    for (const hold of ["editor", "draft"] as const) {
      resetStore();
      loadFeed([{ ...page("Draft", ["draft"], "x"), id: undefined, rev: undefined }, page("Owner", ["owner"], "owner-0"), page("Other", ["other"], "other-0")]);
      const disk = ownerDisk(["owner"]);
      const ownerRoot = pageByName("Owner")!.roots[0];
      let release = () => {};
      vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
        disk.raws = entries.find((entry) => entry.id === "pages/Owner.md")!.page.blocks.map((b) => b.raw);
        if (hold === "editor") startEditing(ownerRoot, 0, null);
        else release = pinPageWhileDrafting(() => "Owner");
        return { ok: entries.map(() => "owner-1") };
      });
      setRaw(pageByName("Draft")!.roots[0], "draft edited");
      try {
        if (mode === "group") void persistTogether(["Draft", "Other"], "move-blocks");
        await flushPage("Draft");
        expect(pageByName("Draft")).toBeDefined();
        expect(isConflicted("Draft")).toBe(true);
        expect(doc.byId[ownerRoot]?.raw).toBe("owner");
        expect(disk.raws).toEqual(["owner", "draft edited"]);
      } finally { endEdit("page-navigation"); release(); }
      vi.restoreAllMocks();
    }
  });
});
