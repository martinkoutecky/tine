import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

function productionSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? productionSources(file) : /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}

// The 10b/10c migration boundary. Function keys are independent of line shifts.
// This list may only shrink.
const ORIGINAL_UNMIGRATED = new Set<string>(JSON.parse(readFileSync("src/lateLanding.original.json", "utf8")));
const UNMIGRATED: Record<string, string> = JSON.parse(readFileSync("src/lateLanding.unmigrated.json", "utf8"));
const EXEMPT: Record<string, string> = {
  "src/components/Block.tsx#Editor.capturePhotoCmd": "asset editor token checks binding before insertion and every error toast",
  "src/components/Block.tsx#Editor.voiceMemoToggle": "native recorder must be cancelled from a stale start result; editor token guards insertion and toasts",
  "src/debug.ts#initDebug": "one-time device debug probe has no graph or route landing",
  "src/graph.ts#loadGraphPath": "the graph transition changes its own binding; its transition lock owns publication",
  "src/plugins/manager.ts#uninstall": "device-local plugin removal completes in the process-wide manager across graph navigation",
  "src/plugins/registry.ts#loadVerifiedCachedRegistry": "one-time verified cache migration uses native expected-legacy comparison before publication",
};
// Device-local app preferences are owned by preferenceWrites' write revision.
const DEVICE_LOCAL = /^(get|set)App[A-Z]/;

function functionName(node: ts.Node): string {
  const names: string[] = [];
  for (let parent = node.parent; parent; parent = parent.parent) {
    if (ts.isFunctionDeclaration(parent) && parent.name) names.push(parent.name.text);
    if (ts.isMethodDeclaration(parent) && parent.name) names.push(parent.name.getText());
    if ((ts.isArrowFunction(parent) || ts.isFunctionExpression(parent)) && parent.parent) {
      const declaration = parent.parent;
      if (ts.isVariableDeclaration(declaration) && ts.isIdentifier(declaration.name)) names.push(declaration.name.text);
      if (ts.isPropertyAssignment(declaration)) names.push(declaration.name.getText());
    }
  }
  return names.reverse().join(".") || "<module>";
}

/** Report backend completions outside the owned-result boundary. */
export function lateLandingViolations(file: string, source: string): string[] {
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const aliases = new Set(["api", "deps"]);
  const collectAliases = (node: ts.Node): void => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer &&
      ts.isCallExpression(node.initializer) && ts.isIdentifier(node.initializer.expression) &&
      node.initializer.expression.text === "backend") aliases.add(node.name.text);
    ts.forEachChild(node, collectAliases);
  };
  collectAliases(tree);
  const backendMethod = (node: ts.Node): string | null => {
    if (!ts.isCallExpression(node) || !ts.isPropertyAccessExpression(node.expression)) return null;
    const receiver = node.expression.expression;
    const direct = ts.isCallExpression(receiver) && ts.isIdentifier(receiver.expression) && receiver.expression.text === "backend";
    if (direct || (ts.isIdentifier(receiver) && aliases.has(receiver.text))) return node.expression.name.text;
    return null;
  };
  const backendCalls = (node: ts.Node): string[] => {
    const found: string[] = [];
    const walk = (child: ts.Node): void => {
      if (ts.isCallExpression(child) && ts.isIdentifier(child.expression) &&
        (child.expression.text === "readOwned" || child.expression.text === "serializeOwned")) return;
      const method = backendMethod(child);
      if (method && !DEVICE_LOCAL.test(method)) found.push(method);
      ts.forEachChild(child, walk);
    };
    walk(node);
    return found;
  };
  const guarded = (node: ts.Node): boolean => {
    if (ts.isAwaitExpression(node) && ts.isCallExpression(node.expression) &&
      ts.isIdentifier(node.expression.expression) &&
      (node.expression.expression.text === "readOwned" || node.expression.expression.text === "serializeOwned")) return true;
    for (let parent = node.parent; parent; parent = parent.parent) {
      if (ts.isCallExpression(parent) && ts.isIdentifier(parent.expression) &&
        (parent.expression.text === "readOwned" || parent.expression.text === "serializeOwned")) return true;
      if (ts.isStatement(parent)) break;
    }
    return false;
  };
  const violations: string[] = [];
  const visit = (node: ts.Node): void => {
    const thenReceiver = ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)
      ? node.expression.expression : null;
    const ownedReceiver = thenReceiver && ts.isCallExpression(thenReceiver) &&
      ts.isIdentifier(thenReceiver.expression) && thenReceiver.expression.text === "readOwned";
    const isThen = thenReceiver && !ownedReceiver &&
      ts.isPropertyAccessExpression((node as ts.CallExpression).expression) &&
      (node as ts.CallExpression & { expression: ts.PropertyAccessExpression }).expression.name.text === "then" &&
      backendCalls(thenReceiver).length > 0;
    if (((ts.isAwaitExpression(node) && backendCalls(node.expression).length > 0) || isThen) && !guarded(node)) {
      const line = tree.getLineAndCharacterOfPosition(node.getStart(tree)).line + 1;
      violations.push(`${file}#${functionName(node)}:${line}: backend completion bypasses owned result`);
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
  return violations;
}

