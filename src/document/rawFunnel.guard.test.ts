import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// BIGLOAD-090b: outline windows learn about raw edits from the one notification
// in model.ts (`onBlockRawChanged`) instead of one tracked computation per row.
// That holds only if every block raw write goes through it: setDoc's field form
// `setDoc("byId", id, "raw", v)` or `writeRaw(state, id, v)` inside a producer.
// Node replacement and removal are structure, which windows observe directly.

function productionSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? productionSources(file) : /\.tsx?$/.test(file) && !/\.test(?:\.support)?\.tsx?$/.test(file) ? [file] : [];
  });
}

// Exact `.raw` assignment counts outside the funnel, each with its reason.
const RAW_ASSIGNMENTS: Record<string, { count: number; why: string }> = {
  "src/document/model.ts": { count: 1, why: "writeRaw itself: the funnel's in-place assignment" },
  "src/editor/outline.ts": { count: 2, why: "local paste-parse OutlineNode objects, never store nodes" },
};

const isRawName = (name: ts.Node) =>
  (ts.isIdentifier(name) || ts.isStringLiteral(name)) && name.text === "raw";

/** Violations in one file: `.raw` assignments, setDoc forms that can write a
 * node's raw without the field form, and Object.assign of a `raw` property. */
function check(file: string, source: string): { rawAssignments: number; violations: string[] } {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, file.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const at = (node: ts.Node) => `${file}:${sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1}`;
  const violations: string[] = [];
  let rawAssignments = 0;
  const visit = (node: ts.Node) => {
    if (ts.isBinaryExpression(node) && node.operatorToken.kind >= ts.SyntaxKind.FirstAssignment &&
      node.operatorToken.kind <= ts.SyntaxKind.LastAssignment) {
      const left = node.left;
      if ((ts.isPropertyAccessExpression(left) && isRawName(left.name)) ||
        (ts.isElementAccessExpression(left) && isRawName(left.argumentExpression))) rawAssignments++;
    }
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "setDoc") {
      const [first, , field] = node.arguments;
      const whole = first && (ts.isObjectLiteralExpression(first) || (ts.isCallExpression(first) && ts.isIdentifier(first.expression) && first.expression.text === "produce"));
      const path = first && ts.isStringLiteral(first);
      if (whole ? node.arguments.length !== 1 : !path) violations.push(`${at(node)} setDoc must take one produce()/state object or a literal path`);
      else if (path && first.text === "byId" && (node.arguments.length < 4 || !field || !ts.isStringLiteral(field)))
        violations.push(`${at(node)} setDoc("byId", ...) must name a literal field (a merge can rewrite raw unnotified)`);
    }
    const parent = node.parent;
    if (ts.isIdentifier(node) && node.text === "setDoc" && !(ts.isCallExpression(parent) && parent.expression === node) &&
      !ts.isImportSpecifier(parent) && !ts.isExportSpecifier(parent) && !(ts.isVariableDeclaration(parent) && parent.name === node))
      violations.push(`${at(node)} setDoc may only be called directly`);
    if (ts.isCallExpression(node) && node.expression.getText(sf) === "Object.assign" &&
      node.arguments.some((arg) => ts.isObjectLiteralExpression(arg) && arg.properties.some((p) => p.name && isRawName(p.name))))
      violations.push(`${at(node)} Object.assign of raw bypasses the raw funnel`);
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return { rawAssignments, violations };
}

describe("block raw writes go through the raw-change funnel (BIGLOAD-090b)", () => {
  it("production code writes raw only through setDoc's field form or writeRaw", () => {
    const counts: Record<string, number> = {};
    const violations: string[] = [];
    for (const file of productionSources("src")) {
      const result = check(file, readFileSync(file, "utf8"));
      if (result.rawAssignments) counts[file] = result.rawAssignments;
      violations.push(...result.violations);
    }
    expect(violations).toEqual([]);
    expect(counts).toEqual(Object.fromEntries(Object.entries(RAW_ASSIGNMENTS).map(([file, { count }]) => [file, count])));
  });

  it("catches planted bypasses", () => {
    const planted = [
      "setDoc(produce((s) => { s.byId[id].raw = next; }));",
      "setDoc(produce((s) => { s.byId[id][\"raw\"] = next; }));",
      "setDoc(\"byId\", id, { raw: next });",
      "setDoc(\"byId\", { [id]: node });",
      "setDoc(path, value);",
      "const write = setDoc; write(\"byId\", id, \"raw\", next);",
      "batch(setDoc);",
      "setDoc(produce((s) => { Object.assign(s.byId[id], { raw: next }); }));",
    ];
    for (const line of planted) {
      const result = check("src/planted.ts", line);
      expect(result.rawAssignments + result.violations.length, line).toBeGreaterThan(0);
    }
    expect(check("src/planted.ts", "setDoc(\"byId\", id, \"raw\", next); setDoc(produce((s) => writeRaw(s, id, next)));"))
      .toEqual({ rawAssignments: 0, violations: [] });
  });
});
