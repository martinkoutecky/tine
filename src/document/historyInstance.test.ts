// GH #305 (master 6a6f955f7): an undo/redo entry describes ONE loaded instance of
// each page it touches. Eviction keeps history, and re-opening the page installs a
// fresh instance carrying whatever the file says now; replaying the old entry would
// restore pre-eviction content and save it under the fresh baseline, which the
// base-revision guard accepts. These tests drive the real undo()/redo() entry points.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { deleteBlock, ensurePageLoaded, flushAll, isDirty, loadFeed, pageByName, redo, resetStore, setRaw, splitBlock, undo } from "./index";
import { doc } from "./model";
import { setToasts, toasts } from "../toasts";
import type { BlockDto, PageDto } from "../types";
import { answerOpensFromDocument } from "./host/documentHost.test.support";
import { bindTestHost } from "./host/wiring.test.support";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `hi-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[], rev = `r1-${name}`): PageDto & { id: string; rev: string } => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev, blocks: raws.map(block),
});
const raws = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];

/** Push `name` out of the working set by loading more than the cap of other pages. */
function evict(name: string) {
  for (let i = 0; i < 90; i++) ensurePageLoaded(page(`Filler ${i}`, ["x"]));
  expect(pageByName(name)).toBeUndefined();
}

beforeAll(() => initParser());
beforeEach(async () => {
  serial = 0;
  resetStore();
  setToasts([]);
  answerOpensFromDocument(await bindTestHost());
});
afterEach(() => vi.restoreAllMocks());

describe("undo refuses an entry whose page instance is gone (GH #305)", () => {
  it("a structural edit's undo after eviction + external change does not overwrite the file", async () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["keep me", "delete me"]));
    deleteBlock(pageByName("P")!.roots[1]);
    expect(raws("P")).toEqual(["keep me"]);
    expect(await flushAll()).toBe(true);
    evict("P");
    // The file changed while the page was out of memory; re-opening reads it fresh.
    ensurePageLoaded(page("P", ["changed elsewhere"], "r2-P"));
    undo();
    expect(raws("P")).toEqual(["changed elsewhere"]);
    expect(isDirty("P")).toBe(false);
    expect(toasts().some((t) => /Undo history for this page was discarded/.test(t.message))).toBe(true);
  });

  it("a typing entry's undo after eviction is refused too", async () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["first"]));
    setRaw(pageByName("P")!.roots[0], "typed");
    expect(await flushAll()).toBe(true);
    evict("P");
    ensurePageLoaded(page("P", ["external"], "r2-P"));
    undo();
    expect(raws("P")).toEqual(["external"]);
    expect(isDirty("P")).toBe(false);
  });

  it("redo is refused the same way", async () => {
    loadFeed([page("Journal", ["j"])]);
    ensurePageLoaded(page("P", ["ab"]));
    splitBlock(pageByName("P")!.roots[0], 1);
    undo();
    expect(raws("P")).toEqual(["ab"]);
    expect(await flushAll()).toBe(true);
    evict("P");
    ensurePageLoaded(page("P", ["external"], "r2-P"));
    redo();
    expect(raws("P")).toEqual(["external"]);
    expect(isDirty("P")).toBe(false);
  });

  it("an entry on the same live instance still undoes (no false refusal)", () => {
    loadFeed([page("P", ["keep me", "delete me"])]);
    deleteBlock(pageByName("P")!.roots[1]);
    undo();
    expect(raws("P")).toEqual(["keep me", "delete me"]);
    expect(toasts()).toEqual([]);
  });

  it("an older entry for another page stays usable after one page's history is discarded", async () => {
    loadFeed([page("Journal", ["j"])]);
    setRaw(pageByName("Journal")!.roots[0], "journal typed");
    ensurePageLoaded(page("P", ["p"]));
    setRaw(pageByName("P")!.roots[0], "p typed");
    expect(await flushAll()).toBe(true);
    evict("P");
    ensurePageLoaded(page("P", ["external"], "r2-P"));
    undo(); // refused: P's instance is gone
    undo(); // Journal is still the same instance
    expect(raws("Journal")).toEqual(["j"]);
    expect(raws("P")).toEqual(["external"]);
  });
});
