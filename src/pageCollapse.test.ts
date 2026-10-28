import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { commandDefaults, runGlobalCommand } from "./keybindings";
import { appendFeed, loadSingle } from "./document/workingSet";
import { doc } from "./document/model";
import { resetStore, undo, setCollapsedDeep, setCollapsedTrees } from "./document";
import * as wiring from "./document/host/wiring";
import { openPage } from "./router";
import { focusPane, layoutPaneIds, paneRouter, resetPaneLayoutToSingle, splitRootAtEdge } from "./panes";
import { initParser } from "./render/parse";

beforeAll(() => initParser());

afterEach(() => { resetPaneLayoutToSingle(); resetStore(); vi.restoreAllMocks(); });

it("offers OG's t o and explicit page commands, batching 60 blocks into one dirty page", () => {
  loadSingle({ name: "Folding", title: "Folding", kind: "page", pre_block: null,
    blocks: Array.from({ length: 30 }, (_, i) => ({ id: `p${i}`, raw: `Parent ${i}`, collapsed: false,
      children: [{ id: `c${i}`, raw: `Child ${i}`, collapsed: false, children: [] }] })) });
  openPage("Folding", "page");
  const dirty = vi.spyOn(wiring, "markDirty");
  expect(commandDefaults().find(c => c.id === "editor/toggle-open-blocks")?.binding).toBe("t o");
  expect(runGlobalCommand("editor/collapse-all")).toBe(true);
  expect(doc.byId.p0.collapsed).toBe(true);
  expect(doc.byId.p29.raw).toContain("collapsed:: true");
  expect(doc.byId.c0.raw).toBe("Child 0");
  expect(dirty).toHaveBeenCalledTimes(1);
  runGlobalCommand("editor/collapse-all");
  expect(dirty).toHaveBeenCalledTimes(1); // already folded: no save or Undo
  undo();
  expect(doc.byId.p29.collapsed).toBe(false);
  runGlobalCommand("editor/toggle-open-blocks");
  expect(doc.byId.p29.collapsed).toBe(true);
  runGlobalCommand("editor/expand-all");
  expect(doc.byId.p29.collapsed).toBe(false);
  expect(doc.byId.p29.raw).not.toContain("collapsed::");
});

it.each(["md", "org"] as const)("folds hidden subtree descendants, preserves unrelated properties and undoes atomically (%s)", format => {
  const raw = format === "md" ? "Parent\nowner:: Avery" : "Parent\n:PROPERTIES:\n:owner: Avery\n:END:";
  loadSingle({ name: "Subtree", title: "Subtree", kind: "page", format, pre_block: null, blocks: [
    { id: "root", raw, collapsed: false, children: [
      { id: "nested", raw: "Nested", collapsed: false, children: [{ id: "leaf", raw: "Leaf", collapsed: false, children: [] }] },
    ] }, { id: "other", raw: "Unrelated", collapsed: false, children: [] },
  ] });
  const dirty = vi.spyOn(wiring, "markDirty");
  setCollapsedDeep("root", true);
  expect(doc.byId.nested.collapsed).toBe(true);
  expect(doc.byId.root.raw).toContain(format === "md" ? "collapsed:: true" : ":collapsed: true");
  expect(doc.byId.root.raw).toContain("Avery");
  expect(doc.byId.other.raw).toBe("Unrelated");
  expect(dirty).toHaveBeenCalledTimes(1);
  undo();
  expect(doc.byId.root.raw).toBe(raw);
  expect(doc.byId.nested.collapsed).toBe(false);
  setCollapsedTrees(["root", "nested"], true); // overlapping selected roots
  expect(doc.byId.nested.collapsed).toBe(true);
  setCollapsedDeep("root", false);
  expect(doc.byId.nested.collapsed).toBe(false);
});

it("does not persist folding on read-only pages", () => {
  loadSingle({ name: "Protected", title: "Protected", kind: "page", pre_block: null, read_only: true,
    blocks: [{ id: "protected", raw: "Keep", collapsed: false, children: [{ id: "child", raw: "Child", collapsed: false, children: [] }] }] });
  const dirty = vi.spyOn(wiring, "markDirty");
  setCollapsedTrees(["protected"], true);
  expect(doc.byId.protected.raw).toBe("Keep");
  expect(dirty).not.toHaveBeenCalled();
});

it("changes only the focused pane's page, even with another page loaded", () => {
  const page = (name: string) => ({ name, title: name, kind: "page" as const, pre_block: null,
    blocks: [{ id: name, raw: name, collapsed: false, children: [{ id: `${name}-leaf`, raw: "Child", collapsed: false, children: [] }] }] });
  loadSingle(page("Left"));
  appendFeed([page("Right")]);
  openPage("Left", "page");
  splitRootAtEdge("right");
  const right = layoutPaneIds().at(-1)!;
  paneRouter(right).openPage("Right", "page");
  focusPane(right);
  runGlobalCommand("editor/collapse-all");
  expect(doc.byId.Right.collapsed).toBe(true);
  expect(doc.byId.Left.collapsed).toBe(false);
});

it("bulk folding includes query and page-embed bodies without child blocks", () => {
  loadSingle({ name: "Bodies", title: "Bodies", kind: "page", pre_block: null, blocks: [
    { id: "query", raw: "{{query (todo TODO)}}", collapsed: false, children: [] },
    { id: "embed", raw: "{{embed [[Other]]}}", collapsed: false, children: [] },
  ] });
  setCollapsedTrees(["query", "embed"], true);
  expect(doc.byId.query.collapsed).toBe(true);
  expect(doc.byId.embed.collapsed).toBe(true);
  setCollapsedTrees(["query", "embed"]);
  expect(doc.byId.query.collapsed).toBe(false);
  expect(doc.byId.embed.raw).toContain("collapsed:: false");
});
