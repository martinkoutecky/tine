import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const css = readFileSync("src/styles/app.css", "utf8");

function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`${escaped}\\s*\\{([^}]*)\\}`).exec(css)?.[1] ?? "";
}

// GH #340: a short tab title must not leave the close control sitting right
// after the text. The title absorbs the free space so the ✕ stays pinned to
// the tab's right edge at every width, and min-width: 0 keeps long titles
// truncating with an ellipsis instead of pushing the ✕ out.
describe("tab close stays rightmost (GH #340)", () => {
  it("the title grows into the tab's free space and still truncates", () => {
    expect(rule(".tab-title")).toMatch(/flex:\s*1\s+1\s+auto/);
    expect(rule(".tab-title")).toMatch(/min-width:\s*0/);
  });

  it("the close control never shrinks or grows", () => {
    expect(rule(".tab-close")).toMatch(/flex:\s*none/);
  });
});

// GH #529: tabs are wide enough to read typical page titles. They stretch into a
// roomy strip up to the max and shrink evenly to the min before the strip scrolls.
describe("tab width (GH #529)", () => {
  const px = (prop: string) => Number(new RegExp(`${prop}:\\s*(\\d+)px`).exec(rule(".tab"))?.[1]);

  it("keeps about 14 title characters at the narrowest and about 31 at the widest", () => {
    expect(px("min-width")).toBe(144);
    expect(px("max-width")).toBe(260);
  });

  it("stretches and shrinks (Chrome-style) instead of sizing to the label", () => {
    expect(rule(".tab")).toMatch(/flex:\s*1\s+1\s+260px/);
  });
});
