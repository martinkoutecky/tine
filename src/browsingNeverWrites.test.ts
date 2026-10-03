// Browsing never mutates the graph (GH #623, invariants I-2, I-21; OG parity).
// OG writes `id::` only when a REFERENCE is made (editor.cljs set-blocks-id!
// callers: copy-block-ref!/copy-block-embed!, `((` autocomplete, import, plugin API).
// Zoom, Ctrl-K, sidebar, new tab, pane open and session restart must write no byte;
// a zoom that must survive a restart is saved by position path and restored from it.
// Exemplar for a zero-write navigation test: this file.
import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { blockPositionRef, blockRef, ensureBlockId, isDirty, resetStore } from "./document";
import { doc, setDoc } from "./document/model";
import { openDurableBlock } from "./blockRefActions";
import { activeTab, focusBlock, openInNewTab, openPage, resetTabsToJournals, resolveRouteBlock, route, settleActiveBlock } from "./router";
import { applyParsedSession, buildPersistedSession, parsePersistedSession } from "./session";
import { applySidebarSession, rightSidebar } from "./ui";

const RAW = { a: "alpha", b: "beta", c: "gamma (child of beta)" } as const;

/** Load page "Notes": alpha, beta -> gamma; ID-less blocks keyed with `prefix`. */
function loadNotes(prefix: string, opts: { withBeta?: boolean; betaRaw?: string } = {}) {
  const a = `${prefix}-a`, b = `${prefix}-b`, c = `${prefix}-c`;
  const withBeta = opts.withBeta ?? true;
  setDoc({
    byId: {
      [a]: { id: a, raw: RAW.a, collapsed: false, parent: null, page: "Notes", children: [] },
      ...(withBeta ? {
        [b]: { id: b, raw: opts.betaRaw ?? RAW.b, collapsed: false, parent: null, page: "Notes", children: [c] },
        [c]: { id: c, raw: RAW.c, collapsed: false, parent: b, page: "Notes", children: [] },
      } : {}),
    },
    pages: [{
      name: "Notes", kind: "page", title: "Notes", preBlock: null,
      roots: withBeta ? [a, b] : [a], format: "md", readOnly: false, guide: false, id: "pages/Notes.md",
    }],
    feed: ["Notes"],
    loaded: true,
  });
  return { a, b, c };
}

const rawOf = () => Object.fromEntries(Object.entries(doc.byId).map(([k, v]) => [k, v.raw]));

beforeEach(() => {
  resetStore();
  resetTabsToJournals();
  applySidebarSession({});
  vi.restoreAllMocks();
});

describe("navigation writes no byte", () => {
  it("zoom, new tab, other pane and sidebar open stamp nothing and mint no id", async () => {
    const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["rev"] });
    const random = vi.spyOn(crypto, "randomUUID");
    const { c } = loadNotes("k");
    const before = rawOf();

    openPage("Notes", "page");
    focusBlock(c);
    openDurableBlock(c, "tab");
    openDurableBlock(c, "sidebar");
    openDurableBlock(c, "pane");
    openInNewTab({ kind: "page", name: "Notes", pageKind: "page", block: c });
    await Promise.resolve();

    expect(rawOf()).toEqual(before);
    expect(isDirty("Notes")).toBe(false);
    expect(save).not.toHaveBeenCalled();
    expect(random).not.toHaveBeenCalled();
    expect(rightSidebar()[0]).toMatchObject({ kind: "block", page: "Notes" });
  });

  it("saving the session for a zoomed route writes only the session, never the page", async () => {
    const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["rev"] });
    const { c } = loadNotes("k");
    openPage("Notes", "page");
    focusBlock(c);
    const before = rawOf();
    const persisted = buildPersistedSession();
    JSON.stringify(persisted);
    expect(rawOf()).toEqual(before);
    expect(save).not.toHaveBeenCalled();
  });
});

