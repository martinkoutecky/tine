import { afterEach, expect, it } from "vitest";
import { refreshAfterRename } from "./graph";
import { paneRouter, resetPaneLayoutToSingle } from "./panes";
import { applyOwnerPageRename } from "./document/graphRewrite";

afterEach(() => resetPaneLayoutToSingle());

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
