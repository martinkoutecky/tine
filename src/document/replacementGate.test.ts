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
import { editingId, endEdit, startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import type { Route } from "../routeTypes";
import { answerOpensFromDocument } from "./host/documentHost.test.support";
import { bindTestHost, mailPage, type TestHost } from "./host/wiring.test.support";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `rg-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[], id = `pages/${name}.md`): PageDto & { id: string; rev: string } => ({
  id, name, title: name, kind: "page", pre_block: null, rev: `rev-${serial}`, blocks: raws.map(block),
});
const raws = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];

let unpin: (() => void) | null = null;
let host: TestHost;
beforeAll(() => initParser());
beforeEach(async () => {
  serial = 0;
  resetStore();
  host = await bindTestHost();
  answerOpensFromDocument(host);
});
afterEach(() => {
  unpin?.(); unpin = null;
  endEdit("graph-switch");
  registerPaneRouteProvider(() => []);
  vi.restoreAllMocks();
});

/** The kinds of uncommitted input a loaded page can hold. An IME composition
 *  takes the component-local draft pin (Block.tsx). */
const drafts: Array<[string, (name: string) => void]> = [
  ["a component-local draft (title rename, sheet cell, IME composition)", (name) => { unpin = pinPageWhileDrafting(() => name); }],
  ["an unsaved edit", (name) => setRaw(pageByName(name)!.roots[0], "typed but not saved")],
];
/** Holds of the name slot and the working set, not of the file's content: an
 *  open editor (its typing is already in the store) is one too. */
const slotHolds: Array<[string, (name: string) => void]> = [
  ...drafts,
  ["an active block editor", (name) => startEditing(pageByName(name)!.roots[0], 0)],
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

  it("by a journal feed refresh", () => {
    loadFeed([page("Day", ["mine"])]);
    hold("Day");
    loadFeed([page("Day", ["disk"])], { endEdit: false });
    expect(raws("Day")).not.toEqual(["disk"]);
  });
});

describe.each(slotHolds)("a page holding %s keeps its name slot and stays loaded", (_label, hold) => {
  it("by a same-named file from another path (GH #304)", () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["mine"]));
    hold("P");
    ensurePageLoaded(page("P", ["other file"], "pages/elsewhere/P.md"));
    expect(pageByName("P")!.id).toBe("pages/P.md");
    expect(raws("P")).not.toEqual(["other file"]);
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

// A page changed on disk while one of its blocks is open in the editor (Syncthing
// delivering while Tine is in the background). Every keystroke is already in the
// store, so the editor holds no input the disk version could clobber: the page
// reloads and the editor stays on the same block.
// A page with a block open in the editor is open in the page host (B-Q2): a
// same-file disk change reaches it as host mail (STEP3 §4.2 Push), installed
// through the working set's replacement, which carries the editor across.
describe("a same-file reload while a block is open in the editor", () => {
  const ID = "6f1f0c1e-0000-4000-8000-000000000652";
  let version = 5000;
  const open = async (raws: string[], at: number) => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", raws));
    startEditing(pageByName("P")!.roots[at], 1);
    await vi.waitFor(() => expect(backend().pageOpen).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 0));
  };
  /** The host's push of P's disk text, as the native host mails it. */
  const pushed = (raws: string[]) => {
    const dto = page("P", raws);
    host.deliver({ key: dto.id, answer: null, page: mailPage(++version, dto) });
  };
  const editingRaw = () => { const id = editingId(); return id ? doc.byId[id]?.raw : null; };

  it("applies an external change (host mail) and reopens the editor at the same outline position", async () => {
    await open(["a", "b"], 1);
    pushed(["a", "b on disk"]);
    expect(raws("P")).toEqual(["a", "b on disk"]);
    expect(editingRaw()).toBe("b on disk");
  });

  it("navigation to the same page keeps the host's text and the editor on its block", async () => {
    await open(["a", "b"], 1);
    loadSingle(page("P", ["a", "b on disk"]), { endEdit: false });
    expect(raws("P")).toEqual(["a", "b"]);
    expect(editingRaw()).toBe("b");
  });

  it("follows a block's id:: when the outline around it changed", async () => {
    await open(["a", `b\nid:: ${ID}`], 1);
    pushed([`b\nid:: ${ID}`, "new", "a"]);
    expect(raws("P")).toEqual([`b\nid:: ${ID}`, "new", "a"]);
    expect(editingId()).toBe(pageByName("P")!.roots[0]);
  });

  it("closes the editor when its block is gone", async () => {
    await open(["a", `b\nid:: ${ID}`], 1);
    pushed(["a"]);
    expect(raws("P")).toEqual(["a"]);
    expect(editingId()).toBeNull();
  });

  it("still waits for an IME composition (the page is pinned)", async () => {
    await open(["a", "b"], 1);
    unpin = pinPageWhileDrafting(() => "P");
    expect(reloadPageIfStillSafe("P", page("P", ["a", "b on disk"]))).toBe(false);
    pushed(["a", "b on disk"]);
    expect(raws("P")).toEqual(["a", "b"]);
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
