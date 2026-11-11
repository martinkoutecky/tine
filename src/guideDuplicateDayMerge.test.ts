import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("explains that opening a duplicate day after Merge shows both files' blocks", () => {
  const page = readFileSync("crates/tine-guide/src/templates/files-external-edits-backups.md", "utf8");
  expect(page).toContain("After **Merge**, opening the day shows the blocks from both files.");
});
