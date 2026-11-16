import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

// GH #210 F1 follow-up, D-20261019-H210F2: a move above the page properties is refused with a notice.
it("the Guide says page properties stay at the top of their page and a refused move shows a notice (GH #210)", () => {
  const page = readFileSync("crates/tine-core/src/templates/pages-links-references-search.md", "utf8");
  expect(page).toContain("Page properties always stay at the top of their page");
  expect(page).toContain("does nothing and shows a short notice saying so");
});
