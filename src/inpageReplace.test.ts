// @vitest-environment jsdom
// GH #208 / #338: in-page find & replace. The plan is pure (what one block becomes);
// the entry points write through setRaw inside one withUndoUnit and the page
// host's ordinary save. UI-INPAGE-FIND-REPLACE-208.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "./render/parse";
import { backend, type GraphChange } from "./backend";
import { applyGraphChange, ensurePageLoaded, flushPage, isConflicted, pageByName, resetStore, undo } from "./document";
import { doc } from "./document/model";
import { closeInPageFind, inPageFindActiveIndex, inPageFindMatches, openInPageFind, setInPageFindQuery, clearInPageFindRenderedTextCacheForTests } from "./inpageFind";
import { planBlockReplacement, replaceActiveInPageFindMatch, replaceAllInPageFindMatches } from "./inpageReplace";
import { resetPaneLayoutToSingle } from "./panes";
import { closeAllRightSidebarItems, openPageInSidebar } from "./ui";
import { setToasts, toasts } from "./toasts";
import type { PaneSnapshot } from "./router";
import type { BlockDto, PageDto } from "./types";
import { answerOpensFromDocument } from "./document/host/documentHost.test.support";
import { bindTestHost, submittedPages, type TestHost } from "./document/host/wiring.test.support";

const ID = "6501f00a-0000-4000-8000-000000000000";
const plan = (raw: string, query: string, replacement: string, ordinals: readonly number[] | "all" = "all") =>
  planBlockReplacement(raw, "md", query, replacement, ordinals, true);

beforeAll(async () => { await initParser(); });

describe("planBlockReplacement: only what find shows, only where the editor edits", () => {
  it("replaces visible text and a link label but never the link target", () => {
    // A raw-text replace would rewrite https://foo.com as well.
    expect(plan("foo [label foo](https://foo.com) foo", "foo", "bar"))
      .toEqual({ raw: "bar [label bar](https://foo.com) bar", replaced: 3, skipped: 0 });
  });

  it("never rewrites a hidden property line", () => {
    const raw = `true story\ncollapsed:: true\nid:: ${ID}`;
    expect(plan(raw, "true", "false")).toEqual({ raw: `false story\ncollapsed:: true\nid:: ${ID}`, replaced: 1, skipped: 0 });
    expect(plan(raw, "6501f00a", "x").replaced).toBe(0);
  });

  it("replaces only the requested occurrence", () => {
    expect(plan("foo foo foo", "foo", "bar", [1]).raw).toBe("foo bar foo");
  });

  it("matches as find does (case and accents folded) and keeps the markup around a match", () => {
    expect(plan("**Foo** foo café", "foo", "bar").raw).toBe("**bar** bar café");
    expect(plan("café", "cafe", "tea").raw).toBe("tea");
  });

  it("proves the occurrence instead of trusting the first equal letters", () => {
    // Rendered "xfoo": an order-only alignment lands on the link target "foo".
    expect(plan("[x](foo)foo", "foo", "bar")).toEqual({ raw: "[x](foo)bar", replaced: 1, skipped: 0 });
  });

  it("an empty replacement deletes the match", () => {
    expect(plan("a-drop-b", "drop", "").raw).toBe("a--b");
  });
});

