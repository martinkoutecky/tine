import { readFileSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { expect, it } from "vitest";

it("keeps production interface comments complete (Rule 2)", () => {
  const root = resolve("src");
  const truncated: string[] = [];
  const visit = (directory: string) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(path);
      } else if (/\.(?:ts|tsx)$/.test(entry.name) && !/\.test\.(?:ts|tsx)$/.test(entry.name)) {
        const source = readFileSync(path, "utf8");
        for (const match of source.matchAll(/\/\*\*[\s\S]*?\*\//g)) {
          if (/(?:…|\.\.\.)\s*\*\/$/.test(match[0])) {
            const line = source.slice(0, match.index).split("\n").length;
            truncated.push(`${relative(root, path)}:${line}`);
          }
        }
      }
    }
  };
  visit(root);
  expect(truncated, `Rule 2: truncated production doc comments at ${truncated.join(", ")}`).toEqual([]);
});
