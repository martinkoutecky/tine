import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

// Ported from master f5dbe8cdb: the in-page resolver gets the pane's wide
// width whenever it is mounted, the same cap wide mode uses.
const root = path.resolve(import.meta.dirname, "../..");
const app = fs.readFileSync(path.join(root, "src/styles/app.css"), "utf8");
const conflicts = fs.readFileSync(path.join(root, "src/styles/conflicts.css"), "utf8");

const maxWidth = (css: string, selector: RegExp) =>
  css.match(selector)?.[1].match(/max-width:\s*([^;]+);/)?.[1];

describe("Concord review width", () => {
  it("uses the pane's wide content cap whenever the resolver is mounted", () => {
    const wide = maxWidth(app, /^\.wide-mode \.main-content-inner\s*\{([^}]*)\}/m);
    expect(wide).toBeTruthy();
    expect(maxWidth(conflicts, /^\.main-content-inner:has\(\.page-conflict\)\s*\{([^}]*)\}/m)).toBe(wide);
  });

  it("the width rule keys on the class the resolver actually renders", () => {
    const source = fs.readFileSync(path.join(root, "src/components/ConflictResolution.tsx"), "utf8");
    expect(source).toContain('class="page-conflict"');
  });
});
