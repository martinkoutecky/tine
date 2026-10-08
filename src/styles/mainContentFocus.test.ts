import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const css = readFileSync("src/styles/app.css", "utf8");

function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`${escaped}\\s*\\{([^}]*)\\}`).exec(css)?.[1] ?? "";
}

// GH #345 (master c3b7f42d7): the pane scroller is tabindex="-1" only as a programmatic focus
// target; a click on empty space focuses it and the UA then paints its default frame around
// the whole content area. jsdom has no focus-visible heuristics, so the rule is source-scanned.
describe("main-content default focus frame (GH #345)", () => {
  it("suppresses the default focus outline on the pane scroller", () => {
    expect(rule(".main-content:focus")).toMatch(/outline:\s*(none|0)\b/);
  });

  it("keeps the deliberate pane-select ring intact", () => {
    expect(rule(".pane-selected")).toMatch(/outline:\s*2px solid/);
  });
});
