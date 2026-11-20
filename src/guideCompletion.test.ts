import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the bundled Guide explains narrowing page completion in both editors (D-20261102-PERF)", () => {
  const guide = readFileSync("crates/tine-guide/src/templates/pages-links-references-search.md", "utf8");
  expect(guide).toContain("Keep typing inside `[[...]]` to narrow page suggestions in either editor");
  expect(guide).toContain("The list ranks page names, aliases and namespace parents for your current query");
  expect(guide).toContain("page and tag suggestions appear as soon as the current search finishes, without waiting for another reply");
  expect(guide).toContain("Chinese IME queries wait until composition finishes");
});

it("the bundled Guide explains reference search in property values from templates (#853)", () => {
  const guide = readFileSync("crates/tine-guide/src/templates/pages-links-references-search.md", "utf8");
  expect(guide).toContain("When editing a Markdown property line, type `[[` to find a page, `((` to find a block or `#` to find a tag. This works in a property from a newly inserted template too.");
  expect(guide).toContain("Picking a suggestion for an unfinished `[[`, `#[[` or `((` closes the reference at the caret and keeps the text that follows it.");
  expect(guide).toContain("A pick inside a closed reference replaces through the closing brackets or parentheses when only plain text follows the caret inside it.");
  expect(guide).toContain("the pick finishes at the caret and keeps the following text");
});
