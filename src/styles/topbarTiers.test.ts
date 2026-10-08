// og H4 (og-G #54): master a006f1308 GH #205 half (final shape at master 5dfc84503,
// src/styles/app/00-shell-settings.css): a phone (no .win-controls) keeps calendar,
// journals and theme (.topbar-optional-action) directly visible down to a 345px
// topbar content box; only a frameless window (with .win-controls) collapses them at 460px.
// Master's own harness: scripts/shot-topbar-responsive.mjs "phone-actions-inline-390" optional: 3.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync("src/styles/topbar.css", "utf8");
function tier(px: number): string {
  const at = css.indexOf(`@container topbar (max-width: ${px}px)`);
  if (at < 0) return "";
  const next = css.indexOf("@container topbar", at + 10);
  return css.slice(at, next < 0 ? undefined : next);
}

describe("og-g a006f130 responsive topbar (GH #205)", () => {
  it("does not hide phone topbar actions at the 460px frameless-desktop tier", () => {
    const t460 = tier(460).replace(/\/\*[\s\S]*?\*\//g, "");
    expect(t460).not.toMatch(/\.topbar \.topbar-optional-action\s*\{\s*display:\s*none/);
    expect(t460).toMatch(/\.topbar:where\(:has\(\.win-controls, \.tab-bar\)\) \.topbar-optional-action/);
  });
  it("collapses optional actions for every topbar only at the 345px content-box tier", () => {
    expect(tier(345).replace(/\/\*[\s\S]*?\*\//g, "")).toMatch(/\.topbar \.topbar-optional-action\s*\{\s*display:\s*none/);
  });
});
