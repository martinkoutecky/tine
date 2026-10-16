import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
it("the Guide teaches result-scoped columns and retained saved choices", () => {
  const guide = readFileSync("crates/tine-core/src/templates/queries.md", "utf8");
  for (const outcome of ["only columns found in the current results", "saved column choice stays intact", "checkbox returns when the column appears again"]) expect(guide).toContain(outcome);
});
it("the Guide teaches leaving selection mode and restoring an empty query", () => {
  const guide = readFileSync("crates/tine-core/src/templates/queries.md", "utf8");
  for (const outcome of ["Removing the last condition returns to that same empty state", "Selection actions stay above the result rows", "keep their page or block labels", "Press **Select results** again or **Done** to leave selection mode"]) expect(guide).toContain(outcome);
});
it("the Guide teaches query answer continuity and Save then Escape", () => {
  const guide = readFileSync("crates/tine-core/src/templates/queries.md", "utf8");
  for (const outcome of ["Renaming keeps the current results visible", "unsaved text draft leaves the results unchanged", "Saving keeps focus in the editor", "returns focus to the query's **⋯** menu"]) expect(guide).toContain(outcome);
});
it("the Guide teaches inline checklist clicks, nesting, Org and own-content collapse", () => {
  const guide = readFileSync("crates/tine-core/src/templates/journals-tasks-scheduling.md", "utf8");
  for (const outcome of ["**Shift+Enter**", "`+ [ ] item` in Org", "tick just that item without opening the editor", "same indent step as child blocks", "its own inline checklist stays visible"]) expect(guide).toContain(outcome);
});
it("the canonical Guide teaches linked property keys, shaded embeds and the tagged-page section", () => {
  const guide = readFileSync(new URL("../crates/tine-core/src/templates/pages-links-references-search.md", import.meta.url), "utf8");
  for (const outcome of ["click the key to open its page", "Later groups remain in the file", "shaded background in both light and dark", '**Pages tagged with "X"**', "before Linked References", "Inline block tags still belong to Linked References"]) expect(guide).toContain(outcome);
});

it("the Guide explains journal feed references and their lazy/collapse behavior", () => {
  const guide = readFileSync("crates/tine-core/src/templates/journals-tasks-scheduling.md", "utf8");
  for (const outcome of ["beneath each day in the Journals feed", "References load as you scroll", "Empty sections stay hidden", "collapse threshold"]) expect(guide).toContain(outcome);
});

it("the Guide explains canonical alias chips, unique block counts and compatible filter persistence", () => {
  const guide = readFileSync("crates/tine-core/src/templates/pages-links-references-search.md", "utf8");
  for (const outcome of ["page and its aliases share one chip", "block using two aliases counts once", "Include or exclude applies to every alias", "Logseq-compatible `filters::` property", "older filter naming an alias", "Unlinked References shares the alias chips"]) expect(guide).toContain(outcome);
});
