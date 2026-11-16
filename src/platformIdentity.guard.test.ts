// D-20261018-W1: UI features depend on available room (viewport width) and
// input capability (pointer / hover), never on platform identity. Identity may
// decide window chrome, native APIs and file access, and memory/pixel caps.
// Every remaining read of the platform's name is listed, with its reason, in
// src/PLATFORM_IDENTITY.txt; this test fails on a new read (add the entry only
// with a reason from the closed list) and on an entry whose read is gone.
//
// What counts as a read of identity (AST, not grep):
//  - importing isMobilePlatform | platformKind | touchGesturePlatform |
//    osDrawsWindowControls | canRevealInFileManager | isMac | isMobile |
//    browserPlatform from nativeChrome / platform / browserPlatform;
//  - navigator.userAgent / navigator.platform, `.appPlatform(`,
//    `__TINE_PLATFORM__`;
//  - comparing something to "android" / "ios" / "desktop" (`platform-literal`);
//  - a `data-platform` selector in CSS.
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const SRC = join(__dirname);
const ROOT = join(__dirname, "..");

const IDENTITY_EXPORTS = new Set([
  "isMobilePlatform", "platformKind", "touchGesturePlatform", "osDrawsWindowControls",
  "canRevealInFileManager", "isMac", "isMobile", "browserPlatform",
]);
const IDENTITY_MODULES = /(^|\/)(nativeChrome|platform|browserPlatform)$/;
/** Files that DEFINE or transport identity; reading it is their job. */
const DEFINERS = new Set([
  "src/nativeChrome.ts", "src/platform.ts", "src/browserPlatform.ts", "src/backend.ts",
  "src/backendInterface.ts", "src/mock.ts", "src/publishedBackend.ts", "src/inputCapability.ts",
]);
/** ui-layout is not here on purpose: layout follows room and input. */
const REASONS = ["window-chrome", "native-api", "memory-cap", "os-convention", "host-event"] as const;

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) sourceFiles(path, out);
    else if (/\.(ts|tsx)$/.test(entry.name) && !/\.(test|dom\.test|guard\.test)\.tsx?$|\.test\.tsx?$/.test(entry.name)) out.push(path);
  }
  return out;
}

const isLiteralPlatform = (node: ts.Node) => ts.isStringLiteralLike(node) && ["android", "ios", "desktop"].includes(node.text);

/** The identity reads of one source text: symbol -> number of uses. An import
 *  of an identity export counts every reference to its local name (aliases
 *  followed) and every `ns.symbol` access of a namespace import, so a second
 *  `!isMobilePlatform` in a file that already imports it is a new use, and
 *  `import * as chrome` does not hide anything. */
