import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const DIRECT_BACKEND_WRITERS = new Set(["writeText", "writeRich", "copyImageToClipboard"]);
const ALLOWED = new Set(["src/clipboard.ts", "src/backend.ts", "src/mock.ts"]);

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(file);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [file] : [];
  });
}

export function clipboardWriterViolations(file: string, source: string): string[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const violations: string[] = [];
  const report = (node: ts.Node, message: string) => {
    const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
    violations.push(`${file}:${line + 1}: ${message}`);
  };
  const visit = (node: ts.Node) => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier) &&
      node.moduleSpecifier.text.includes("plugin-clipboard")) {
      report(node, "direct clipboard plugin import");
    }
    if (ts.isCallExpression(node)) {
      const callee = node.expression;
      if (ts.isPropertyAccessExpression(callee) && DIRECT_BACKEND_WRITERS.has(callee.name.text) &&
        ts.isCallExpression(callee.expression) && ts.isIdentifier(callee.expression.expression) &&
        callee.expression.expression.text === "backend") {
        report(node, `direct backend().${callee.name.text} call`);
      }
      const calleeText = callee.getText(sf).replace(/\s/g, "");
      if (/^(?:window\.)?navigator\.clipboard\??\.(?:write|writeText)$/.test(calleeText)) {
        report(node, "direct navigator.clipboard write");
      }
      if (ts.isIdentifier(callee) && callee.text === "invoke" && ts.isStringLiteral(node.arguments[0]) &&
        /clipboard|copy_image_to_clipboard/i.test(node.arguments[0].text)) {
        report(node, "direct native clipboard invoke");
      }
      if (callee.kind === ts.SyntaxKind.ImportKeyword && ts.isStringLiteral(node.arguments[0]) &&
        node.arguments[0].text.includes("plugin-clipboard")) {
        report(node, "direct clipboard plugin import");
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return violations;
}

describe("clipboard writer facade guard", () => {
  it("has no application clipboard transport calls outside the facade/definitions/mocks", () => {
    const root = process.cwd();
    const violations = sourceFiles(path.join(root, "src")).flatMap((absolute) => {
      const relative = path.relative(root, absolute).replaceAll(path.sep, "/");
      if (ALLOWED.has(relative)) return [];
      return clipboardWriterViolations(relative, readFileSync(absolute, "utf8"));
    });
    expect(violations).toEqual([]);
  });

  it("keeps every enumerated producer routed through the shared facade", () => {
    const expected: Record<string, RegExp> = {
      "src/components/ExportModal.tsx": /writeClipboardText\(payload\(\)\)/,
      "src/render/inline.tsx": /writeClipboardText\(props\.text\)/,
      "src/copyImage.ts": /writeClipboardImage\(/,
      "src/sheet/mutations.ts": /copyRich\(text, html\)/,
      "src/components/ContextMenu.tsx": /writeClipboardText\(/,
      "src/components/Block.tsx": /writeClipboardText\(/,
      "src/components/PdfViewer.tsx": /writeClipboardText\(/,
      "src/components/ImproveTab.tsx": /writeClipboardText\(/,
    };
    for (const [file, pattern] of Object.entries(expected)) {
      expect(readFileSync(file, "utf8"), file).toMatch(pattern);
    }
  });

  it("rejects planted backend, navigator, plugin, and native violations", () => {
    const planted = [
      'backend().writeText("x");',
      'navigator.clipboard.writeText("x");',
      'import { writeText } from "@tauri-apps/plugin-clipboard-manager";',
      'invoke("copy_image_to_clipboard", {});',
    ].join("\n");
    expect(clipboardWriterViolations("src/planted.ts", planted)).toHaveLength(4);
  });
});
