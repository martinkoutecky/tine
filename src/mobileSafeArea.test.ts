import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * The system-inset contract (GH #205, master a0afeb665 + e3b1c3868), checked
 * as text: jsdom applies no layout, so these pin the stylesheet shape that the
 * headless-Chromium harness (scripts/check-mobile-safe-area.mjs) measures.
 */
describe("mobile safe-area insets", () => {
  const sheets = readdirSync("src/styles").filter((f) => f.endsWith(".css"));
  const allCss = sheets.map((f) => readFileSync(`src/styles/${f}`, "utf8")).join("\n");
  const css = readFileSync("src/styles/app.css", "utf8");
  const help = readFileSync("src/styles/help.css", "utf8");
  const main = readFileSync("src/main.tsx", "utf8");
  const block = (sheet: string, selector: string) => {
    const at = sheet.startsWith(`${selector} {`) ? 0 : sheet.indexOf(`\n${selector} {`);
    expect(at, `${selector} exists`).toBeGreaterThanOrEqual(0);
    return sheet.slice(at, sheet.indexOf("\n}", at));
  };

  it("centralizes each platform inset behind one system token", () => {
    for (const side of ["top", "right", "bottom", "left"]) {
      expect(css).toContain(`--system-inset-${side}: env(safe-area-inset-${side}, 0px);`);
      expect(css).toContain(`--overlay-inset-${side}: var(--system-inset-${side});`);
    }
    // Every call site reads a token; env() appears only in the four definitions.
    expect(allCss.match(/env\(safe-area-inset-(?:top|right|bottom|left)(?:, 0px)?\)/gu)).toHaveLength(4);
  });

  it("gives Android's already-inset native viewport sole ownership", () => {
    const installation = "installSystemInsetOwner();";
    expect(main).toContain(installation);
    expect(main.indexOf(installation)).toBeLessThan(main.indexOf("applyTheme();"));
    expect(css).toContain('html[data-system-insets="native-viewport"]');
    for (const side of ["top", "right", "bottom", "left"]) {
      expect(css).toContain(`--system-inset-${side}: 0px;`);
    }
  });

  it("reapplies the insets on every viewport-fixed overlay that lays out content", () => {
    for (const selector of [
      ".modal-overlay",
      ".switcher-overlay",
      ".welcome-overlay",
      ".audio-overlay",
      ".lightbox-overlay",
      ".sync-merge-overlay",
    ]) {
      // Rules may live in any split sheet (e.g. .welcome-overlay moved to
      // pdf-workspace.css); allCss joins every src/styles sheet.
      const rule = block(allCss, selector);
      for (const side of ["top", "right", "bottom", "left"]) {
        expect(rule, `${selector} ${side}`).toContain(`var(--overlay-inset-${side})`);
      }
    }
    expect(block(css, ".toast-stack")).toContain("calc(var(--overlay-inset-bottom) + 18px)");
    expect(block(css, ".parser-error-banner")).toContain("calc(var(--overlay-inset-top) + 8px)");
    expect(block(help, ".help-corner")).toContain("var(--overlay-inset-bottom, 0px) + 16px");
    const narrowHelp = help.slice(help.indexOf("@media (max-width: 720px)"));
    expect(narrowHelp.slice(0, narrowHelp.indexOf("\n  }"))).toContain("var(--overlay-inset-bottom, 0px) + 12px");
  });

  it("puts a phone-width modal's top edge on the inset and fills the inset box", () => {
    const phone = css.slice(css.indexOf("@media (max-width: 480px)"));
    const overlay = phone.slice(phone.indexOf(".modal-overlay"), phone.indexOf(".settings-modal"));
    expect(overlay).toContain("padding-top: var(--overlay-inset-top);");
    const modal = phone.slice(phone.indexOf(".settings-modal"), phone.indexOf(".settings-nav"));
    expect(modal).toContain("height: 100%;");
    expect(modal).not.toMatch(/height:\s*96vh/u);
  });
});
