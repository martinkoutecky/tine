import { afterEach, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { backend } from "./backend";
import { paneRouter, resetPaneLayoutToSingle } from "./panes";
import { refreshAfterRename } from "./graph";
import { applyOwnerPageRename, followRename, renamePageOnDisk } from "./document/graphRewrite";
import { graphEpoch, pageInventoryRev } from "./graphSession";
import { fullChangeKeyFlushCount } from "./changeKeys";
import { setDoc } from "./document/model";
import { resetStore } from "./document/workingSet";
import { recentPages, rightSidebar, setRecentPages, setRightSidebar } from "./ui";
import type { RenameDone, RenamedPage } from "./types";

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setRecentPages([]);
  setRightSidebar([]);
  resetPaneLayoutToSingle();
});

const moved = (path: string) => ({ path, moved: true });
const renamed = (from: string, to: string, path: string | null): RenamedPage => ({ from, to, path });
/** The owner's notification of a CLI rename of `old` (file `path`) to `name`. */
const cliRename = (old: string, name: string, path: string | null, report: RenameDone) =>
  applyOwnerPageRename({ old, name, path, kind: "page", binding_generation: 1, report });
const oldToNew: RenameDone = { outcome: "renamed", touched: [moved("pages/Old.md")], renamed: [renamed("Old", "New", "pages/Old.md")] };

it.each([undefined, "pages/Old.md"])("keeps an open source route on the CLI rename destination (path %s)", async (path) => {
  resetPaneLayoutToSingle({
    tabs: [{ history: [
      { kind: "page", name: "Previous", pageKind: "page" },
      { kind: "page", name: "Old", pageKind: "page", ...(path ? { path } : {}) },
    ], pos: 1, pinned: false }], activeIndex: 0,
  });
  await cliRename("Old", "New", "pages/Old.md", oldToNew);
  expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "New", pageKind: "page" });
  expect(paneRouter("main").tabs()[0].history[0]).toMatchObject({ name: "Previous" });
});

it("keeps an open referrer selected when the CLI renames its target", async () => {
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "page", name: "Referrer", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0,
  });
  await cliRename("Old", "New", "pages/Old.md", oldToNew);
  expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Referrer" });
});

it("follows a fileless source using the owner's null-path notification", async () => {
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "page", name: "Ghost", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0,
  });
  await cliRename("Ghost", "NewGhost", null, { outcome: "renamed", touched: [], renamed: [renamed("Ghost", "NewGhost", null)] });
  expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "NewGhost" });
});

// D-20261102-PERF row 3: the rename's publication carries a precise change
// summary (both names, the moved page's blocks, the rewritten referrers;
// `crates/tine-store/tests/page_lifecycle_summary.rs`). A display-epoch bump
// here re-subscribed every mounted query and reference panel; only the page
// index is refetched now.
it("refreshes the page index but no other reader after a rename", () => {
  const [epoch, inventory, full] = [graphEpoch(), pageInventoryRev(), fullChangeKeyFlushCount()];
  refreshAfterRename(followRename(oldToNew.renamed));
  expect(pageInventoryRev()).toBeGreaterThan(inventory);
  expect(graphEpoch()).toBe(epoch);
  expect(fullChangeKeyFlushCount()).toBe(full);
});

// small-1109c r2: renaming `Foo` into a free name inside its own namespace
// (allowed, pages_rename_plan.rs `own_child_used`) moves one page, once. Two
// passes (the exact target, then a namespace follower over its result) sent
// every view to `Foo/bar/bar`.
const fooReport: RenameDone = { outcome: "renamed", touched: [moved("pages/Foo.md")], renamed: [renamed("Foo", "Foo/bar", "pages/Foo.md")] };

function openFoo() {
  resetPaneLayoutToSingle({
    tabs: [{ history: [page("Previous"), page("Foo", "pages/Foo.md")], pos: 1, pinned: false }], activeIndex: 0,
  });
  paneRouter("side").restoreSnapshot({ tabs: [{ history: [page("Foo")], pos: 0, pinned: false }], activeIndex: 0 });
  setRightSidebar([{ kind: "page", name: "Foo", pageKind: "page", collapsed: false }]);
  setRecentPages([{ name: "Foo", kind: "page" }, { name: "Elsewhere", kind: "page" }]);
}

function expectFooOnceMoved() {
  expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Foo/bar" });
  expect(paneRouter("side").route()).toMatchObject({ kind: "page", name: "Foo/bar" });
  expect(rightSidebar().map((item) => item.kind === "page" ? item.name : item.page)).toEqual(["Foo/bar"]);
  const recent = recentPages().map((item) => item.name);
  expect(recent).toEqual(expect.arrayContaining(["Foo/bar", "Elsewhere"]));
  // Recent also holds whatever a pane just showed; nothing keeps the old name or moved twice.
  expect(recent.filter((name) => name === "Foo" || name.startsWith("Foo/bar/"))).toEqual([]);
}

