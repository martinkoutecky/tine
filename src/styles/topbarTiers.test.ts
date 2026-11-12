// og H4 (og-G #54): master a006f1308 GH #205 half (final shape at master 5dfc84503,
// src/styles/app/00-shell-settings.css): a phone (no .win-controls) keeps calendar,
// journals and theme (.topbar-optional-action) directly visible down to a 345px
// topbar content box; only a frameless window (with .win-controls) collapses them at 460px.
// Master's own harness: scripts/shot-topbar-responsive.mjs "phone-actions-inline-390" optional: 3.
// GH #926: tiers are in rem (px container lengths never engage under WebKit's
// interface zoom), and the shared tier is now the measured 386px content box
// (24.125rem): since Share joined the optional actions a 390px phone no longer
// fits them inline, so they use the "…" menu there (logged in RECEIPT-fix-ios-ui-926.md).
// TOOLBAR-390 (D-20261111-QP2): that holds only beside the compact workspace
// switcher; without it the optional actions stay inline down to 333px (RECEIPT-og-926.md).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync("src/styles/topbar.css", "utf8");
function tier(length: string): string {
  const at = css.indexOf(`@container topbar (max-width: ${length})`);
  if (at < 0) return "";
  const next = css.indexOf("@container topbar", at + 10);
  return css.slice(at, next < 0 ? undefined : next);
}

describe("og-g a006f130 responsive topbar (GH #205)", () => {
  it("does not hide phone topbar actions at the 460px frameless-desktop tier", () => {
    const t460 = tier("28.75rem").replace(/\/\*[\s\S]*?\*\//g, "");
    expect(t460).not.toMatch(/\.topbar \.topbar-optional-action\s*\{\s*display:\s*none/);
    expect(t460).toMatch(/\.topbar:where\(:has\(\.win-controls, \.tab-bar\)\) \.topbar-optional-action/);
  });
  // GH #926 TOOLBAR-390: the 386px tier collapses them only beside the compact
  // workspace switcher; without it they fit down to a 333px content box.
  it("collapses optional actions at the 386px tier only beside the compact workspace switcher", () => {
    const t386 = tier("24.125rem").replace(/\/\*[\s\S]*?\*\//g, "");
    expect(t386).toMatch(/\.topbar:has\(\[data-workspace-switcher-compact\]\) \.topbar-optional-action\s*\{\s*display:\s*none/);
    expect(t386).not.toMatch(/\.topbar \.topbar-optional-action\s*\{\s*display:\s*none/);
  });
  it("collapses optional actions for every topbar at the 332px content-box tier", () => {
    expect(tier("20.75rem").replace(/\/\*[\s\S]*?\*\//g, "")).toMatch(/\.topbar \.topbar-optional-action\s*\{\s*display:\s*none/);
  });
  it("keeps Settings and the overflow menu on screen below the last fixed tier (GH #926)", () => {
    const floor = tier("12.125rem").replace(/\/\*[\s\S]*?\*\//g, "");
    expect(floor).toMatch(/\[data-workspace-switcher-compact\][\s\S]*?display:\s*none/);
    expect(floor).toMatch(/\.topbar \.topbar-left\s*\{[^}]*min-width:\s*0;[^}]*overflow-x:\s*auto/);
  });
});