let serial = 0;
const block = (raw: string, children: BlockDto[] = []): BlockDto => ({ id: `rp-${++serial}`, raw, collapsed: false, children });
const pageDto = (name: string, blocks: BlockDto[], extra: Pick<PageDto, "read_only"> = {}): PageDto & { id: string; rev: string } => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev: `rev-${++serial}`, blocks, ...extra,
});
const raws = (name: string) => {
  const out: string[] = [];
  const walk = (ids: readonly string[]) => { for (const id of ids) { out.push(doc.byId[id].raw); walk(doc.byId[id].children); } };
  walk(pageByName(name)?.roots ?? []);
  return out;
};
const pageSnapshot = (name: string): PaneSnapshot => ({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
const journalsSnapshot = (): PaneSnapshot => ({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });

let host: TestHost;
let disk: Record<string, PageDto>;
beforeEach(async () => {
  serial = 0; resetStore(); setToasts([]); disk = {};
  host = await bindTestHost();
  answerOpensFromDocument(host);
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => (disk[name] ?? null) as never);
});
afterEach(() => {
  setInPageFindQuery("");
  closeInPageFind({ restoreFocus: false });
  closeAllRightSidebarItems();
  clearInPageFindRenderedTextCacheForTests();
  resetStore();
  resetPaneLayoutToSingle(journalsSnapshot());
  vi.restoreAllMocks();
});

function findOn(name: string, query: string) {
  resetPaneLayoutToSingle(pageSnapshot(name));
  openInPageFind();
  setInPageFindQuery(query);
}

describe("replace through the find panel's state", () => {
  it("replace all edits only the find pane's page, as one undo step", () => {
    ensurePageLoaded(pageDto("Notes", [block("foo one", [block("two foo foo")]), block("nothing")]));
    ensurePageLoaded(pageDto("Side", [block("foo in the sidebar")]));
    openPageInSidebar("Side", "page");
    findOn("Notes", "foo");
    expect(replaceAllInPageFindMatches("bar")).toEqual({ replaced: 3, skipped: 0 });
    expect(raws("Notes")).toEqual(["bar one", "two bar bar", "nothing"]);
    expect(raws("Side")).toEqual(["foo in the sidebar"]);
    expect(inPageFindMatches()).toEqual([]);
    expect(undo()).toBe(true);
    expect(raws("Notes")).toEqual(["foo one", "two foo foo", "nothing"]);
  });

  it("replace edits the active match and moves on, even when the replacement contains the query", () => {
    ensurePageLoaded(pageDto("Notes", [block("foo foo"), block("foo")]));
    findOn("Notes", "foo");
    expect(inPageFindActiveIndex()).toBe(0);
    replaceActiveInPageFindMatch("foo!");
    expect(raws("Notes")).toEqual(["foo! foo", "foo"]);
    expect(inPageFindActiveIndex()).toBe(1);
    replaceActiveInPageFindMatch("foo!");
    expect(raws("Notes")).toEqual(["foo! foo!", "foo"]);
    expect(inPageFindActiveIndex()).toBe(2);
    expect(undo()).toBe(true);
    expect(raws("Notes")).toEqual(["foo! foo", "foo"]);
  });

  it("a read-only page is never written and says so", () => {
    ensurePageLoaded(pageDto("Locked", [block("foo")], { read_only: true }));
    expect(pageByName("Locked")!.readOnly).toBe(true);
    findOn("Locked", "foo");
    expect(replaceAllInPageFindMatches("bar").replaced).toBe(0);
    expect(raws("Locked")).toEqual(["foo"]);
    expect(toasts().some((t) => t.kind === "error")).toBe(true);
  });
});

describe("data safety: the ordinary save, an external change, a conflict", () => {
  it("saves the replaced text through the page host's submit", async () => {
    const submit = vi.spyOn(backend(), "pageSubmit").mockImplementation(async () => null);
    ensurePageLoaded(pageDto("Notes", [block("foo one"), block("two foo")]));
    findOn("Notes", "foo");
    replaceAllInPageFindMatches("bar");
    void flushPage("Notes");
    await vi.waitFor(() => expect(submittedPages(submit, "Notes")).toHaveLength(1));
    expect(submittedPages(submit, "Notes")[0].blocks.map((b) => b.raw)).toEqual(["bar one", "two bar"]);
  });

  it("an external change that lands between find and replace is the text replace edits", async () => {
    ensurePageLoaded(pageDto("Notes", [block("foo one"), block("two foo")]));
    findOn("Notes", "foo");
    expect(inPageFindMatches()).toHaveLength(2);
    disk.Notes = pageDto("Notes", [block("foo one, edited elsewhere"), block("two foo"), block("three foo, added elsewhere")]);
    const change: GraphChange = { name: "Notes", kind: "page", created: false, removed: false };
    await applyGraphChange(change);
    await vi.waitFor(() => expect(raws("Notes")).toHaveLength(3));
    expect(replaceAllInPageFindMatches("bar").replaced).toBe(3);
    expect(raws("Notes")).toEqual(["bar one, edited elsewhere", "two bar", "three bar, added elsewhere"]);
  });

  it("a disk change under an unsaved replace becomes a conflict, never a silent loss", async () => {
    const admitted: Array<{ id: number; key: string }> = [];
    vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key) => { admitted.push({ id, key }); return null; });
    ensurePageLoaded(pageDto("Notes", [block("foo one")]));
    findOn("Notes", "foo");
    replaceAllInPageFindMatches("bar");
    void flushPage("Notes");
    await vi.waitFor(() => expect(admitted).toHaveLength(1));
    const { id, key } = admitted[0];
    host.deliver({ key, answer: { id, version: 50, took: true, outcome: { kind: "applied" } }, notice: { conflictReported: true },
      page: { version: 50, conflict: true, risk: false, disk: { kind: "file", rev: "theirs-rev" }, text: { kind: "unchanged" } } });
    expect(isConflicted("Notes")).toBe(true);
    expect(raws("Notes")).toEqual(["bar one"]);
  });
});
