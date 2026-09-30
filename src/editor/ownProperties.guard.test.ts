import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: hidden-property splitting uses lsdoc's own-property regions; exemplar editor/properties.ts", () => {
  const source = readFileSync(new URL("./properties.ts", import.meta.url), "utf8");
  const classifier = source.slice(source.indexOf("function classifyLines("), source.indexOf("/** Split a block"));
  expect(classifier).toContain("blockRegions(raw, format)");
  expect(classifier).toContain("p.primary");
  expect(classifier).not.toMatch(/orgBlockDrawerRange|propLineKey|orgDrawerKey|literalBlockOfLine|\.test\(|\.exec\(/);
});