it("renames Foo to Foo/bar once in every pane, the sidebar and Recent (CLI rename)", async () => {
  openFoo();
  await cliRename("Foo", "Foo/bar", "pages/Foo.md", fooReport);
  expectFooOnceMoved();
});

it("renames Foo to Foo/bar once from the app's rename dialog, which reopens the page itself", async () => {
  openFoo();
  vi.spyOn(backend(), "renamePage").mockResolvedValueOnce(fooReport);
  // As the page title's dialog does (Page.tsx): reopen the new name in its pane.
  expect(await renamePageOnDisk("Foo", "Foo/bar", { name: "Foo", pageKind: "page", path: "pages/Foo.md" }, undefined,
    () => paneRouter("main").openPage("Foo/bar", "page"))).toBe("renamed");
  expectFooOnceMoved();
});

// Card HDnCws (CLI slice 3 review r1 R5): a rename moves the file-backed
// namespace children with their parent, and every view of a moved child
// follows it, whoever started the rename. The backend reports each identity it
// moved (`report.renamed`); the views follow exactly that map.
const nsReport: RenameDone = {
  outcome: "renamed",
  touched: [moved("pages/Old.md"), moved("pages/Old___child.md"), moved("pages/Old___CHILD___leaf.md"), { path: "pages/Refs.md", moved: false }],
  renamed: [
    renamed("Old", "New", "pages/Old.md"),
    renamed("Old/child", "New/child", "pages/Old___child.md"),
    renamed("Old/CHILD/leaf", "New/CHILD/leaf", "pages/Old___CHILD___leaf.md"),
  ],
};
const page = (name: string, path?: string) => ({ kind: "page" as const, name, pageKind: "page" as const, ...(path ? { path } : {}) });

/** Panes on the children and unrelated pages in two panes, the sidebar and Recent. */
function openChildren() {
  resetPaneLayoutToSingle({
    tabs: [{ history: [page("Previous"), page("Old/child", "pages/Old___child.md")], pos: 1, pinned: false }], activeIndex: 0,
  });
  paneRouter("side").restoreSnapshot({
    tabs: [{ history: [page("Older/child"), page("old/CHILD/leaf")], pos: 1, pinned: false }], activeIndex: 0,
  });
  setRightSidebar([
    { kind: "page", name: "Old/child", pageKind: "page", collapsed: false },
    { kind: "block", uuid: "u1", page: "Old/child/leaf", pageKind: "page", collapsed: false },
    { kind: "page", name: "Elsewhere/child", pageKind: "page", collapsed: false },
  ]);
  setRecentPages([{ name: "Old/child", kind: "page" }, { name: "Other/child", kind: "page" }]);
}

