import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("explains Hierarchy bracket visibility and bulletless comment threading", () => {
  const template = (name: string) => readFileSync(`crates/tine-guide/src/templates/${name}.md`, "utf8");
  expect(template("pages-links-references-search")).toContain("controls the `[[ ]]` around Hierarchy links as well as page references inside blocks");
  expect(template("plugins")).toContain("Bulletless comment messages have no threading stub; outlines with bullets inside a comment can still thread");
});
