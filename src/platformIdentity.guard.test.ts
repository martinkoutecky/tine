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
import { readdirSync, readFileSync } from "node:fs";
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

/** The identity reads of one source text, as `symbol` names (deduplicated). */
export function identityReads(text: string, fileName = "x.ts"): Set<string> {
  const file = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true, fileName.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const found = new Set<string>();
  const visit = (node: ts.Node) => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier) && IDENTITY_MODULES.test(node.moduleSpecifier.text)) {
      const bindings = node.importClause?.namedBindings;
      if (bindings && ts.isNamedImports(bindings)) {
        for (const spec of bindings.elements) {
          const imported = (spec.propertyName ?? spec.name).text;
          if (IDENTITY_EXPORTS.has(imported)) found.add(imported);
        }
      }
    }
    if (ts.isPropertyAccessExpression(node)) {
      if (ts.isIdentifier(node.expression) && node.expression.text === "navigator" && ["userAgent", "platform"].includes(node.name.text)) {
        found.add(`navigator.${node.name.text}`);
      }
      if (node.name.text === "appPlatform") found.add("appPlatform");
    }
    if (ts.isIdentifier(node) && node.text === "__TINE_PLATFORM__") found.add("__TINE_PLATFORM__");
    if (ts.isBinaryExpression(node) && [
      ts.SyntaxKind.EqualsEqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsEqualsToken,
      ts.SyntaxKind.EqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsToken,
    ].includes(node.operatorToken.kind) && (isLiteralPlatform(node.left) || isLiteralPlatform(node.right))) {
      found.add("platform-literal");
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

function scanTree(): string[] {
  const keys: string[] = [];
  for (const path of sourceFiles(SRC)) {
    const rel = relative(ROOT, path);
    if (DEFINERS.has(rel)) continue;
    for (const symbol of identityReads(readFileSync(path, "utf8"), path)) keys.push(`${rel} ${symbol}`);
  }
  for (const css of readdirSync(join(SRC, "styles")).filter((name) => name.endsWith(".css"))) {
    if (/data-platform/.test(readFileSync(join(SRC, "styles", css), "utf8"))) keys.push(`src/styles/${css} data-platform`);
  }
  return keys.sort();
}

type Entry = { key: string; reason: string };
function parseAllowlist(text: string): Entry[] {
  return text.split("\n").filter((line) => line.trim() && !line.startsWith("#")).map((line) => {
    const match = /^(\S+) (\S+)\s+# why: (\S+); (.+)$/.exec(line);
    if (!match) throw new Error(`PLATFORM_IDENTITY.txt: malformed line (want "<file> <symbol>  # why: <reason>; <detail>"): ${line}`);
    if (!(REASONS as readonly string[]).includes(match[3]!)) {
      throw new Error(`PLATFORM_IDENTITY.txt: reason "${match[3]}" is not one of ${REASONS.join(", ")} (layout follows room and input, D-20261018-W1): ${line}`);
    }
    return { key: `${match[1]} ${match[2]}`, reason: match[3]! };
  });
}

function check(actual: string[], entries: Entry[]): string[] {
  const listed = entries.map((entry) => entry.key);
  const problems: string[] = [];
  for (const key of actual) if (!listed.includes(key)) problems.push(`unlisted identity read: ${key}`);
  for (const key of listed) if (!actual.includes(key)) problems.push(`stale entry (no such read any more): ${key}`);
  if (new Set(listed).size !== listed.length) problems.push("duplicate entries");
  if (listed.join("\n") !== [...listed].sort().join("\n")) problems.push("entries are not sorted");
  return problems;
}

describe("platform identity is an allowlist (D-20261018-W1)", () => {
  it("every read of platform identity in src is listed with a reason, and every entry is live", () => {
    const entries = parseAllowlist(readFileSync(join(SRC, "PLATFORM_IDENTITY.txt"), "utf8"));
    expect(check(scanTree(), entries), "add the entry with a reason, or express the gate with viewportClass / inputCapability").toEqual([]);
  });

  it("detects a planted identity gate in layout code, and a stale entry", () => {
    const planted = identityReads(
      `import { isMobilePlatform } from "./nativeChrome";\nexport const showStrip = () => !isMobilePlatform;\nconst p = navigator.userAgent;\nif (kind === "ios") {}\n`,
    );
    expect([...planted].sort()).toEqual(["isMobilePlatform", "navigator.userAgent", "platform-literal"]);
    expect(check(["src/a.ts isMobilePlatform"], [])).toEqual(["unlisted identity read: src/a.ts isMobilePlatform"]);
    expect(check([], [{ key: "src/a.ts isMobilePlatform", reason: "native-api" }])).toEqual(["stale entry (no such read any more): src/a.ts isMobilePlatform"]);
    expect(() => parseAllowlist("src/a.ts isMobilePlatform  # why: ui-layout; hide the strip")).toThrow(/not one of/);
    expect(() => parseAllowlist("src/a.ts isMobilePlatform")).toThrow(/malformed/);
  });

  it("ignores room and input reads", () => {
    const quiet = identityReads(
      `import { touchPrimary } from "./inputCapability";\nimport { tabStripFits } from "./viewportClass";\nconst q = matchMedia("(pointer: coarse)");\n`,
    );
    expect([...quiet]).toEqual([]);
  });
});
