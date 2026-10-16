import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the canonical context tour teaches pinned ancestor navigation and selecting an embed host", () => {
  const page = readFileSync("crates/tine-core/src/templates/keep-context-visible.md", "utf8");
  expect(page).toContain("Click a pinned ancestor to scroll back to its block");
  expect(page).toContain("desktop and mobile");
  expect(page).toContain("Select embedding block");
  expect(page).toContain("command starts without a key");
  expect(page).toContain("block and page embeds");
});

it("the context tour distinguishes top splits from tab-strip reordering", () => {
  const page = readFileSync("crates/tine-core/src/templates/keep-context-visible.md", "utf8");
  expect(page).toContain("To split above a pane, drag just below its tab strip");
  expect(page).toContain("nearest edge wins");
  expect(page).toContain("preview shows where the new pane will open");
  expect(page).toContain("centre appends the tab");
  expect(page).toContain("tab strip keeps its reorder behavior");
});