function assertLateLandings(file: string, source: string): void {
  const violations = lateLandingViolations(file, source);
  if (violations.length) throw new Error(
    `I-20: await or .then on a backend call must use readOwned or a named rule exemption; exemplar src/components/Page.tsx runJournalFeedRestart.\n${violations.join("\n")}`
  );
}

describe("I-20 owned backend completion syntax", () => {
  it("keeps production backend completions behind owned results", () => {
    const violations = productionSources("src").flatMap((file) => lateLandingViolations(file, readFileSync(file, "utf8")));
    expect(Object.keys(UNMIGRATED).filter((key) => !ORIGINAL_UNMIGRATED.has(key)),
      "I-20: the function-keyed 10b/10c list may only shrink; exemplar Page.tsx runJournalFeedRestart").toEqual([]);
    expect(violations.filter((violation) => !UNMIGRATED[violation.split(":")[0]] && !EXEMPT[violation.split(":")[0]]),
      "I-20: backend completions require owned results; exemplar Page.tsx runJournalFeedRestart").toEqual([]);
    expect(Object.keys(EXEMPT).filter((key) => !violations.some((violation) => violation.startsWith(key + ":"))),
      "I-20: remove obsolete rule exemptions").toEqual([]);
    expect(Object.keys(UNMIGRATED).filter((key) => !violations.some((violation) => violation.startsWith(key + ":"))),
      "I-20: remove migrated function exemptions").toEqual([]);
  });
  it("fails a planted old graph completion", () => {
    expect(() => assertLateLandings("src/planted.ts", "async function stale() { const dto = await backend().getPage('P', 'page'); reloadPage(dto); }")).toThrow(/I-20.*exemplar src\/components\/Page\.tsx/s);
  });
  it("finds multiline then, nested await and a backend alias", () => {
    expect(lateLandingViolations("src/planted.ts", "backend().getPage('a', 'page')\n .then((dto) => setPage(dto));")).toHaveLength(1);
    expect(lateLandingViolations("src/planted.ts", "function outer() { return items.map(async () => { const dto = await backend().getPage('a', 'page'); setPage(dto); }); }")).toHaveLength(1);
    expect(lateLandingViolations("src/planted.ts", "async function alias() { const api = backend(); const dto = await api.getPage('a', 'page'); setPage(dto); }")).toHaveLength(1);
    expect(lateLandingViolations("src/planted.ts", "async function safe() { const result = await readOwned(owner, backend().getPage('a', 'page')); if (result.kind === 'stale') return; setPage(result.value); }")).toHaveLength(0);
  });
});
