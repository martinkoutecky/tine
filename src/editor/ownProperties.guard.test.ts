import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: hidden-property splitting uses lsdoc's own-property regions; exemplar editor/properties.ts", () => {
  const source = readFileSync(new URL("./properties.ts", import.meta.url), "utf8");
  const classifier = source.slice(source.indexOf("function classifyLines("), source.indexOf("/** Split a block"));
  expect(classifier).toContain("blockRegions(raw, format)");
  expect(classifier).toContain("p.primary");
  expect(classifier).not.toMatch(/orgBlockDrawerRange|propLineKey|orgDrawerKey|literalBlockOfLine|\.test\(|\.exec\(/);
});

it("I-12: Org reattachment uses accepted regions; exemplar editor/properties.ts::joinProps", () => {
  const source = readFileSync(new URL("./properties.ts", import.meta.url), "utf8");
  expect(source).not.toMatch(/orgBlockDrawerRange|orgLinesWithNewDrawer/);
  const join = source.slice(source.indexOf("export function joinProps"), source.indexOf("/** First value for"));
  expect(join).toContain('editBlock(visible, format, { kind: "reattach_properties", hidden })');
  expect(join).not.toMatch(/\.test\(|\.exec\(|RegExp|startsWith\(\"(?:SCHEDULED|DEADLINE|CLOSED|:PROPERTIES:)/);
  const session = readFileSync(new URL("./propertySession.ts", import.meta.url), "utf8");
  expect(session).toContain("p.primary && hide(p.key.toLowerCase())");
  expect(session).not.toMatch(/seedFacets|cache\.set|regionCache\.set/);
});
