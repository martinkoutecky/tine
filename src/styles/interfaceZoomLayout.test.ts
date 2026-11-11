// GH #926: layout rules that must survive interface zoom on iOS (WebKit) and
// Android (Chromium). Root CSS zoom on a 390px phone leaves the app a 195px-wide
// CSS viewport at 200%, so every row has to fit by wrapping, and every
// responsive tier has to engage in both engines.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// Every stylesheet in the app (src/styles and the component sheets beside their code).
const sheets = (readdirSync("src", { recursive: true }) as string[])
  .filter((name) => name.endsWith(".css"))
  .map((name) => ({ name, css: readFileSync(join("src", name), "utf8").replace(/\/\*[\s\S]*?\*\//g, "") }));
const app = sheets.find((s) => s.name === join("styles", "app.css"))!.css;
const rows = sheets.find((s) => s.name === join("styles", "settingsRows.css"))!.css;

// Every top-level rule whose whole selector is `selector`, concatenated.
function block(css: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return [...css.matchAll(new RegExp(`^${escaped} \\{[^}]*\\}`, "gm"))].map((m) => m[0]).join("\n");
}

describe("interface zoom layout (GH #926)", () => {
  it("states every container-query length in rem or em, never px", () => {
    // WebKit compares a px container-query length with the zoomed box under
    // CSS zoom (measured on WebKitGTK 2.50), so a px tier never engages on iOS.
    const px = sheets.flatMap(({ name, css }) =>
      [...css.matchAll(/@container[^{]*/g)].filter((m) => /\d(\.\d+)?px/.test(m[0])).map((m) => `${name}: ${m[0].trim()}`));
    expect(px).toEqual([]);
  });

  it("lets both settings row primitives wrap the control under its label", () => {
    expect(block(rows, ".settings-row")).toMatch(/flex-wrap:\s*wrap/);
    expect(block(rows, ".settings-field-row")).toMatch(/flex-wrap:\s*wrap/);
    // A non-shrinking control is what pushed the pane into sideways scroll.
    expect(block(rows, ".settings-field-control")).toMatch(/flex:\s*0 1 auto/);
    expect(rows).toMatch(/:is\(\.settings-row, \.settings-field-row\) > \.settings-label\s*\{[^}]*min-width:\s*min\(8rem, 100%\)/);
    expect(rows).toMatch(/\.settings-row > :not\(\.settings-label\),\s*\.settings-field-control\s*\{[^}]*min-width:\s*0;[^}]*max-width:\s*100%;[^}]*overflow-wrap:\s*anywhere/);
  });

  it("keeps fixed-width settings controls inside a narrow pane", () => {
    expect(block(app, ".spellcheck-dicts")).toMatch(/min-width:\s*min\(240px, 100%\)/);
    expect(block(rows, ".og-badge")).not.toMatch(/white-space:\s*nowrap/);
    expect(block(app, ".settings-segment button")).not.toMatch(/white-space:\s*nowrap/);
  });

  it("wraps the page title row and the references header instead of widening the page", () => {
    expect(block(app, ".page-title-row")).toMatch(/flex-wrap:\s*wrap/);
    expect(block(app, ".references-header")).toMatch(/flex-wrap:\s*wrap/);
  });
});