function expectChildrenFollowed() {
  const main = paneRouter("main").tabs()[0].history;
  expect(main.map((r) => r.kind === "page" ? [r.name, r.path] : null)).toEqual([["Previous", undefined], ["New/child", undefined]]);
  const side = paneRouter("side").tabs()[0].history;
  // A sibling that merely shares a prefix with the renamed page is not its
  // child; a view in another spelling of a moved page takes the name the
  // backend wrote.
  expect(side.map((r) => r.kind === "page" ? r.name : null)).toEqual(["Older/child", "New/CHILD/leaf"]);
  expect(rightSidebar().map((item) => item.kind === "page" ? item.name : item.page))
    .toEqual(["New/child", "New/CHILD/leaf", "Elsewhere/child"]);
  // Recent also holds whatever a pane just showed; none of it keeps the old name.
  const recent = recentPages().map((item) => item.name);
  expect(recent).toEqual(expect.arrayContaining(["New/child", "Other/child"]));
  expect(recent.filter((name) => /^old\//i.test(name))).toEqual([]);
}

it("moves a pane, a split pane, the sidebar and Recent from a namespace child to its new name (CLI rename)", async () => {
  openChildren();
  await cliRename("Old", "New", "pages/Old.md", nsReport);
  expectChildrenFollowed();
});

it("moves them the same way when the app's own rename dialog renames the parent", async () => {
  openChildren();
  vi.spyOn(backend(), "renamePage").mockResolvedValueOnce(nsReport);
  expect(await renamePageOnDisk("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("renamed");
  expectChildrenFollowed();
});

it("moves reference-only children the rename renamed, loaded or not, in Recent, the sidebar and closed-tab history; an untouched sibling stays", async () => {
  // `Old/ghost` exists only through references and is not loaded; `Old/kept`
  // is loaded without a file of its own. The backend rewrote every reference
  // to both, and its report lists them (`path: null`). `Older/x` is no member.
  resetPaneLayoutToSingle({
    tabs: [
      { history: [page("Old/kept")], pos: 0, pinned: false },
      { history: [page("Previous"), page("Old/ghost")], pos: 1, pinned: false },
    ], activeIndex: 1,
  });
  const main = paneRouter("main");
  await main.closeTab(main.tabs()[1].id);
  setRightSidebar([
    { kind: "page", name: "Old/ghost", pageKind: "page", collapsed: false },
    { kind: "page", name: "Older/x", pageKind: "page", collapsed: false },
    { kind: "page", name: "Old/kept", pageKind: "page", collapsed: false },
  ]);
  setRecentPages([{ name: "Old/ghost", kind: "page" }, { name: "Older/x", kind: "page" }]);
  setDoc({ byId: {}, feed: [], loaded: true, pages: [
    { name: "Old/kept", id: undefined, kind: "page", title: "Old/kept", preBlock: null, roots: [], format: "md", readOnly: false, guide: false },
  ] } as never);
  await cliRename("Old", "New", "pages/Old.md", {
    outcome: "renamed",
    touched: [moved("pages/Old.md"), { path: "pages/Refs.md", moved: false }],
    renamed: [renamed("Old", "New", "pages/Old.md"), renamed("Old/ghost", "New/ghost", null), renamed("Old/kept", "New/kept", null)],
  });
  expect(main.route()).toMatchObject({ name: "New/kept" });
  expect(rightSidebar().map((item) => item.kind === "page" ? item.name : item.page)).toEqual(["New/ghost", "Older/x", "New/kept"]);
  expect(recentPages().map((item) => item.name)).toEqual(expect.arrayContaining(["New/ghost", "Older/x"]));
  expect(recentPages().map((item) => item.name)).not.toContain("Old/ghost");
  main.reopenClosedTab();
  expect(main.tabs().at(-1)!.history.map((r) => r.kind === "page" ? r.name : null)).toEqual(["Previous", "New/ghost"]);
  expect(main.route()).toMatchObject({ name: "New/ghost" });
});

it.each(["CLI", "dialog"].flatMap((entry) => ["conflict", "targets"].map((scenario) => ({ entry, scenario }))))("follows applied targets and keeps skipped-only children ($entry, $scenario)", async ({ entry, scenario }) => {
  const names = scenario === "conflict" ? ["Old/ghost", "Old/untouched"]
    : ["Old/macro", "Old/query", "Old/org", "Old/file", "Old/link", "Old/tag"];
  const expected = names.map((name) => scenario === "conflict" ? name : name.replace("Old/", "New/"));
  resetPaneLayoutToSingle({ tabs: names.map((name) => ({ history: [page(name)], pos: 0, pinned: false })), activeIndex: 1 });
  const main = paneRouter("main");
  await main.closeTab(main.tabs()[0].id);
  paneRouter("side").restoreSnapshot({ tabs: [{ history: [page(names[0])], pos: 0, pinned: false }], activeIndex: 0 });
  setRightSidebar(names.map((name) => ({ kind: "page", name, pageKind: "page", collapsed: false })));
  setRecentPages(names.map((name) => ({ name, kind: "page" })));
  // Rust pins these members against the bytes actually written. The optional
  // capture runs these same navigation assertions on its real serialized report.
  const capture = process.env.TINE_RENAME_REPORT_CAPTURE;
  const report: RenameDone = capture ? JSON.parse(readFileSync(join(capture, `${scenario}.json`), "utf8")) : {
    ...oldToNew,
    skipped_conflicted_referrers: ["pages/Conflict.md"],
    renamed: [...oldToNew.renamed, ...(scenario === "conflict" ? [] : names.map((name, i) => renamed(name, expected[i], null)))],
  };
  if (entry === "CLI") await cliRename("Old", "New", "pages/Old.md", report);
  else {
    vi.spyOn(backend(), "renamePage").mockResolvedValueOnce(report);
    expect(await renamePageOnDisk("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("renamed");
  }
  expect(main.tabs().map((tab) => tab.history[0]).map((route) => route.kind === "page" ? route.name : null)).toEqual(expected.slice(1));
  expect(paneRouter("side").route()).toMatchObject({ name: expected[0] });
  expect(rightSidebar().map((item) => item.kind === "page" ? item.name : item.page)).toEqual(expected);
  expect(recentPages().map((item) => item.name)).toEqual(expect.arrayContaining(expected));
  if (scenario === "targets") expect(recentPages().map((item) => item.name).filter((name) => name.startsWith("Old/"))).toEqual([]);
  main.reopenClosedTab();
  expect(main.route()).toMatchObject({ name: expected[0] });
});
