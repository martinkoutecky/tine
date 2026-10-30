import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { commandDefaults, paletteCommands, runGlobalCommand } from "./keybindings";
import { loadSingle } from "./document/workingSet";
import { clearSelection, resetStore, selectBlock } from "./document";
import { openPage, resolveRouteBlock, route } from "./router";
import { editingId, endEdit, startEditing } from "./editorController";
import { focusPane, layoutPaneIds, paneRouter, resetPaneLayoutToSingle, splitRootAtEdge } from "./panes";
import { initParser } from "./render/parse";
import { zoomInOnTarget, zoomOutOneLevel } from "./zoomCommands";
import { zoomedBlock } from "./ui";

beforeAll(() => initParser());
afterEach(() => { vi.useRealTimers(); endEdit("blur"); clearSelection(); resetPaneLayoutToSingle(); resetStore(); vi.restoreAllMocks(); });

const leaf = (id: string, children: unknown[] = []) => ({ id, raw: id, collapsed: false, children });
function load(name = "Zoomy") {
  loadSingle({ name, title: name, kind: "page", pre_block: null, blocks: [
    leaf("top", [leaf("mid", [leaf("deep", [leaf("deeper")])]), leaf("sibling")]),
    leaf("other"),
  ] } as never);
}
const zoomedKey = (r = route()) => resolveRouteBlock(r);

it("lists Zoom in on block and Zoom out one level as unbound commands in the palette (GH #799)", () => {
  const rows = Object.fromEntries(commandDefaults().map((c) => [c.id, c]));
  expect(rows["editor/zoom-in"]).toMatchObject({ english: "Zoom in on block", binding: "", scope: "global" });
  expect(rows["editor/zoom-out"]).toMatchObject({ english: "Zoom out one level", binding: "", scope: "global" });
  const palette = paletteCommands().map((c) => c.id);
  expect(palette).toContain("editor/zoom-in");
  expect(palette).toContain("editor/zoom-out");
});

it("zooms into the selected block and out one level at a time, down to the page, then stops", () => {
  load();
  openPage("Zoomy", "page");
  selectBlock("deep");
  expect(runGlobalCommand("editor/zoom-in")).toBe(true);
  expect(zoomedKey()).toBe("deep");
  expect(zoomedBlock()).not.toBeNull();
  expect(runGlobalCommand("editor/zoom-out")).toBe(true);
  expect(zoomedKey()).toBe("mid");
  runGlobalCommand("editor/zoom-out");
  expect(zoomedKey()).toBe("top");
  runGlobalCommand("editor/zoom-out");
  expect(route()).toMatchObject({ kind: "page", name: "Zoomy" });
  expect(route()).not.toHaveProperty("block");
  expect(zoomedKey()).toBeNull();
  const before = route();
  expect(zoomOutOneLevel()).toBe(false); // at the page: nothing to do, nothing changes
  expect(route()).toBe(before);
});

it("zooms into the block being edited and keeps the caret in it", () => {
  vi.useFakeTimers();
  load();
  openPage("Zoomy", "page");
  startEditing("mid", 2, null, "main");
  expect(zoomInOnTarget()).toBe(true);
  expect(zoomedKey()).toBe("mid");
  vi.runAllTimers();
  expect(editingId()).toBe("mid");
  // Zooming out re-opens the same block in the parent's view.
  zoomOutOneLevel();
  expect(zoomedKey()).toBe("top");
  vi.runAllTimers();
  expect(editingId()).toBe("mid");
});

it("without a target, Zoom in toasts and does not navigate", () => {
  load();
  openPage("Zoomy", "page");
  expect(zoomInOnTarget()).toBe(false);
  expect(zoomedKey()).toBeNull();
});

it("acts on the pane that holds the block, not on a bystander pane (routed page, split view)", () => {
  load();
  openPage("Zoomy", "page");
  const right = splitRootAtEdge("right")!;
  paneRouter(right).openPage("Zoomy", "page");
  focusPane(right);
  selectBlock("deep");
  runGlobalCommand("editor/zoom-in");
  expect(zoomedKey(paneRouter(right).route())).toBe("deep");
  expect(zoomedKey(paneRouter("main").route())).toBeNull();
  runGlobalCommand("editor/zoom-out");
  expect(zoomedKey(paneRouter(right).route())).toBe("mid");
  expect(layoutPaneIds()).toContain("main");
});

it("a block edited in the right sidebar zooms the focused pane, like its bullet", () => {
  load();
  openPage("Zoomy", "page");
  startEditing("deeper", 0, null, "sidebar:test");
  runGlobalCommand("editor/zoom-in");
  expect(zoomedKey()).toBe("deeper");
});
