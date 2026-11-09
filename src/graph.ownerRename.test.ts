import { afterEach, expect, it, vi } from "vitest";
import { refreshAfterRename } from "./graph";
import { backend } from "./backend";
import { paneRouter, resetPaneLayoutToSingle } from "./panes";
import { applyOwnerPageRename, renamePageOnDisk } from "./document/graphRewrite";
import { doc, setDoc } from "./document/model";
import { resetStore } from "./document/workingSet";
import { recentPages, rightSidebar, setRecentPages, setRightSidebar } from "./ui";
import type { RenameDone } from "./types";

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setRecentPages([]);
  setRightSidebar([]);
  resetPaneLayoutToSingle();
});

it.each([undefined, "pages/Old.md"])("keeps an open source route on the CLI rename destination (path %s)", (path) => {
  resetPaneLayoutToSingle({
    tabs: [{ history: [
      { kind: "page", name: "Previous", pageKind: "page" },
      { kind: "page", name: "Old", pageKind: "page", ...(path ? { path } : {}) },
    ], pos: 1, pinned: false }], activeIndex: 0,
  });
  refreshAfterRename("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" }, true);
  expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "New", pageKind: "page" });
  expect(paneRouter("main").tabs()[0].history[0]).toMatchObject({ name: "Previous" });
});

it("keeps an open referrer selected when the CLI renames its target", () => {
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "page", name: "Referrer", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0,
  });
  refreshAfterRename("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" }, true);
  expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Referrer" });
});

it("follows a fileless source using the owner's null-path notification", async () => {
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "page", name: "Ghost", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0,
  });
  await applyOwnerPageRename({ old: "Ghost", name: "NewGhost", path: null, kind: "page", binding_generation: 1,
    report: { outcome: "renamed", touched: [] } });
  expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "NewGhost" });
});

// Card HDnCws (CLI slice 3 review r1 R5): a rename moves the namespace children
// with their parent, and every view of a moved child follows it, whoever started
// the rename. The backend reports paths; the views hold names.
const moved = (path: string) => ({ path, moved: true });
const nsReport: RenameDone = {
  outcome: "renamed",
  touched: [moved("pages/Old.md"), moved("pages/Old___child.md"), moved("pages/Old___child___leaf.md"), { path: "pages/Refs.md", moved: false }],
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
  // A sibling that merely shares a prefix with the renamed page is not its child;
  // a child keeps its own spelling below the new name, as the backend names it.
  expect(side.map((r) => r.kind === "page" ? r.name : null)).toEqual(["Older/child", "New/CHILD/leaf"]);
  expect(rightSidebar().map((item) => item.kind === "page" ? item.name : item.page))
    .toEqual(["New/child", "New/child/leaf", "Elsewhere/child"]);
  // Recent also holds whatever a pane just showed; none of it keeps the old name.
  const recent = recentPages().map((item) => item.name);
  expect(recent).toEqual(expect.arrayContaining(["New/child", "Other/child"]));
  expect(recent.filter((name) => /^old\//i.test(name))).toEqual([]);
}

it("moves a pane, a split pane, the sidebar and Recent from a namespace child to its new name (CLI rename)", async () => {
  openChildren();
  await applyOwnerPageRename({ old: "Old", name: "New", path: "pages/Old.md", kind: "page", binding_generation: 1, report: nsReport });
  expectChildrenFollowed();
});

it("moves them the same way when the app's own rename dialog renames the parent", async () => {
  openChildren();
  vi.spyOn(backend(), "renamePage").mockResolvedValueOnce(nsReport);
  expect(await renamePageOnDisk("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("renamed");
  expectChildrenFollowed();
});

it("leaves a reference-only namespace child where it is: the rename moved no file for it", async () => {
  openChildren();
  // `Old/child` is loaded without a file of its own, and `leaf`'s file did not move.
  setDoc({ byId: {}, feed: [], loaded: true, pages: [
    { name: "Old/child", id: undefined, kind: "page", title: "Old/child", preBlock: null, roots: [], format: "md", readOnly: false, guide: false },
  ] } as never);
  await applyOwnerPageRename({ old: "Old", name: "New", path: "pages/Old.md", kind: "page", binding_generation: 1,
    report: { outcome: "renamed", touched: [moved("pages/Old.md"), moved("pages/Old___child___leaf.md")] } });
  expect(paneRouter("main").route()).toMatchObject({ name: "Old/child" });
  expect(paneRouter("side").route()).toMatchObject({ name: "New/CHILD/leaf" });
  expect(doc.pages).toHaveLength(1);
});
