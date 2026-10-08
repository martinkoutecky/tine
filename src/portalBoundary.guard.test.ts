import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// GH #619: Solid's <Portal> keeps the LOGICAL parent in the event path, so every
// press/key on a floating surface reached the block that rendered it. The only
// sanctioned way to float a surface is `FloatingPortal` (src/components/FloatingPortal.tsx,
// the blessed exemplar). This scan fails when any other file imports `Portal` from
// `solid-js/web` (by name, namespace, re-export, `import()` or `require()`), or calls `createPortal`.
const ALLOWED = new Set(["src/components/FloatingPortal.tsx"]);
// The surfaces that were found portalled from inside a block/editor/tab when the rule was written.
const EXPECTED_USERS = [
  "src/components/EditorAutocomplete.tsx",
  "src/components/QueryBuilder.tsx",
  "src/components/TabBar.tsx",
  "src/render/PeekPopup.tsx",
];

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(file);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [file] : [];
  });
}

const PORTAL_NAMES = new Set(["Portal", "createPortal"]);
const SOLID_WEB = "solid-js/web";

export function portalViolations(file: string, source: string): string[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const violations: string[] = [];
  const report = (node: ts.Node, message: string) => {
    const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
    violations.push(`${file}:${line + 1}: ${message}`);
  };
  const isSolidWeb = (node: ts.Expression | undefined) =>
    node !== undefined && ts.isStringLiteralLike(node) && node.text === SOLID_WEB;
  // Local names bound to the whole solid-js/web namespace (`import * as W`).
  const namespaces = new Set<string>();
  for (const statement of sf.statements) {
    if (ts.isImportDeclaration(statement) && isSolidWeb(statement.moduleSpecifier)) {
      const bindings = statement.importClause?.namedBindings;
      if (bindings && ts.isNamespaceImport(bindings)) namespaces.add(bindings.name.text);
    }
  }
  const visit = (node: ts.Node) => {
    if (ts.isImportDeclaration(node) && isSolidWeb(node.moduleSpecifier)) {
      const bindings = node.importClause?.namedBindings;
      if (bindings && ts.isNamedImports(bindings)) {
        for (const element of bindings.elements) {
          const imported = (element.propertyName ?? element.name).text;
          if (PORTAL_NAMES.has(imported)) report(element, `imports ${imported} from solid-js/web`);
        }
      }
    }
    // `export { Portal } from "solid-js/web"` / `export * from "solid-js/web"` re-expose it.
    if (ts.isExportDeclaration(node) && isSolidWeb(node.moduleSpecifier)) {
      if (!node.exportClause) report(node, "re-exports everything from solid-js/web");
      else if (ts.isNamedExports(node.exportClause)) {
        for (const element of node.exportClause.elements) {
          if (PORTAL_NAMES.has((element.propertyName ?? element.name).text)) report(element, `re-exports ${(element.propertyName ?? element.name).text} from solid-js/web`);
        }
      }
    }
    // `import("solid-js/web")` and `require("solid-js/web")` hand back the namespace
    // with no static binding to inspect, so they are refused outright: import the
    // names you need statically.
    if (ts.isCallExpression(node) && node.arguments.length > 0 && isSolidWeb(node.arguments[0]) &&
      (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
        (ts.isIdentifier(node.expression) && node.expression.text === "require"))) {
      report(node, "dynamic import of solid-js/web");
    }
    // `W.Portal` / `W["createPortal"]` through a namespace import.
    if (ts.isPropertyAccessExpression(node) && ts.isIdentifier(node.expression) && namespaces.has(node.expression.text) &&
      PORTAL_NAMES.has(node.name.text)) report(node, `${node.expression.text}.${node.name.text} through a namespace import`);
    if (ts.isElementAccessExpression(node) && ts.isIdentifier(node.expression) && namespaces.has(node.expression.text) &&
      ts.isStringLiteralLike(node.argumentExpression) && PORTAL_NAMES.has(node.argumentExpression.text)) {
      report(node, `${node.expression.text}[${node.argumentExpression.text}] through a namespace import`);
    }
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "createPortal") {
      report(node, "createPortal call");
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return violations;
}

describe("portal boundary guard (GH #619)", () => {
  it("floats every surface through FloatingPortal, never Solid's bare Portal", () => {
    const root = process.cwd();
    const violations = sourceFiles(path.join(root, "src")).flatMap((absolute) => {
      const relative = path.relative(root, absolute).replaceAll(path.sep, "/");
      if (ALLOWED.has(relative)) return [];
      return portalViolations(relative, readFileSync(absolute, "utf8"));
    });
    expect(
      violations,
      "RULE (GH #619): a floating surface must be rendered with <FloatingPortal> from " +
        "src/components/FloatingPortal.tsx (the blessed exemplar), not Solid's <Portal>: the bare Portal " +
        "forwards every press and key to the block that rendered it and swaps the surface out from under the user.",
    ).toEqual([]);
  });

  it("every enumerated floating surface uses FloatingPortal", () => {
    for (const file of EXPECTED_USERS) {
      const source = readFileSync(file, "utf8");
      expect(source, file).toMatch(/<FloatingPortal[\s>]/);
    }
  });

  it("detects the violation shapes it claims to (necessity)", () => {
    expect(portalViolations("x.tsx", `import { Portal } from "solid-js/web";`)).toHaveLength(1);
    expect(portalViolations("x.tsx", `import { render, Portal as P } from "solid-js/web";`)).toHaveLength(1);
    expect(portalViolations("x.tsx", `import { render } from "solid-js/web";`)).toHaveLength(0);
    // Shapes that bypass a named-import scan.
    expect(portalViolations("x.tsx", `import * as W from "solid-js/web"; const P = W.Portal;`)).toHaveLength(1);
    expect(portalViolations("x.tsx", `import * as W from "solid-js/web"; const P = W["createPortal"];`)).toHaveLength(1);
    expect(portalViolations("x.tsx", `const { Portal } = await import("solid-js/web");`)).toHaveLength(1);
    expect(portalViolations("x.tsx", `const W = require("solid-js/web");`)).toHaveLength(1);
    expect(portalViolations("x.tsx", `export { Portal } from "solid-js/web";`)).toHaveLength(1);
    expect(portalViolations("x.tsx", `export * from "solid-js/web";`)).toHaveLength(1);
    // Legitimate neighbours stay clean.
    expect(portalViolations("x.tsx", `import * as W from "solid-js/web"; W.render(a, b);`)).toHaveLength(0);
    expect(portalViolations("x.tsx", `export { render } from "solid-js/web";`)).toHaveLength(0);
    expect(portalViolations("x.tsx", `const m = await import("./other");`)).toHaveLength(0);
  });
});