export function identityReads(text: string, fileName = "x.ts"): Map<string, number> {
  const file = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true, fileName.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const found = new Map<string, number>();
  const add = (symbol: string) => found.set(symbol, (found.get(symbol) ?? 0) + 1);
  const touch = (symbol: string) => { if (!found.has(symbol)) found.set(symbol, 0); };
  const locals = new Map<string, string>(); // local name -> identity export
  const namespaces = new Set<string>(); // `import * as ns` of an identity module
  // Pass 1: imports (and re-exports) of the identity modules.
  for (const node of file.statements) {
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)
      && IDENTITY_MODULES.test(node.moduleSpecifier.text)) {
      if (ts.isExportDeclaration(node)) {
        // `export { isMobilePlatform } from ...` / `export * from ...` re-launders identity.
        if (!node.exportClause) add("export-star");
        else if (ts.isNamedExports(node.exportClause)) {
          for (const spec of node.exportClause.elements) {
            const name = (spec.propertyName ?? spec.name).text;
            if (IDENTITY_EXPORTS.has(name)) add(`re-export:${name}`);
          }
        }
        continue;
      }
      const bindings = node.importClause?.namedBindings;
      if (bindings && ts.isNamespaceImport(bindings)) namespaces.add(bindings.name.text);
      if (bindings && ts.isNamedImports(bindings)) {
        for (const spec of bindings.elements) {
          const imported = (spec.propertyName ?? spec.name).text;
          if (IDENTITY_EXPORTS.has(imported)) { locals.set(spec.name.text, imported); touch(imported); }
        }
      }
    }
  }
  const visit = (node: ts.Node) => {
    if (ts.isImportDeclaration(node) || ts.isImportEqualsDeclaration(node)) return;
    if (ts.isPropertyAccessExpression(node)) {
      if (ts.isIdentifier(node.expression)) {
        if (node.expression.text === "navigator" && ["userAgent", "platform"].includes(node.name.text)) add(`navigator.${node.name.text}`);
        if (namespaces.has(node.expression.text) && IDENTITY_EXPORTS.has(node.name.text)) add(node.name.text);
      }
      if (node.name.text === "appPlatform") add("appPlatform");
    }
    if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword
      && node.arguments[0] && ts.isStringLiteralLike(node.arguments[0]) && IDENTITY_MODULES.test(node.arguments[0].text)) add("dynamic-import");
    if (ts.isElementAccessExpression(node) && ts.isIdentifier(node.expression) && namespaces.has(node.expression.text)) add("namespace-element-access");
    if (ts.isIdentifier(node)) {
      const parent = node.parent;
      const isPropertyName = (ts.isPropertyAccessExpression(parent) && parent.name === node)
        || (ts.isPropertyAssignment(parent) && parent.name === node)
        || (ts.isBindingElement(parent) && parent.propertyName === node);
      const local = locals.get(node.text);
      if (local && !isPropertyName) add(local);
      if (node.text === "__TINE_PLATFORM__") add("__TINE_PLATFORM__");
    }
    if (ts.isBinaryExpression(node) && [
      ts.SyntaxKind.EqualsEqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsEqualsToken,
      ts.SyntaxKind.EqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsToken,
    ].includes(node.operatorToken.kind) && (isLiteralPlatform(node.left) || isLiteralPlatform(node.right))) {
      add("platform-literal");
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

/** `file symbol` -> uses, for every non-test source file under `srcDir`. */
export function scanTree(srcDir = SRC, rootDir = ROOT): Map<string, number> {
  const keys = new Map<string, number>();
  for (const path of sourceFiles(srcDir)) {
    const rel = relative(rootDir, path);
    if (DEFINERS.has(rel)) continue;
    for (const [symbol, uses] of identityReads(readFileSync(path, "utf8"), path)) keys.set(`${rel} ${symbol}`, uses);
  }
  const styles = join(srcDir, "styles");
  if (existsSync(styles)) {
    for (const css of readdirSync(styles).filter((name) => name.endsWith(".css"))) {
      if (/data-platform/.test(readFileSync(join(styles, css), "utf8"))) keys.set(`${relative(rootDir, join(styles, css))} data-platform`, 1);
    }
  }
  return keys;
}

type Entry = { key: string; uses: number; reason: string };
function parseAllowlist(text: string): Entry[] {
  return text.split("\n").filter((line) => line.trim() && !line.startsWith("#")).map((line) => {
    const match = /^(\S+) (\S+) x(\d+)\s+# why: (\S+); (.+)$/.exec(line);
    if (!match) throw new Error(`PLATFORM_IDENTITY.txt: malformed line (want "<file> <symbol> x<uses>  # why: <reason>; <detail>"): ${line}`);
    if (!(REASONS as readonly string[]).includes(match[4]!)) {
      throw new Error(`PLATFORM_IDENTITY.txt: reason "${match[4]}" is not one of ${REASONS.join(", ")} (layout follows room and input, D-20261018-W1): ${line}`);
    }
    return { key: `${match[1]} ${match[2]}`, uses: Number(match[3]), reason: match[4]! };
  });
}

function check(actual: Map<string, number>, entries: Entry[]): string[] {
  const listed = new Map(entries.map((entry) => [entry.key, entry.uses]));
  const problems: string[] = [];
  for (const [key, uses] of actual) {
    if (!listed.has(key)) problems.push(`unlisted identity read: ${key} (x${uses})`);
    else if (listed.get(key) !== uses) problems.push(`uses changed for ${key}: approved x${listed.get(key)}, found x${uses} (a new use needs a decision, not a bump)`);
  }
  for (const key of listed.keys()) if (!actual.has(key)) problems.push(`stale entry (no such read any more): ${key}`);
  if (listed.size !== entries.length) problems.push("duplicate entries");
  const order = entries.map((entry) => entry.key);
  if (order.join("\n") !== [...order].sort().join("\n")) problems.push("entries are not sorted");
  return problems;
}

/** Build a throwaway src tree and scan it with the REAL tree scanner. */
function plantedTree(files: Record<string, string>): { scan: Map<string, number>; cleanup: () => void } {
  const root = mkdtempSync(join(tmpdir(), "tine-identity-guard-"));
  const src = join(root, "src");
  mkdirSync(join(src, "styles"), { recursive: true });
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(join(src, name, ".."), { recursive: true });
    writeFileSync(join(src, name), text);
  }
  return { scan: scanTree(src, root), cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

describe("platform identity is an allowlist (D-20261018-W1)", () => {
  it("every read of platform identity in src is listed with a reason and use count, and every entry is live", () => {
    const entries = parseAllowlist(readFileSync(join(SRC, "PLATFORM_IDENTITY.txt"), "utf8"));
    expect(check(scanTree(), entries), "add the entry with a reason, or express the gate with viewportClass / inputCapability").toEqual([]);
  });

  it("detects a planted identity gate in layout code, and a stale entry", () => {
    const planted = identityReads(
      `import { isMobilePlatform } from "./nativeChrome";\nexport const showStrip = () => !isMobilePlatform;\nconst p = navigator.userAgent;\nif (kind === "ios") {}\n`,
    );
    expect([...planted].sort()).toEqual([["isMobilePlatform", 1], ["navigator.userAgent", 1], ["platform-literal", 1]]);
    expect(check(new Map([["src/a.ts isMobilePlatform", 1]]), [])).toEqual(["unlisted identity read: src/a.ts isMobilePlatform (x1)"]);
    expect(check(new Map(), [{ key: "src/a.ts isMobilePlatform", uses: 1, reason: "native-api" }])).toEqual(["stale entry (no such read any more): src/a.ts isMobilePlatform"]);
    expect(() => parseAllowlist("src/a.ts isMobilePlatform x1  # why: ui-layout; hide the strip")).toThrow(/not one of/);
    expect(() => parseAllowlist("src/a.ts isMobilePlatform  # why: native-api; x")).toThrow(/malformed/);
  });

  it("a second use of an already-approved import is a new use (planted through the real scanner)", () => {
    const approved = `import { isMobilePlatform } from "./nativeChrome";\nexport const a = () => (isMobilePlatform ? 1 : 2);\n`;
    const entries: Entry[] = [{ key: "src/App.tsx isMobilePlatform", uses: 1, reason: "native-api" }];
    const clean = plantedTree({ "App.tsx": approved });
    try { expect(check(clean.scan, entries)).toEqual([]); } finally { clean.cleanup(); }
    const planted = plantedTree({ "App.tsx": approved + `export const showStrip = () => !isMobilePlatform;\n` });
    try {
      expect(check(planted.scan, entries)).toEqual([expect.stringContaining("uses changed for src/App.tsx isMobilePlatform: approved x1, found x2")]);
    } finally { planted.cleanup(); }
  });

  it("a namespace import, an alias, a dynamic import and a re-export cannot hide identity (planted through the real scanner)", () => {
    const planted = plantedTree({
      "Ns.tsx": `import * as chrome from "./nativeChrome";\nexport const hide = () => !chrome.isMobilePlatform;\n`,
      "Alias.tsx": `import { isMobilePlatform as mobile } from "./nativeChrome";\nexport const hide = () => !mobile;\n`,
      "Dyn.ts": `export const load = () => import("./nativeChrome");\n`,
      "Re.ts": `export { isMobilePlatform } from "./nativeChrome";\n`,
      "Star.ts": `export * from "./platform";\n`,
    });
    try {
      expect(Object.fromEntries(planted.scan)).toEqual({
        "src/Ns.tsx isMobilePlatform": 1,
        "src/Alias.tsx isMobilePlatform": 1,
        "src/Dyn.ts dynamic-import": 1,
        "src/Re.ts re-export:isMobilePlatform": 1,
        "src/Star.ts export-star": 1,
      });
      expect(check(planted.scan, []).filter((p) => p.startsWith("unlisted"))).toHaveLength(5);
    } finally { planted.cleanup(); }
  });

  it("ignores room and input reads", () => {
    const quiet = identityReads(
      `import { touchPrimary } from "./inputCapability";\nimport { tabStripFits } from "./viewportClass";\nconst q = matchMedia("(pointer: coarse)");\n`,
    );
    expect([...quiet]).toEqual([]);
  });
});
