// og H4 (og-G #54): master a006f1308 GH #205 half (final shape at master 5dfc84503,
// src/styles/app/00-shell-settings.css): a phone (no .win-controls) keeps its
// optional actions (.topbar-optional-action) directly visible down to the shared
// content-box tier; only a frameless window (with .win-controls) collapses them at 460px.
// Discussion #968 (stable port of develop 3b837685d, GH #926): tiers are in rem
// (px container lengths never engage under WebKit's interface zoom), and the shared
// tier is the measured 386px content box (24.125rem): with Back/Forward, the compact
// workspace switcher and Share a 390px phone no longer fits them inline, and the old
// 345px tier left Settings past the right edge of the screen.
// Browser proof: scripts/shot-topbar-responsive.mjs (Settings on screen in every case).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync("src/styles/topbar.css", "utf8");
function tier(length: string): string {
  const at = css.indexOf(`@container topbar (max-width: ${length})`);
  if (at < 0) return "";
  const next = css.indexOf("@container topbar", at + 10);
  return css.slice(at, next < 0 ? undefined : next);
}
const uncommented = (text: string) => text.replace(/\/\*[\s\S]*?\*\//g, "");

describe("og-g a006f130 responsive topbar (GH #205)", () => {
  it("does not hide phone topbar actions at the 460px frameless-desktop tier", () => {
    const t460 = uncommented(tier("28.75rem"));
    expect(t460).not.toMatch(/\.topbar \.topbar-optional-action\s*\{\s*display:\s*none/);
    expect(t460).toMatch(/\.topbar:where\(:has\(\.win-controls, \.tab-bar\)\) \.topbar-optional-action/);
  });
  it("collapses optional actions for every topbar at the 386px content-box tier, so a 375-414px phone keeps Settings on screen (discussion #968)", () => {
    expect(uncommented(tier("24.125rem"))).toMatch(/\.topbar \.topbar-optional-action\s*\{\s*display:\s*none/);
  });
  it("states every topbar tier in rem, so the tiers engage under WebKit interface zoom (GH #926)", () => {
    expect(uncommented(css)).not.toMatch(/@container topbar \(max-width:\s*[\d.]+px\)/);
  });
  it("adds a floor tier that hides the compact workspace switcher and lets the left cluster scroll (GH #926)", () => {
    const floor = uncommented(tier("12.125rem"));
    expect(floor).toMatch(/\[data-workspace-switcher-compact\][\s\S]*?display:\s*none/);
    expect(floor).toMatch(/\.topbar \.topbar-left\s*\{[^}]*min-width:\s*0;[^}]*overflow-x:\s*auto/);
  });
  it("bounds the phone overflow menu to the toolbar so its rows stay on screen at high zoom (GH #926 review B1)", () => {
    const shared = uncommented(tier("24.125rem"));
    expect(shared).toMatch(/\.topbar \.topbar-overflow\s*\{\s*position:\s*static;/);
    expect(shared).toMatch(/\.topbar \.topbar-overflow-menu\s*\{[^}]*max-width:\s*calc\(100% - 8px\)/);
  });
});
