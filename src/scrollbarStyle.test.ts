import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const root = path.resolve(import.meta.dirname, "..");
const theme = fs.readFileSync(path.join(root, "src/styles/theme.css"), "utf8");
const app = fs.readFileSync(path.join(root, "src/styles/app.css"), "utf8");

describe("cross-pane scrollbar contract (GH #103)", () => {
  it("defines semantic thumb tokens from theme colors", () => {
    expect(theme).toContain("--scrollbar-thumb:");
    expect(theme).toContain("--scrollbar-thumb-hover:");
    expect(theme).toMatch(/--scrollbar-thumb:[^;]*var\(--text-secondary\)/);
  });

  it("styles every primary pane without forcing classic scrollbar geometry", () => {
    for (const selector of [".left-sidebar-scroll", ".main-content", ".right-sidebar"]) {
      expect(app).toContain(selector);
    }
    expect(app).toContain("scrollbar-color: var(--scrollbar-thumb) transparent");
    expect(app).toContain("scrollbar-width: thin");
    expect(app).toContain("::-webkit-scrollbar-thumb");
    expect(app).toContain("::-webkit-scrollbar-track");
    expect(app).toContain("::-webkit-scrollbar-corner");
    expect(app).not.toMatch(/::-webkit-scrollbar\s*\{[^}]*\b(?:width|height)\s*:/s);
  });

  it("restores native behavior for forced colors and coarse pointers", () => {
    expect(app).toContain("@media (forced-colors: active)");
    expect(app).toContain("@media (pointer: coarse)");
    expect(app).toContain("scrollbar-color: auto");
    expect(app).toContain("scrollbar-width: auto");
  });
});
