import { execFileSync } from "node:child_process";
import { describe, expect, it } from "vitest";

// Windows and macOS check out onto case-insensitive filesystems. There, two tracked paths that
// differ only by case become one file, and two modules whose extensionless import specifiers
// differ only by case resolve to whichever extension the bundler tries first:
// src/render/RelativeFileLink.tsx next to relativeFileLink.ts made `import "./RelativeFileLink"`
// load the .ts module on Windows, and the hosted Windows build failed while every Linux gate
// stayed green. Exemplar of the fix: RelativeFileLinkView.tsx.
const MODULE_EXTENSION = /\.(?:d\.ts|tsx?|jsx?|mjs|cjs)$/;

const KNOWN_PAIRS = new Set([
  // Two different documents; neither is compiled or shipped. Merge them, then drop this entry.
  "docs/ui-regression-testing.md",
]);

describe("tracked paths", () => {
  const paths = execFileSync("git", ["ls-files", "-z"], { encoding: "utf8" }).split("\0").filter(Boolean);

  function collisions(keyOf: (path: string) => string | null): string[] {
    const seen = new Map<string, string>();
    const found: string[] = [];
    for (const path of paths) {
      const key = keyOf(path);
      if (key === null) continue;
      const other = seen.get(key);
      if (other !== undefined && other !== path && !KNOWN_PAIRS.has(key)) found.push(`${other} <-> ${path}`);
      seen.set(key, path);
    }
    return found;
  }

  it("never differ only by letter case", () => {
    expect(
      collisions((path) => path.toLowerCase()),
      "rename one of each pair: case-insensitive checkouts (Windows, macOS) merge them",
    ).toEqual([]);
  });

  it("never give two modules import specifiers that differ only by letter case", () => {
    // Same spelling with different extensions (foo.ts + foo.d.ts) is one specifier, not a clash.
    const spelling = new Map<string, string>();
    const found: string[] = [];
    for (const path of paths) {
      if (!MODULE_EXTENSION.test(path) || /\.test\.[jt]sx?$/.test(path)) continue;
      const bare = path.replace(MODULE_EXTENSION, "");
      const other = spelling.get(bare.toLowerCase());
      if (other !== undefined && other !== bare) found.push(`${other} <-> ${bare}`);
      spelling.set(bare.toLowerCase(), bare);
    }
    expect(found, "rename one module: on Windows and macOS the extensionless import resolves to the other file").toEqual([]);
  });
});
