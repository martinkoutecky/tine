import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// One name answerer: the store. The frontend caches `page_inventory` only in
// src/pageIndex.ts and answers names only through it; imitate that file.
const RULE = "one name answerer: the store; the frontend caches `page_inventory` only in pageIndex.ts " +
  "(imitate src/pageIndex.ts: look a name up there, never build a name/alias map)";

const INVENTORY_OWNER = "src/pageIndex.ts";
// `resolvePage` answers ONE name for a save that has no file id yet (B15b): the
// first save of a working-set page, journal seed/template, QuickSwitcher Create
// and query materialize. It never feeds a name map.
const RESOLVE_FOR_SAVE = new Set([
  "src/persistence.ts",
  "src/graph.ts",
  "src/components/QuickSwitcher.tsx",
  "src/components/QueryWorkspace.tsx",
]);
const DEFINITIONS = new Set(["src/backend.ts", "src/mock.ts"]);
const FORBIDDEN_NAMES = new Set(["aliasMap", "setAliasMap", "resolveAlias"]);
const DELETED_METHODS = new Set(["listPages", "pageAliases", "referencedPageNames"]);
const DELETED_COMMANDS = new Set(["list_pages", "page_aliases", "referenced_page_names"]);
const ITERATORS = new Set(["map", "flatMap", "forEach", "reduce", "filter", "some", "every", "all", "allSettled"]);

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(file);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [file] : [];
  });
}

function insideIteration(node: ts.Node): boolean {
  for (let at: ts.Node | undefined = node.parent; at; at = at.parent) {
    if (ts.isForStatement(at) || ts.isForOfStatement(at) || ts.isForInStatement(at) ||
      ts.isWhileStatement(at) || ts.isDoStatement(at)) return true;
    if (ts.isCallExpression(at) && ts.isPropertyAccessExpression(at.expression) &&
      ITERATORS.has(at.expression.name.text)) return true;
    if (ts.isFunctionDeclaration(at) || ts.isMethodDeclaration(at)) return false;
  }
  return false;
}

export function nameAnswererViolations(file: string, source: string): string[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const violations: string[] = [];
  const report = (node: ts.Node, message: string) => {
    const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
    violations.push(`${file}:${line + 1}: ${message} — ${RULE}`);
  };
  const visit = (node: ts.Node) => {
    if (ts.isIdentifier(node) && FORBIDDEN_NAMES.has(node.text)) {
      report(node, `frontend name map \`${node.text}\``);
    }
    if (ts.isIdentifier(node) && DELETED_METHODS.has(node.text)) {
      report(node, `deleted backend method \`${node.text}\``);
    }
    if (ts.isStringLiteralLike(node) && DELETED_COMMANDS.has(node.text)) {
      report(node, `deleted command "${node.text}"`);
    }
    if (ts.isStringLiteralLike(node) && node.text === "page_inventory" && file !== "src/backend.ts") {
      report(node, "page_inventory invoked outside backend.ts");
    }
    if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)) {
      const method = node.expression.name.text;
      if (method === "pageInventory" && file !== INVENTORY_OWNER) {
        report(node, "pageInventory() called outside pageIndex.ts");
      }
      if (method === "resolvePage") {
        if (!RESOLVE_FOR_SAVE.has(file) && !DEFINITIONS.has(file)) {
          report(node, "resolvePage() used as a name answerer outside the save callers");
        } else if (insideIteration(node)) {
          report(node, "resolvePage() called per name in a loop (a name map)");
        }
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return violations;
}

describe("page index guard (Rule 3: one answerer, frontend included)", () => {
  it("keeps page_inventory in pageIndex.ts and no other frontend name map", () => {
    const root = process.cwd();
    const violations = sourceFiles(path.join(root, "src")).flatMap((absolute) => {
      const relative = path.relative(root, absolute).replaceAll(path.sep, "/");
      return nameAnswererViolations(relative, readFileSync(absolute, "utf8"));
    });
    expect(violations).toEqual([]);
  });

  it("rejects planted maps, deleted commands and stray inventory/resolve callers", () => {
    const planted = [
      'export const [aliasMap, setAliasMap] = createSignal({});',
      'const pages = await backend().listPages();',
      'invoke("page_aliases");',
      'const inv = await backend().pageInventory();',
      'const target = await backend().resolvePage(name, "page");',
    ].join("\n");
    expect(nameAnswererViolations("src/components/Planted.tsx", planted)).toHaveLength(6);
    const loop = 'for (const n of names) await backend().resolvePage(n, "page");\n' +
      'await Promise.all(names.map((n) => backend().resolvePage(n, "page")));';
    expect(nameAnswererViolations("src/persistence.ts", loop)).toHaveLength(2);
    expect(nameAnswererViolations("src/pageIndex.ts", "await backend().pageInventory();")).toEqual([]);
  });
});
