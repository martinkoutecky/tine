import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the search tour teaches pointer and keyboard activation in the focused split", () => {
  const page = readFileSync("crates/tine-core/src/templates/find-and-revisit.md", "utf8");
  expect(page).toContain("Click or tap a result to open it in the pane you were using");
  expect(page).toContain("click or tap a command, or highlight it and press **Enter**");
  expect(page).toContain("Search and Commands keep your focused split pane");
  expect(page).toContain("Picking a slash command or a `[[` page suggestion while editing also keeps that editor's split pane focused");
});

it("the code tour explains raw source entry on desktop and mobile", () => {
  const page = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  expect(page).toContain("To edit the language or add text before or after the code");
  expect(page).toContain("the whole bullet opens as source, including its fences");
  expect(page).toContain("These paths work on desktop and mobile");
  expect(page).toContain("skip the language list by tapping inside the code");
  expect(page).toContain("Typing continues at your caret");
});

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
