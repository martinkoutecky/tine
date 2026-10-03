import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// GH #619: Solid's <Portal> keeps the LOGICAL parent in the event path, so every
// press/key on a floating surface reached the block that rendered it. The only
// sanctioned way to float a surface is `FloatingPortal` (src/components/FloatingPortal.tsx,
// the blessed exemplar). This scan fails when any other file imports `Portal` from
// `solid-js/web` (statically or via `import()`), or calls `createPortal`.
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

export function portalViolations(file: string, source: string): string[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const violations: string[] = [];
  const report = (node: ts.Node, message: string) => {
    const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
    violations.push(`${file}:${line + 1}: ${message}`);
  };
  const visit = (node: ts.Node) => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier) &&
      node.moduleSpecifier.text === "solid-js/web") {
      const bindings = node.importClause?.namedBindings;
      if (bindings && ts.isNamedImports(bindings)) {
        for (const element of bindings.elements) {
          const imported = (element.propertyName ?? element.name).text;
          if (imported === "Portal" || imported === "createPortal") report(element, `imports ${imported} from solid-js/web`);
        }
      }
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
  });
});
