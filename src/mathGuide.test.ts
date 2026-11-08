import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("onboards formula-local scrolling when KaTeX cannot break a long equation", () => {
  const guide = readFileSync("crates/tine-core/src/templates/showcase.md", "utf8");
  expect(guide).toContain("break at suitable operators when KaTeX permits it");
  expect(guide).toContain("scroll the formula horizontally to read both ends");
});
