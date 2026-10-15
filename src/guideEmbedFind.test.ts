import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the Guide teaches selection-seeded Find and the page-embed host fold", () => {
  const find = readFileSync("crates/tine-core/src/templates/find-and-revisit.md", "utf8");
  expect(find).toContain("Select a single line of text in a block editor or rendered content");
  expect(find).toContain("highlights the first match");
  expect(find).toContain("it keeps your previous query");
  const embed = readFileSync("crates/tine-core/src/templates/pages-links-references-search.md", "utf8");
  expect(embed).toContain("Only its page-name line remains");
  expect(embed).toContain("saved on the host block, never the embedded page");
  expect(embed).toContain("the same fold hides those too");
  expect(embed).toContain("main pane and right sidebar");
});
