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
