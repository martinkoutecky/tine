import { expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { authorKey } from "../commentIdentity";

// GH #892 (docs/contracts/theme-tokens.md section 8): the key two names are compared by, and the stylesheet
// hooks the contract promises. The rows themselves are in Block.authorRuns.test.tsx.
const css = readFileSync("src/styles/comments.css", "utf8");
const threadLook = css.slice(css.indexOf("GH #892: the thread look"), css.indexOf("Threads read flat"));

it("a name's key is trimmed, space-collapsed and case-folded; the stored text is not touched", () => {
  expect(authorKey("  Ada   Lovelace ")).toBe("ada lovelace");
  expect(authorKey("ADA lovelace")).toBe(authorKey("Ada Lovelace"));
  expect(authorKey("Ada")).not.toBe(authorKey("Bo"));
});

it("the thread look is one class on <html>, drawn with logical sides and the public run classes", () => {
  expect(threadLook.length).toBeGreaterThan(500);
  expect(threadLook).toMatch(/^\.tine-author-thread \{/m);
  for (const hook of [".author-run-continued", ".author-run-continues", "inset-inline-end", "float: inline-end", "margin-inline-end", "--author-thread-clear: 44px"])
    expect(threadLook, hook).toContain(hook);
  expect(threadLook).not.toMatch(/(^|[\s;{])(left|right|margin-left|margin-right|padding-left|padding-right|float: right|float: left):/m);
  expect(threadLook).not.toMatch(/float: (left|right)/);
});

it("the tint steps aside for the thread look and uses the card classes, not the run classes", () => {
  const tint = css.slice(css.indexOf("GH #893: another author's block"), css.indexOf("GH #892: the thread look"));
  expect(tint).toContain(":where(:root:not(.tine-author-thread))");
  expect(tint).toContain("author-card-continued");
  expect(tint).not.toContain("author-run-");
});

it("the chip takes an icon from the two public tokens", () => {
  expect(css).toContain("background-image: var(--tine-author-icon, none)");
  expect(css).toContain("width: var(--tine-author-icon-size, auto)");
  expect(css).toContain("text-indent: var(--tine-author-icon-size, 0px)");
});
