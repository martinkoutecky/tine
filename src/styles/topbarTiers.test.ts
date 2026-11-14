// og H4 (og-G #54): master a006f1308 GH #205 half (final shape at master 5dfc84503,
// src/styles/app/00-shell-settings.css): a phone (no .win-controls) keeps calendar,
// journals and theme (.topbar-optional-action) directly visible down to a 345px
// topbar content box; only a frameless window (with .win-controls) collapses them at 460px.
// Master's own harness: scripts/shot-topbar-responsive.mjs "phone-actions-inline-390" optional: 3.
// GH #926: tiers are in rem (px container lengths never engage under WebKit's
// interface zoom), and the shared tier is now the measured 386px content box
// (24.125rem): since Share joined the optional actions a 390px phone no longer
// fits them inline, so they use the "…" menu there (logged in RECEIPT-fix-ios-ui-926.md).
// TOOLBAR-390 / TOOLBAR-390B (D-20261111-QP2): the compact workspace switcher is
// what gives way at that boundary (its twin is the drawer's first row), so the
// optional actions stay inline down to a 333px content box (RECEIPT-og-926-b.md).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { placeToolbarActions, toolbarActions } from "../toolbarChoice";

const css = readFileSync("src/styles/topbar.css", "utf8");
function tier(length: string): string {
  const at = css.indexOf(`@container topbar (max-width: ${length})`);
  if (at < 0) return "";
  const next = css.indexOf("@container topbar", at + 10);
  return css.slice(at, next < 0 ? undefined : next);
}

describe("og-g a006f130 responsive topbar (GH #205)", () => {
  const optional = ["calendar", "journals", "share", "theme"];
  const placement = (width: number, tabs = false, windowControls = false, workspace = true) => placeToolbarActions(null, width, {
    tabs, windowControls, workspace, available: toolbarActions.map(action => action.id), budget: 0, costs: {},
  });
  it("does not hide phone topbar actions at the 460px frameless-desktop tier", () => {
    expect(placement(460).bar.filter(id => optional.includes(id))).toEqual(optional);
    for (const [tabs, controls] of [[true, false], [false, true]]) {
      expect(placement(460, tabs, controls).menu.filter(id => optional.includes(id))).toEqual(optional);
    }
  });
  // GH #926 TOOLBAR-390B: at the 386px boundary the compact workspace switcher
  // leaves the bar; calendar, journals, Share and theme stay.
  it("hides the compact workspace switcher, not the optional actions, at the 386px tier", () => {
    expect(placement(386).menu).toContain("workspaces");
    expect(placement(386).bar.filter(id => optional.includes(id))).toEqual(optional);
    expect(placement(386, false, false, false).bar.filter(id => optional.includes(id))).toEqual(optional);
  });
  it("collapses optional actions for every topbar at the 332px content-box tier", () => {
    for (const [tabs, controls] of [[false, false], [true, false], [false, true]]) {
      expect(placement(332, tabs, controls).menu.filter(id => optional.includes(id))).toEqual(optional);
    }
  });
  it("keeps Settings and the overflow menu on screen below the last fixed tier (GH #926)", () => {
    const floor = tier("12.125rem").replace(/\/\*[\s\S]*?\*\//g, "");
    expect(floor).not.toMatch(/data-workspace-switcher-compact/);
    expect(floor).toMatch(/\.topbar \.topbar-left\s*\{[^}]*min-width:\s*0;[^}]*overflow-x:\s*auto/);
  });
});
