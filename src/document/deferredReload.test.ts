// GH #337 (master 9869c1cfe): an external change declined while its page is held
// (block being edited, block move in flight, component draft) is replayed when
// the hold ends, through the same `applyGraphChange` entry point a live watcher
// event uses. Each hold kind is driven through its real release.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend, type GraphChange } from "../backend";
import { applyGraphChange, ensurePageLoaded, loadFeed, pageByName, pinPageWhileDrafting, resetStore, setRaw, withBlockMoving } from "./index";
import { registerPaneRouteProvider } from "./workingSet";
import { doc } from "./model";
import { endEdit, startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `dr-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[]): PageDto & { id: string; rev: string } => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev: `rev-${serial}`, blocks: raws.map(block),
});
const raws = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];
const changed = (name: string): GraphChange => ({ name, kind: "page", created: false, removed: false });

let disk: PageDto & { id: string; rev: string };
let reads = 0;
beforeAll(() => initParser());
beforeEach(() => {
  serial = 0;
  resetStore();
  reads = 0;
  vi.spyOn(backend(), "getPage").mockImplementation(async () => { reads++; return disk as never; });
  vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => ({ ok: entries.map((_, i) => `saved-${i}`) }));
});
afterEach(() => {
  endEdit("graph-switch");
  registerPaneRouteProvider(() => []);
  vi.restoreAllMocks();
});

function load(name: string, mine: string[]) {
  loadFeed([page("Journal", ["j"])]);
  ensurePageLoaded(page(name, mine));
}

describe("an external change declined mid-edit is replayed (GH #337)", () => {
  it("replays after the block editor ends", async () => {
    load("P", ["mine"]);
    startEditing(pageByName("P")!.roots[0], 0);
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    expect(raws("P")).toEqual(["mine"]); // caret is never stolen
    endEdit("blur");
    await vi.waitFor(() => expect(raws("P")).toEqual(["from disk"]));
  });

  it("replays when editing moves to a block of another page", async () => {
    load("P", ["mine"]);
    startEditing(pageByName("P")!.roots[0], 0);
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    startEditing(pageByName("Journal")!.roots[0], 0);
    await vi.waitFor(() => expect(raws("P")).toEqual(["from disk"]));
  });

  it("replays after a block move settles", async () => {
    load("P", ["mine"]);
    let settle!: () => void;
    const moving = withBlockMoving("P", () => new Promise<void>((resolve) => { settle = resolve; }));
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    expect(raws("P")).toEqual(["mine"]);
    settle();
    await moving;
    await vi.waitFor(() => expect(raws("P")).toEqual(["from disk"]));
  });

  it("replays after a component draft (title rename, sheet cell) is released", async () => {
    load("P", ["mine"]);
    const unpin = pinPageWhileDrafting(() => "P");
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    expect(raws("P")).toEqual(["mine"]);
    unpin();
    await vi.waitFor(() => expect(raws("P")).toEqual(["from disk"]));
  });

  it("the latest observation wins and one replay reads the page once", async () => {
    load("P", ["mine"]);
    startEditing(pageByName("P")!.roots[0], 0);
    disk = page("P", ["first"]);
    await applyGraphChange(changed("P"));
    disk = page("P", ["second"]);
    await applyGraphChange(changed("P"));
    expect(reads).toBe(0);
    endEdit("blur");
    await vi.waitFor(() => expect(raws("P")).toEqual(["second"]));
    expect(reads).toBe(1);
  });

  it("a replay while the page is still held stays deferred", async () => {
    load("P", ["mine"]);
    startEditing(pageByName("P")!.roots[0], 0);
    const unpin = pinPageWhileDrafting(() => "P");
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    endEdit("blur");
    expect(raws("P")).toEqual(["mine"]); // the draft still holds the page
    unpin();
    await vi.waitFor(() => expect(raws("P")).toEqual(["from disk"]));
  });

  it("never clobbers: a page that became dirty meanwhile keeps its edit", async () => {
    load("P", ["mine"]);
    startEditing(pageByName("P")!.roots[0], 0);
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    setRaw(pageByName("P")!.roots[0], "typed but not saved");
    endEdit("blur");
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(raws("P")).toEqual(["typed but not saved"]);
  });

  it("a graph switch discards the deferred change", async () => {
    load("P", ["mine"]);
    startEditing(pageByName("P")!.roots[0], 0);
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    resetStore();
    load("P", ["other graph"]);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(raws("P")).toEqual(["other graph"]);
    expect(reads).toBe(0);
  });
});
