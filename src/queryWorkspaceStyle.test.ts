import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const root = path.resolve(import.meta.dirname, "..");
const app = fs.readFileSync(path.join(root, "src/styles/app.css"), "utf8");

describe("persistent search result geometry (GH #140)", () => {
  it("allows both Search renderers and the persistent grid chain to shrink and wrap", () => {
    const selectors = [
      /\.query-result-row\s*\{([^}]*)\}/s,
      /\.query-search-hit,\s*\n\.query-search-page\s*\{([^}]*)\}/s,
    ];
    for (const selector of selectors) {
      const rule = app.match(selector)?.[1] ?? "";
      expect(rule).toContain("box-sizing: border-box");
      expect(rule).toContain("max-width: 100%");
      expect(rule).toContain("min-width: 0");
      expect(rule).toContain("white-space: normal");
    }

    const workspace = app.match(/\.query-workspace\s*\{([^}]*)\}/s)?.[1] ?? "";
    expect(workspace).toContain("max-width: 100%");
    expect(workspace).toContain("min-width: 0");

    const grid = app.match(/\.query-results-search,\s*\n\.query-results-list\s*\{([^}]*)\}/s)?.[1] ?? "";
    expect(grid).toContain("grid-template-columns: minmax(0, 1fr)");
    expect(grid).toContain("max-width: 100%");
    expect(grid).toContain("min-width: 0");

    const item = app.match(/\.query-results-search\s*>\s*\[role="listitem"\]\s*\{([^}]*)\}/s)?.[1] ?? "";
    expect(item).toContain("max-width: 100%");
    expect(item).toContain("min-width: 0");
  });
});
