// Contract 2 of og 20b (GH #304 family, master a7584bb40 + its lifecycle.ts
// successor): unsaved text is never discarded without the user's choice. Every
// path that REPLACES a loaded page instance asks one gate, `reloadDisposition`,
// which sees store state (dirty / saving / conflicted / grouped), the active
// editor, a block move in flight, and component-local drafts that registered with
// `pinPageWhileDrafting` (a sheet cell, the page-title rename input). Each test
// drives one replacement entry point with an unsaved draft of each kind.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { ensurePageLoaded, flushAll, isDirty, loadFeed, pageByName, pinPageWhileDrafting, resetStore, setRaw } from "./index";
import { loadSingle, reloadDisposition, reloadPageIfStillSafe, registerPaneRouteProvider } from "./workingSet";
import { doc } from "./model";
import { endEdit, startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import type { Route } from "../routeTypes";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `rg-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[], id = `pages/${name}.md`): PageDto & { id: string; rev: string } => ({
  id, name, title: name, kind: "page", pre_block: null, rev: `rev-${serial}`, blocks: raws.map(block),
});
const raws = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];

let unpin: (() => void) | null = null;
beforeAll(() => initParser());
beforeEach(() => {
  serial = 0;
  resetStore();
  vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => ({ ok: entries.map((_, i) => `saved-${i}`) }));
});
afterEach(() => {
  unpin?.(); unpin = null;
  endEdit("graph-switch");
  registerPaneRouteProvider(() => []);
  vi.restoreAllMocks();
});

/** The kinds of uncommitted input a loaded page can hold. */
const drafts: Array<[string, (name: string) => void]> = [
  ["an active block editor", (name) => startEditing(pageByName(name)!.roots[0], 0)],
  ["a component-local draft (title rename, sheet cell)", (name) => { unpin = pinPageWhileDrafting(() => name); }],
  ["an unsaved edit", (name) => setRaw(pageByName(name)!.roots[0], "typed but not saved")],
];

describe.each(drafts)("a page holding %s is never replaced", (_label, hold) => {
  it("by navigation to the same page (loadSingle)", () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["mine"]));
    hold("P");
    const before = pageByName("P");
    loadSingle(page("P", ["disk"]));
    expect(pageByName("P")).toBe(before);
    expect(raws("P")).not.toEqual(["disk"]);
  });

  it("by an external-change reload", () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["mine"]));
    hold("P");
    expect(reloadPageIfStillSafe("P", page("P", ["disk"]))).toBe(false);
    expect(raws("P")).not.toEqual(["disk"]);
  });

  it("by a same-named file from another path (GH #304)", () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["mine"]));
    hold("P");
    ensurePageLoaded(page("P", ["other file"], "pages/elsewhere/P.md"));
    expect(pageByName("P")!.id).toBe("pages/P.md");
    expect(raws("P")).not.toEqual(["other file"]);
  });

  it("by a journal feed refresh", () => {
    loadFeed([page("Day", ["mine"])]);
    hold("Day");
    loadFeed([page("Day", ["disk"])], { endEdit: false });
    expect(raws("Day")).not.toEqual(["disk"]);
  });

  it("by working-set eviction", () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["mine"]));
    hold("P");
    for (let i = 0; i < 90; i++) ensurePageLoaded(page(`Filler ${i}`, ["x"]));
    expect(pageByName("P")).toBeDefined();
  });

  it("when the pane showing it closes", () => {
    let routes: Route[] = [{ kind: "page", name: "P", pageKind: "page" }];
    registerPaneRouteProvider(() => routes);
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["mine"]));
    hold("P");
    routes = []; // the pane closed; its page is no longer routed anywhere
    for (let i = 0; i < 90; i++) ensurePageLoaded(page(`Filler ${i}`, ["x"]));
    expect(pageByName("P")).toBeDefined();
  });
});

describe("the replacement gate still replaces a clean, idle page", () => {
  it("reloads an external change and releases a draft pin", async () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["mine"]));
    unpin = pinPageWhileDrafting(() => "P");
    expect(reloadDisposition("P")).toBe("skip");
    unpin(); unpin = null;
    expect(reloadDisposition("P")).toBe("reload");
    expect(reloadPageIfStillSafe("P", page("P", ["disk"]))).toBe(true);
    expect(raws("P")).toEqual(["disk"]);
  });

  it("saves an unsaved edit once its pane closes instead of dropping it", async () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["mine"]));
    setRaw(pageByName("P")!.roots[0], "typed");
    expect(await flushAll()).toBe(true);
    expect(isDirty("P")).toBe(false);
  });
});
