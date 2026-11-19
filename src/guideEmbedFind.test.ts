import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the Guide explains copying offscreen references and counting export levels from each reference", () => {
  const find = readFileSync("crates/tine-guide/src/templates/find-and-revisit.md", "utf8");
  expect(find).toContain("including groups you haven’t scrolled to");
  expect(find).toContain("Level 1 copies the reference itself, and level 2 also includes its immediate children");
  expect(find).toContain("A checked descendant that falls below the chosen level starts its own branch");
  expect(find).toContain("While Tine reads the selected sources, you can cancel the export");
});

it("the Guide teaches selection-seeded Find and the page-embed host fold", () => {
  const find = readFileSync("crates/tine-guide/src/templates/find-and-revisit.md", "utf8");
  expect(find).toContain("Select a single line of text in a block editor or rendered content");
  expect(find).toContain("highlights the first match");
  expect(find).toContain("it keeps your previous query");
  expect(find).toContain("Find and Replace search loaded days only");
  expect(find).toContain("N older days not searched — search all journals");
  expect(find).toContain("Linked references not searched");
  expect(find).toContain("Embeds and queries inside the page are searched as they load");
  expect(find).toContain("the full count stays stable while scrolling");
  expect(find).toContain("**search linked references** closes Find and opens that section's own search with your query");
  const embed = readFileSync("crates/tine-guide/src/templates/pages-links-references-search.md", "utf8");
  expect(embed).toContain("Find says how many embeds or queries are not searched yet");
  expect(embed).toContain("Only its page-name line remains");
  expect(embed).toContain("saved on the host block, never the embedded page");
  expect(embed).toContain("the same fold hides those too");
  expect(embed).toContain("main pane and right sidebar");
  expect(embed).toContain("Folds inside a page or block embed belong to the source block");
  expect(embed).toContain("indent guide's **Collapse/Expand all children** save `collapsed::` on the source");
  expect(embed).toContain("Enter after a folded branch creates a visible sibling");
  expect(embed).toContain("If the source is read-only, its folds stay local to the session");
});
