// Master 3ce672cee: a highlighted block keeps a 4px background overhang, and the dot and
// ordinal bullets share one 22px horizontal track so text starts at the same x.
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const css = fs.readFileSync(path.join(process.cwd(), "src/styles/app.css"), "utf8");

function ruleBody(selector: string, requiredDeclaration?: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const matches = [...css.matchAll(new RegExp(`^${escaped}\\s*\\{([^}]*)\\}`, "gm"))];
  return matches.find((match) => !requiredDeclaration || match[1].includes(requiredDeclaration))?.[1] ?? "";
}

function pixels(body: string, property: string): number {
  const escaped = property.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const value = body.match(new RegExp(`${escaped}:\\s*(-?\\d+)px`))?.[1];
  if (value == null) throw new Error(`${property} is not an integer pixel declaration`);
  return Number(value);
}

describe("block bullet and background spacing", () => {
  it("gives dot and ordinal controls the same 22px horizontal track", () => {
    const bullet = ruleBody(".bullet-container", "width:");
    const ordered = ruleBody(".bullet-container.ordered", "min-width:");

    expect(pixels(bullet, "width") + pixels(bullet, "margin-right")).toBe(22);
    expect(pixels(ordered, "min-width") + pixels(ordered, "margin-right")).toBe(22);
  });

  it("keeps highlighted text aligned while limiting the background overhang to 4px", () => {
    const highlighted = ruleBody(".block-content.has-bg");
    expect(highlighted).toMatch(/padding:\s*1px 4px/);
    expect(highlighted).toMatch(/margin:\s*-1px -4px/);
    expect(highlighted).toMatch(/border-radius:\s*5px/);
  });
});
