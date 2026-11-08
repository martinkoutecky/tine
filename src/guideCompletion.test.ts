import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the bundled Guide explains narrowing page completion in both editors (D-20261102-PERF)", () => {
  const guide = readFileSync("crates/tine-core/src/templates/pages-links-references-search.md", "utf8");
  expect(guide).toContain("Keep typing inside `[[...]]` to narrow page suggestions in either editor");
  expect(guide).toContain("The list ranks page names, aliases and namespace parents for your current query");
  expect(guide).toContain("page and tag suggestions appear as soon as the current search finishes, without waiting for another reply");
  expect(guide).toContain("Chinese IME queries wait until composition finishes");
});
