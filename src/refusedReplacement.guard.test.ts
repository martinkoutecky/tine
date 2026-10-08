// GH #254 family (master 7bd793bd0, og J1; I-2, I-20): a page load the working
// set declines returns a typed refusal, and every caller must act on it. A call
// whose answer is thrown away, or `loadFeed`'s status read for truthiness,
// treats a refused replacement as success: content lands in (or is shown from)
// another file holding the same name.
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { expect, it } from "vitest";

const RULE = "I-2/I-20 (GH #254 family, og J1): a page load's refusal must be consumed — bind it, branch on it, or return it; " +
  "never discard it, and compare loadFeed's status instead of testing its truthiness. " +
  "Exemplar: src/document/edits/capture.ts (admitPageFile) and src/components/Page.tsx (loadFeed).";

/** Doors whose result says whether the requested file took its name slot. */
const DOORS = new Set(["ensurePageLoaded", "loadRoutedPage", "loadSingle", "loadFeed", "appendFeed", "restoreTodayJournalInFeed", "admitPageFile", "upsertUnlessDirty"]);
/** Doors whose every outcome is truthy, so a truthiness test cannot tell them apart. */
const ALWAYS_TRUTHY = new Set(["loadFeed"]);

function calleeName(call: ts.CallExpression): string | null {
  const e = call.expression;
  if (ts.isIdentifier(e)) return e.text;
  if (ts.isPropertyAccessExpression(e)) return e.name.text;
  return null;
}

/** The expression the call's value flows into, past await/parentheses/ternary branches. */
function consumer(node: ts.Node): ts.Node {
  let at: ts.Node = node;
  for (;;) {
    const up = at.parent;
    if (ts.isAwaitExpression(up) || ts.isParenthesizedExpression(up) || ts.isAsExpression(up) || ts.isNonNullExpression(up)
      || (ts.isConditionalExpression(up) && up.condition !== at)) { at = up; continue; }
    return at;
  }
}

function isBooleanUse(at: ts.Node): boolean {
  const up = at.parent;
  if (ts.isPrefixUnaryExpression(up) && up.operator === ts.SyntaxKind.ExclamationToken) return true;
  if ((ts.isIfStatement(up) || ts.isWhileStatement(up)) && up.expression === at) return true;
  if (ts.isConditionalExpression(up) && up.condition === at) return true;
  return ts.isBinaryExpression(up) && [ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken].includes(up.operatorToken.kind);
}

function violations(fileName: string, source: string): string[] {
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true, fileName.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node)) {
      const name = calleeName(node);
      if (name && DOORS.has(name)) {
        const at = consumer(node);
        const up = at.parent;
        const line = file.getLineAndCharacterOfPosition(node.getStart()).line + 1;
        const discarded = ts.isExpressionStatement(up) || ts.isVoidExpression(up)
          || (ts.isArrowFunction(up) && up.body === at && isVoidCallback(up));
        if (discarded) found.push(`${fileName}:${line}: ${name}(...) result discarded`);
        else if (ALWAYS_TRUTHY.has(name) && isBooleanUse(at)) found.push(`${fileName}:${line}: ${name}(...) tested for truthiness`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

/** An arrow whose value nobody reads: a `.then`/`.forEach` callback or event handler body. */
function isVoidCallback(arrow: ts.ArrowFunction): boolean {
  const call = arrow.parent;
  return ts.isCallExpression(call) && ts.isPropertyAccessExpression(call.expression) && ["forEach", "then", "finally"].includes(call.expression.name.text);
}

function productionFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return productionFiles(full);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) && !entry.name.endsWith(".d.ts") ? [full] : [];
  });
}

it("every production caller consumes a page load's refusal", () => {
  const found = productionFiles("src").flatMap((file) => violations(file, readFileSync(file, "utf8")));
  expect(found, RULE).toEqual([]);
});

it("the scan still sees the doors it guards", () => {
  // A rename that left DOORS stale would make the guard vacuous.
  const uses = productionFiles("src").map((file) => readFileSync(file, "utf8"));
  for (const door of DOORS) expect(uses.some((text) => text.includes(`${door}(`)), `${door} has no production call; update DOORS`).toBe(true);
});

it("rejects planted discards and truthiness tests", () => {
  const planted = [
    "ensurePageLoaded(dto);",
    "void loadSingle(dto);",
    "await admitPageFile(name, kind, owner, empty);",
    "items.forEach((d) => ensurePageLoaded(d));",
    "if (!loadFeed(pages)) return;",
    "const ok = loadFeed(pages) && ready;",
    "doc.restoreTodayJournalInFeed();",
  ];
  for (const line of planted) expect(violations("planted.ts", `async function f() { ${line} }`), line).toHaveLength(1);
});

it("accepts consumed results", () => {
  const consumed = [
    "const refusal = ensurePageLoaded(dto);",
    "if (ensurePageLoaded(dto)) return;",
    "if (ready && ensurePageLoaded(dto)) return false;",
    "const admitted = await admitPageFile(name, kind, owner, empty);",
    "return loadRoutedPage(dto);",
    "const r = flag ? ensurePageLoaded(d) : upsertUnlessDirty(d);",
    "for (const refusal of appendFeed(pages)) report(refusal);",
    "const loaded = loadFeed(pages); if (loaded !== \"published\") return;",
    "// ensurePageLoaded(dto); in a comment",
  ];
  for (const line of consumed) expect(violations("consumed.ts", `async function f() { ${line} }`), line).toEqual([]);
});
