// The open-threads review item (DESIGN §12): its query text, its sidebar identity, and that it
// survives the session restore path without disturbing an older or newer reader.
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { buildPersistedSession, parsePersistedSession } from "./session";
import { resetPaneLayoutToSingle } from "./panes";
import { resetWorkspacesForTest } from "./workspaces";
import { applySidebarSession, parseStoredSidebarItems, rightSidebar, rightSidebarOpen, setRightSidebar, sidebarItemKey, type SidebarItem, type SidebarQuery } from "./ui";
import { openCommentReviewInSidebar, openThreadsQuery, setCommentReviewAuthor } from "./commentReviewQuery";

const fixture = JSON.parse(readFileSync(new URL("../crates/tine-store/tests/fixtures/comment-review-queries.json", import.meta.url), "utf8")) as {
  cases: { scope: "page" | "graph"; page: string; author: string | null; query: string }[];
};

beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => void store.set(key, value),
    removeItem: (key: string) => void store.delete(key), clear: () => store.clear(),
  });
  resetWorkspacesForTest();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0, scrolls: [0] });
  applySidebarSession({ right: false, items: [] });
});

const review = (over: Partial<SidebarQuery> = {}): SidebarQuery => ({ kind: "query", scope: "graph", page: "", pageKind: "page", ...over });

describe("the review query text", () => {
  it.each(fixture.cases.map((c) => [`${c.scope}${c.author ? ` by ${c.author}` : ""} on ${c.page || "the graph"}`, c] as const))(
    "prints the query the engine test runs: %s", (_label, c) => {
      expect(openThreadsQuery({ scope: c.scope, page: c.page, ...(c.author ? { author: c.author } : {}) })).toBe(c.query);
    });

  it("asks for thread roots that are not resolved, and nothing else about comments", () => {
    const text = openThreadsQuery({ scope: "graph", page: "" });
    expect(text).toContain("(property tine.quote)");
    expect(text).toContain("(not (property tine.resolved true))");
    expect(text).not.toMatch(/tine\.(date|author)/);
  });
});

describe("the review item in the sidebar", () => {
  it("has one slot per scope and the page scope follows its page", () => {
    expect(sidebarItemKey(review())).toBe("query:graph");
    expect(sidebarItemKey(review({ scope: "page", page: "Notes", pageKind: "page" }))).toBe("query:page:Notes");
    expect(sidebarItemKey(review({ author: "Ada" }))).toBe(sidebarItemKey(review()));
  });

  it("opens the sidebar, promotes an existing review and keeps its author filter", () => {
    setRightSidebar([{ kind: "page", name: "Other", pageKind: "page" }, review({ author: "Ada", collapsed: true })]);
    openCommentReviewInSidebar("graph");
    expect(rightSidebarOpen()).toBe(true);
    expect(rightSidebar().map(sidebarItemKey)).toEqual(["query:graph", "page:page:Other"]);
    expect(rightSidebar()[0]).toMatchObject({ author: "Ada", collapsed: false });
    openCommentReviewInSidebar("page", { name: "Notes", pageKind: "page", path: "pages/Notes.md" });
    expect(rightSidebar().map(sidebarItemKey)).toEqual(["query:page:Notes", "query:graph", "page:page:Other"]);
  });

  it("sets and clears the author filter in place", () => {
    openCommentReviewInSidebar("graph");
    setCommentReviewAuthor(rightSidebar()[0] as SidebarQuery, "  Ada ");
    expect((rightSidebar()[0] as SidebarQuery).author).toBe("Ada");
    setCommentReviewAuthor(rightSidebar()[0] as SidebarQuery, "");
    expect("author" in rightSidebar()[0]).toBe(false);
  });
});

describe("session restore", () => {
  const items = (): SidebarItem[] => [
    review({ scope: "page", page: "Notes", pageKind: "page", path: "pages/Notes.md", author: "Ada" }),
    review({ collapsed: true }),
    { kind: "page", name: "Other", pageKind: "page" },
  ];

  it("round-trips both scopes through the persisted session", () => {
    setRightSidebar(items());
    const persisted = buildPersistedSession();
    expect(persisted.rightSidebarItems).toEqual(items());
    const parsed = parsePersistedSession(JSON.stringify(persisted))!;
    setRightSidebar([]);
    applySidebarSession(parsed.sidebar);
    expect(rightSidebar()).toEqual(items());
  });

  it("round-trips through the localStorage copy", () => {
    setRightSidebar(items());
    expect(parseStoredSidebarItems(localStorage.getItem("logseq-claude.rightSidebarItems"))).toEqual(items());
  });

  it("drops a kind this build does not know and restores the rest (a session written by a newer version)", () => {
    const stored = JSON.stringify([{ kind: "timeline", scope: "page" }, ...items(), { kind: "query", scope: "everything", page: "", pageKind: "page" }]);
    expect(parseStoredSidebarItems(stored)).toEqual(items());
    const session = JSON.parse(JSON.stringify(buildPersistedSession())) as { rightSidebarItems?: unknown[] };
    session.rightSidebarItems = [{ kind: "timeline" }, ...items()];
    applySidebarSession(parsePersistedSession(JSON.stringify(session))!.sidebar);
    expect(rightSidebar()).toEqual(items());
  });

  it("refuses malformed review items rather than rendering them", () => {
    for (const bad of [
      { kind: "query", scope: "page", page: "", pageKind: "page" },
      { kind: "query", scope: "graph", page: "", pageKind: "other" },
      { kind: "query", scope: "graph", page: "", pageKind: "page", author: "" },
      { kind: "query", scope: "graph", page: 4, pageKind: "page" },
    ]) expect(parseStoredSidebarItems(JSON.stringify([bad]))).toEqual([]);
  });
});
