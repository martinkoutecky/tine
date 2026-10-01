import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
it("I-12: raw HTML resources use htmlSanitize.ts's inert DOM presentation owner", () => {
  const inline = readFileSync("src/render/inline.tsx", "utf8");
  const owner = readFileSync("src/render/htmlSanitize.ts", "utf8");
  expect(inline, "I-12: render raw HTML via rawHtmlPresentation in src/render/htmlSanitize.ts").toContain("rawHtmlPresentation(props.text, props.allowIframe)");
  for (const source of [inline, owner]) {
    expect(source, "I-12: HTML elements/attributes belong to the inert DOM, not source regexes; imitate htmlSanitize.ts").not.toMatch(/\/<(?:iframe|img)\\b|const (?:IMG_RE|SRC_RE)/);
  }
  expect(owner).toContain('querySelector("iframe")');
  expect(owner).toContain('getAttribute("src")');
  expect(owner).toContain("IN_PLACE: true");
});
