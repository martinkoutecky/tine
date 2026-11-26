import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const css = fs.readFileSync(path.join(process.cwd(), "src/styles/app.css"), "utf8");

describe("page actions responsive discoverability", () => {
  // GH #1025: the title's actions show through one group rule (titleRowReveal.test.ts); the ellipsis takes it.
  it("keeps the ellipsis visible for keyboard focus and while its menu is open", () => {
    expect(css).toMatch(/\.page-title-row:focus-within \.page-title-actions\s*\{\s*--title-actions-shown:\s*1;/);
    expect(css).toMatch(/\.page-actions-trigger\[aria-expanded="true"\]\s*\{\s*--reveal:\s*1;/);
  });

  it("gives coarse pointers a visible 36px target", () => {
    expect(css).toMatch(/@media \(pointer: coarse\)\s*\{\s*\.page-actions-trigger\s*\{\s*min-width:\s*36px;\s*min-height:\s*36px;/);
    expect(css).toMatch(/@media \(hover: none\), \(pointer: coarse\)\s*\{\s*\.page-title-actions\s*\{\s*--title-actions-shown:\s*1;/);
  });

  it("does not hide the ellipsis on narrow journal rows with carry actions", () => {
    expect(css).toContain(".page-title-row:has(.page-carry-actions) .fav-star");
    expect(css).not.toContain(".page-title-row:has(.page-carry-actions) .page-actions-trigger");
  });
});
