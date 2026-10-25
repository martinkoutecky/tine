import { expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import ts from "typescript";

// No reader exceptions. The definition is the only allowed site (nothing re-exports
// the counter, so a renamed re-export is itself a violation); full refreshes flow
// through changeKeys for every reader.
const allowed = new Map([
  ["dataRevision.ts", "Defines the full-fallback counter and publication adapter."],
]);

it("production readers cannot subscribe to dataRev directly, including aliases and namespace access", () => {
  const root = new URL(".", import.meta.url).pathname;
  const violations: string[] = [];
  function visitDirectory(directory: string) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const file = join(directory, entry.name);
      if (entry.isDirectory()) { visitDirectory(file); continue; }
      if (!/\.tsx?$/.test(file) || /\.(test|support)\./.test(file)) continue;
      const name = relative(root, file);
      if (allowed.has(name)) continue;
      const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true);
      function visit(node: ts.Node) {
        if ((ts.isIdentifier(node) || ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) && ["dataRev", "setDataRev", "dataRevision"].includes(node.text)) {
          violations.push(`${name}:${source.getLineAndCharacterOfPosition(node.getStart()).line + 1}: ${node.text}`);
        }
        ts.forEachChild(node, visit);
      }
      visit(source);
    }
  }
  visitDirectory(root);
  expect(violations).toEqual([]);
});
