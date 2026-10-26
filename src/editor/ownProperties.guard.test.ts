import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

// I-12, twin 13 (outline-kernel DESIGN §10): the editor-text projection has one
// owner, the kernel's `editor_text` (crates/tine-outline/src/editor_text.rs),
// which reads lsdoc's own-property regions. The window only delegates.
const source = readFileSync(new URL("./properties.ts", import.meta.url), "utf8");
const projection = source.slice(source.indexOf("export function splitProps"), source.indexOf("/** First value for"));

it("I-12: hidden-property splitting is the kernel's editor_text; exemplar editor/properties.ts::splitProps", () => {
  expect(projection).toContain("editorText(raw, format, policyText(hide))");
  expect(projection).toContain("editorOffsetOfRaw(raw, format, policyText(hide), rawOffset)");
  // No second classifier: the window never decides which lines are metadata.
  expect(source).not.toMatch(/function classifyLines|LineClass|splitPropsInternal/);
  expect(projection).not.toMatch(/\.split\("\\n"\)|\.test\(|\.exec\(|RegExp|p\.primary/);
});

it("I-12: Org reattachment and hide policies are the kernel's; exemplar editor/properties.ts::joinProps", () => {
  expect(source).not.toMatch(/orgBlockDrawerRange|orgLinesWithNewDrawer|BUILTIN_HIDDEN/);
  const join = projection.slice(projection.indexOf("export function joinProps"));
  expect(join).toContain("editorTextJoin(visible, hidden, format)");
  expect(join).not.toMatch(/\.test\(|\.exec\(|RegExp|startsWith\(\"(?:SCHEDULED|DEADLINE|CLOSED|:PROPERTIES:)/);
  const hides = source.slice(source.indexOf("export function hides"), source.indexOf("export function hides") + 200);
  expect(hides).toContain("hidePolicyHides(policyText(policy), key)");
  const session = readFileSync(new URL("./propertySession.ts", import.meta.url), "utf8");
  expect(session).toContain("p.primary && hides(hide, p.key)");
  expect(session).not.toMatch(/seedFacets|cache\.set|regionCache\.set/);
});
