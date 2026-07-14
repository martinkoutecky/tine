import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const root = path.resolve(import.meta.dirname, "..");
const capture = fs.readFileSync(path.join(root, "src/styles/capture.css"), "utf8");

describe("Quick Capture frameless window chrome", () => {
  it("draws a theme-aware inset border without changing the window geometry", () => {
    const bodyRules = [...capture.matchAll(/^body\s*\{([^}]*)\}/gms)];
    const bodyRule = bodyRules.at(-1)?.[1] ?? "";
    expect(bodyRule).toContain("box-shadow: inset 0 0 0 1px var(--border-color)");
  });
});