describe("a zoom survives a restart without an id::", () => {
  it("stores the position path and restores to the same block under fresh runtime keys", () => {
    const { c } = loadNotes("old");
    openPage("Notes", "page");
    focusBlock(c);
    const saved = buildPersistedSession();
    const json = JSON.stringify(saved);
    const savedRoute = saved.tabs[saved.activeIndex].history[saved.tabs[saved.activeIndex].pos];
    expect(savedRoute).toMatchObject({ kind: "page", name: "Notes", blockPos: [1, 0] });

    // "Restart": every runtime key is new (the stale one must not be trusted).
    resetStore();
    const fresh = loadNotes("new");
    const parsed = parsePersistedSession(json);
    expect(parsed).not.toBeNull();
    applyParsedSession(parsed!);

    expect(resolveRouteBlock(route())).toBe(fresh.c);
    settleActiveBlock();
    expect(route()).toMatchObject({ kind: "page", name: "Notes", block: fresh.c });
    expect(route()).not.toHaveProperty("blockPos");
    expect(rawOf()).toEqual({ "new-a": RAW.a, "new-b": RAW.b, "new-c": RAW.c });
  });

  it("falls back to the page top when the saved position no longer exists", () => {
    const { c } = loadNotes("old");
    openPage("Notes", "page");
    focusBlock(c);
    const json = JSON.stringify(buildPersistedSession());

    resetStore();
    loadNotes("new", { withBeta: false }); // the page lost the zoomed block while Tine was closed
    applyParsedSession(parsePersistedSession(json)!);

    expect(resolveRouteBlock(route())).toBeNull();
    expect(settleActiveBlock).not.toThrow();
    expect(activeTab()).toBeTruthy();
  });

  it("rejects a malformed saved position instead of trusting it", () => {
    const { c } = loadNotes("old");
    openPage("Notes", "page");
    focusBlock(c);
    const json = JSON.stringify(buildPersistedSession());
    expect(json).toContain('"blockPos":[1,0]');
    // Every copy of the position (legacy tabs and layout snapshots) is corrupted.
    const parsed = parsePersistedSession(json.split('"blockPos":[1,0]').join('"blockPos":[-1,"x"]'));
    const restored = [...(parsed?.snapshots.values() ?? [])].flatMap((snap) => snap.tabs.flatMap((t) => t.history));
    expect(JSON.stringify(restored)).not.toContain("blockPos");
    expect(JSON.stringify(restored)).not.toContain("[-1");
  });

  it("a block opened in the sidebar is saved by position and restored", () => {
    const { c } = loadNotes("old");
    openDurableBlock(c, "sidebar");
    const saved = buildPersistedSession();
    expect(saved.rightSidebarItems?.[0]).toMatchObject({ kind: "block", page: "Notes", blockPos: [1, 0] });
    const json = JSON.stringify(saved);

    resetStore();
    applySidebarSession({});
    loadNotes("new");
    applyParsedSession(parsePersistedSession(json)!);
    expect(rightSidebar()[0]).toMatchObject({ kind: "block", page: "Notes", blockPos: [1, 0] });
  });
});

describe("a reference still writes id::", () => {
  it("zoom, then Copy block ref: the id is stamped once, the zoom keeps showing the block, and it is saved by id", async () => {
    const uuid = "12345678-1234-4234-8234-123456789abc";
    vi.spyOn(crypto, "randomUUID").mockReturnValue(uuid);
    const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["rev"] });
    const { c } = loadNotes("k");
    openPage("Notes", "page");
    focusBlock(c);
    expect(save).not.toHaveBeenCalled();

    expect(await ensureBlockId(c)).toBe(uuid);
    expect(doc.byId[c].raw).toBe(`${RAW.c}\nid:: ${uuid}`);
    expect(save).toHaveBeenCalledTimes(1);

    // The zoomed route opened by runtime key keeps resolving after the stamp (GH #373 rule relaxed for navigation only).
    expect(resolveRouteBlock(route())).toBe(c);
    // From now on the saved session names the block by its authored id, not by position.
    const savedRoute = buildPersistedSession().tabs[0].history.at(-1);
    expect(savedRoute).toMatchObject({ kind: "page", name: "Notes", block: uuid });
    expect(savedRoute).not.toHaveProperty("blockPos");
    expect(blockPositionRef(blockRef(c))).toMatchObject({ uuid });
  });
});

describe("only reference creation may stamp an id:: (guard, I-2, I-21)", () => {
  it("ensureBlockId / persistBlockRefTarget are used by reference creators only", () => {
    const stamping = /\b(ensureBlockId|persistBlockRefTarget)\b/;
    const walk = (dir: string): string[] => readdirSync(dir).flatMap((name) => {
      const full = path.join(dir, name);
      return statSync(full).isDirectory() ? walk(full) : /\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name) ? [full] : [];
    });
    const users = walk(path.join(__dirname)).filter((f) => stamping.test(readFileSync(f, "utf8"))).map((f) => path.relative(__dirname, f).split(path.sep).join("/")).sort();
    expect(
      users,
      "RULE (I-2/I-21): browsing never writes an id:: (OG writes it only when a reference is made: editor.cljs set-blocks-id! callers). "
      + "Only copy-ref/embed (ContextMenu, blockLinkCopy) and `((` autocomplete (Block) may stamp. Navigation must open by runtime key / blockPos "
      + "like src/blockRefActions.ts openDurableBlock; do not add a stamping call to a navigation path.",
    ).toEqual([
      "components/Block.tsx",
      "components/ContextMenu.tsx",
      "components/blockLinkCopy.ts",
      "document/edits/identity.ts",
      "document/index.ts",
    ]);
  });
});
