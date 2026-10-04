import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
it("the canonical Guide teaches linked property keys, shaded embeds and the tagged-page section", () => {
  const guide = readFileSync(new URL("../crates/tine-core/src/templates/pages-links-references-search.md", import.meta.url), "utf8");
  for (const outcome of ["click the key to open its page", "Later groups remain in the file", "shaded background in both light and dark", '**Pages tagged with "X"**', "before Linked References", "Inline block tags still belong to Linked References"]) expect(guide).toContain(outcome);
});

it("the Guide explains journal feed references and their lazy/collapse behavior", () => {
  const guide = readFileSync("crates/tine-core/src/templates/journals-tasks-scheduling.md", "utf8");
  for (const outcome of ["beneath each day in the Journals feed", "References load as you scroll", "Empty sections stay hidden", "collapse threshold"]) expect(guide).toContain(outcome);
});
