import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";
import { VIEW_KINDS } from "./editor/queryIr";
import { normalizeQueryPresentation } from "./router";

// I-12: the four query presentations are spelled once, as `VIEW_KINDS` in
// `src/editor/queryIr.ts`. `QueryPresentation`, `QueryView`, the router and session
// normalizer, and every picker derive from it, so a fifth presentation is one edit.
// This scan fails on any other array literal naming those four strings (blessed
// exemplar: `Macro.tsx` and `QueryWorkspace.tsx` iterate `VIEW_KINDS`).
const OWNER = "src/editor/queryIr.ts";

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(file);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [file] : [];
  });
}

export function presentationListViolations(file: string, source: string): string[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const out: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isArrayLiteralExpression(node)) {
      const names = new Set(node.elements.filter(ts.isStringLiteralLike).map((e) => e.text));
      if (VIEW_KINDS.every((kind) => names.has(kind))) {
        out.push(`${file}:${sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1}: spells the query presentations; use VIEW_KINDS (${OWNER})`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return out;
}

describe("query presentations have one list", () => {
  it("only queryIr.ts spells them", () => {
    const violations = sourceFiles("src").map((f) => f.split(path.sep).join("/"))
      .filter((f) => f !== OWNER)
      .flatMap((f) => presentationListViolations(f, readFileSync(f, "utf8")));
    expect(violations).toEqual([]);
  });
  it("the scanner sees a reordered or aliased copy", () => {
    expect(presentationListViolations("x.ts", 'const a = ["board", "table", "list", "search"];')).toHaveLength(1);
    expect(presentationListViolations("x.ts", 'const a = ["list", "table"];')).toEqual([]);
  });
  it("the router accepts exactly the listed presentations", () => {
    for (const kind of VIEW_KINDS) expect(normalizeQueryPresentation(kind)).toBe(kind);
    expect(normalizeQueryPresentation("grid")).toBeNull();
  });
});
