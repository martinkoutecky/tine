import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// One answerer for "this page's properties" (og 14 Q5, Rule 3). The grammar is
// editor/properties.ts `pagePropertyEntries`; a loaded page's properties are
// read ONLY through document/edits/properties.ts `pageHeaderProperties` /
// `readPageProperties` / `readPageProperty`, which also decide which source
// (pre-block or header root) the writer targets. A second parser of a page's
// pre-block drifts from what the header renders and what the panel edits
// (SheetTable/SheetBoard did: they skipped the properties-only first root).
const RULE = "one page-property answerer: read a loaded page's properties with readPageProperties / " +
  "readPageProperty / pageHeaderProperties (imitate src/components/SheetBoard.tsx `pageFormulas`), " +
  "never by parsing its preBlock";

// Files allowed to call the raw grammar, and why.
const GRAMMAR_CALLERS: Record<string, readonly string[]> = {
  // The grammar itself and its writer (pagePartsWithProperty).
  "src/editor/properties.ts": ["pagePropertyEntries"],
  // The renderer's `[key, value]` view of the grammar.
  "src/render/block.ts": ["pagePropertyEntries", "pageProperties"],
  // The one answerer over a loaded page.
  "src/document/edits/properties.ts": ["pagePropertyEntries"],
  // A save DTO's title, before (or without) a loaded page.
  "src/document/save/engine.ts": ["pagePropertyEntries"],
  // A backend search hit (`page_property` block raw); no loaded page exists.
  "src/components/RefBlocks.tsx": ["pageProperties"],
};
const GRAMMAR = new Set(["pagePropertyEntries", "pageProperties"]);
// Other property parsers that must never be pointed at a page's pre-block.
const OTHER_PARSERS = new Set(["facetsOf", "splitProps", "parsePageHeaderPropertyLine"]);
const DELETED = new Set(["aliasNames"]);

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(file);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [file] : [];
  });
}

function mentionsPreBlock(node: ts.Node): boolean {
  let found = false;
  const visit = (n: ts.Node) => {
    if ((ts.isIdentifier(n) || ts.isPrivateIdentifier(n)) && /^(preBlock|pre_block)$/.test(n.text)) found = true;
    if (!found) ts.forEachChild(n, visit);
  };
  visit(node);
  return found;
}

export function pagePropertyAnswererViolations(file: string, source: string): string[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const allowed = new Set(GRAMMAR_CALLERS[file] ?? []);
  const violations: string[] = [];
  const report = (node: ts.Node, message: string) => {
    const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
    violations.push(`${file}:${line + 1}: ${message} — ${RULE}`);
  };
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node)) {
      const callee = ts.isIdentifier(node.expression) ? node.expression.text
        : ts.isPropertyAccessExpression(node.expression) ? node.expression.name.text : "";
      if (GRAMMAR.has(callee) && !allowed.has(callee)) {
        report(node, `page-property grammar \`${callee}\` called outside its owners`);
      }
      if (OTHER_PARSERS.has(callee) && node.arguments.some(mentionsPreBlock)) {
        report(node, `\`${callee}\` parses a page pre-block`);
      }
      if (ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === "exec" &&
        ts.isIdentifier(node.expression.expression) && node.expression.expression.text === "PROP_LINE" &&
        node.arguments.some(mentionsPreBlock)) {
        report(node, "`PROP_LINE` run over a page pre-block");
      }
    }
    if (ts.isIdentifier(node) && DELETED.has(node.text)) report(node, `deleted pre-block parser \`${node.text}\``);
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return violations;
}

describe("one page-property answerer (og 14 Q5)", () => {
  it("no production file parses page properties outside the one answerer", () => {
    const violations = sourceFiles("src").flatMap((file) => {
      const rel = file.split(path.sep).join("/");
      return pagePropertyAnswererViolations(rel, readFileSync(file, "utf8"));
    });
    expect(violations).toEqual([]);
  });

  it("every allowlisted grammar caller still calls what it is allowed to", () => {
    for (const [file, names] of Object.entries(GRAMMAR_CALLERS)) {
      const source = readFileSync(file, "utf8");
      for (const name of names) expect(source.includes(`${name}(`), `${file} no longer calls ${name}; shrink the allowlist`).toBe(true);
    }
  });

  it("catches a second pre-block parser (the SheetTable shape)", () => {
    const sheet = "const f = formulasOf(pageProperties(page.preBlock, page.format));";
    expect(pagePropertyAnswererViolations("src/components/SheetTable.tsx", sheet)).toHaveLength(1);
    expect(pagePropertyAnswererViolations("src/x.ts", "facetsOf(page.preBlock, 'md')")).toHaveLength(1);
    expect(pagePropertyAnswererViolations("src/x.ts", "PROP_LINE.exec(dto.pre_block ?? '')")).toHaveLength(1);
    expect(pagePropertyAnswererViolations("src/x.ts", "aliasNames(page.preBlock)").length).toBeGreaterThan(0);
    expect(pagePropertyAnswererViolations("src/x.ts", "formulasOf(readPageProperties(name))")).toEqual([]);
  });
});
