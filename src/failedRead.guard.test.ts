import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { expect, it } from "vitest";

const RULE = "I-2/I-9: a failed read is not empty/absent; preserve the last good value and reportUiFailure, or propagate the typed error. Exemplar src/pageIndex.ts and crates/tine-graph-features/src/assets.rs orphan_assets";

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? sources(file) : /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}
function emptyValue(node: ts.Expression): boolean {
  if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isTypeAssertionExpression(node)) return emptyValue(node.expression);
  return node.kind === ts.SyntaxKind.NullKeyword || ts.isIdentifier(node) && node.text === "undefined"
    || ts.isStringLiteral(node) && node.text === ""
    || ts.isArrayLiteralExpression(node) && node.elements.length === 0
    || ts.isObjectLiteralExpression(node) && node.properties.length === 0;
}
function fabricated(source: string): number {
  let count = 0;
  const tree = ts.createSourceFile("source.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  function returns(node: ts.Node) {
    if (ts.isFunctionLike(node)) return;
    if (ts.isReturnStatement(node) && node.expression && emptyValue(node.expression)) count++;
    ts.forEachChild(node, returns);
  }
  function handler(node: ts.Node) {
    if (ts.isArrowFunction(node) || ts.isFunctionExpression(node)) {
      if (ts.isBlock(node.body)) ts.forEachChild(node.body, returns);
      else if (emptyValue(node.body)) count++;
    }
  }
  function visit(node: ts.Node) {
    if (ts.isCatchClause(node)) ts.forEachChild(node.block, returns);
    if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === "catch") {
      node.arguments.forEach(handler);
    }
    ts.forEachChild(node, visit);
  }
  visit(tree);
  return count;
}

// Remaining pre-existing failures, frozen after OG-B-FAIL. Counts are per file
// rather than line numbers so formatting cannot add exceptions. They may fall.
const TS_BASELINE: Record<string, number> = {
  "src/assetCache.ts": 1,
  "src/backend.ts": 1,
  "src/components/ExportModal.tsx": 1,
  "src/components/LinkedReferences.tsx": 2,
  "src/components/Macro.tsx": 2,
  "src/components/UnlinkedReferences.tsx": 1,
  "src/editor/htmlPaste.ts": 1,
  "src/favorites.ts": 1,
  "src/gpu.ts": 1,
  "src/graph.ts": 1,
  "src/mediaEditorSettings.ts": 1,
  "src/publishedPermalink.ts": 1,
  "src/render/hiccup.ts": 1,
  "src/render/inline.tsx": 1,
  "src/session.ts": 1,
  "src/sheet/exportSheets.ts": 1,
  "src/themeGallery.ts": 1,
  "src/ui.ts": 5,
  "src/workspaces.ts": 1
};

it("ratchets catches that fabricate empty answers in production TS", () => {
  for (const file of sources("src")) {
    expect(fabricated(readFileSync(file, "utf8")), `${RULE}: ${file}`).toBeLessThanOrEqual(TS_BASELINE[file] ?? 0);
  }
});
it("detects multiline, cast and expression-body fabricated catch results", () => {
  expect(fabricated(`try { work(); } catch (e) { return []; }
    try { work(); } catch { if (x) return null; return undefined; }
    work().catch(() => ({} as Counts));
    work().catch(e => { report(e); return ""; });`)).toBe(5);
  expect(fabricated(`try { work(); } catch (e) { reportUiFailure("page-inventory", e); return; }`)).toBe(0);
});

function rustFabricated(source: string): number {
  const production = source.replace(/#\[cfg\(test\)\]\s*mod\s+\w+\s*\{[\s\S]*?^\}/gm, "");
  return [...production.matchAll(/\b(?:read|read_to_string|read_parse_input|read_bounded|read_line|open|metadata|scan_area)\s*\([^;]*?(?:\.ok\(\)|\.unwrap_or_default\(\))/g)].length;
}
const RUST_BASELINE: Record<string, number> = {
  "crates/tine-graph-features/src/lib.rs": 0,
  "crates/tine-graph-features/src/assets.rs": 0,
  "crates/tine-store/src/model.rs": 7,
  "crates/tine-store/src/model/page_identity.rs": 5,
  "src-tauri/src/commands.rs": 0,
  "src-tauri/src/backup.rs": 4,
  "src-tauri/src/plugins.rs": 1,
  "src-tauri/src/settings.rs": 8,
};
it("ratchets IO reads converted to defaults in touched Rust modules", () => {
  for (const [file, baseline] of Object.entries(RUST_BASELINE)) {
    expect(rustFabricated(readFileSync(file, "utf8")), `${RULE}: ${file}`).toBeLessThanOrEqual(baseline);
  }
  expect(rustFabricated("let config = read_parse_input(path).map(Config::parse).unwrap_or_default();")).toBe(1);
  expect(rustFabricated("let bytes = fs::read(path).ok()?; let line = reader.read_line(&mut text).ok()?;")).toBe(2);
});

it("rollback restore failures keep recovery evidence instead of discarding move-back errors", () => {
  const source = readFileSync("crates/tine-store/src/model.rs", "utf8");
  expect(source, RULE).not.toMatch(/let _ = move_file_noreplace\(&staged,\s*path\)/);
});
